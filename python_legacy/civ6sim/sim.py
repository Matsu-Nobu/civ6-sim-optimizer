"""One-turn economy step for a single player.

The simulator carries the accumulators (production per item, pending overflow, food store, population,
tech/civic progress, gold). The "environment" of each city for the turn (worked and owned tiles,
buildings, districts, policies, governors, ...) is given from outside: from a save during replay, or
from a planner later.
"""
import math
from dataclasses import dataclass, field

from .amenities import amenity_scores, happiness
from .growth import food_added, growth_threshold
from .modifiers import engine_for
from .production import LOCK_DISTRICT_COST, item_cost, item_kind, item_production_flat, item_production_percent
from .rules import Rules
from .yields import city_yields, player_yields

YIELD_PCT_AFFECTED = ("YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH")


@dataclass
class CityAcc:
    population: int
    food_store: float = 0.0
    progress: dict = field(default_factory=dict)  # item -> production put into it
    overflow: float = 0.0  # from an item completed last turn; added next turn
    locked_cost: dict = field(default_factory=dict)  # district -> cost fixed when it was placed


@dataclass
class PlayerAcc:
    gold: float
    tech_progress: dict
    civic_progress: dict
    techs: set
    civics: set
    units_trained: dict
    faith: float = 0.0
    tech_overflow: float = 0.0
    civic_overflow: float = 0.0
    cities: dict = field(default_factory=dict)  # name -> CityAcc
    # completed specialty districts as of the last tech/civic finished: the district discount reads this snapshot
    districts_snapshot: int = None


def research_cost(rules: Rules, item, speed, game_era="ERA_ANCIENT"):
    """Technologies/Civics cost: base * speed, +30% (techs) / +20% (civics) for eras after the game era,
    -20% before it, rounded down."""
    is_tech = item.startswith("TECH_")
    table, col = ("Technologies", "TechnologyType") if is_tech else ("Civics", "CivicType")
    base, era = rules.q(f"select Cost, EraType from {table} where {col}=?", item)[0]
    order = lambda e: rules.q("select ChronologyIndex from Eras where EraType=?", e)[0][0]
    pct = 0
    if order(era) > order(game_era):
        pct = rules.param("TECH_COST_PERCENT_CHANGE_AFTER_GAME_ERA" if is_tech else "CIVIC_COST_PERCENT_CHANGE_AFTER_GAME_ERA")
    elif order(era) < order(game_era):
        pct = rules.param("TECH_COST_PERCENT_CHANGE_BEFORE_GAME_ERA" if is_tech else "CIVIC_COST_PERCENT_CHANGE_BEFORE_GAME_ERA")
    # floor(floor(base) x speed x era adjustment): BBG has fractional base costs (409.5 -> 409 x 0.5 x 1.3 = 265.85
    # -> 265) and civics round only at the end (175 x 0.5 x 1.2 = 105), both as the game reports.
    return math.floor(math.floor(base) * rules.game_speed_cost_multiplier(speed) * (1 + pct / 100) + 1e-9)


def boost_amount(cost, extra_pct=0.0):
    """Eureka / inspiration size: the boost is 40% of the current cost (Boosts.Boost; community: of the current,
    era-adjusted cost), plus EFFECT_ADJUST_TECHNOLOGY_BOOST / _CIVIC_BOOST percents. The progress jumps the probe
    measured on untouched items are floor(0.4 * (cost - 2.5)) in 103 of 130 cases (one less than 40%; reason not
    known yet), so that form is kept."""
    return math.floor((0.4 + extra_pct / 100) * (cost - 2.5))


def boost_percents(rules, player, cities):
    """Extra boost % for techs and civics from modifiers (China, golden-age dedications, ...)."""
    from .modifiers import engine_for
    e = engine_for(rules, player, cities)
    tech = sum(float(a.get("Amount") or 0) for a, m, s in e.player_effects("EFFECT_ADJUST_TECHNOLOGY_BOOST"))
    civic = sum(float(a.get("Amount") or 0) for a, m, s in e.player_effects("EFFECT_ADJUST_CIVIC_BOOST"))
    return tech, civic


def drop_weakest_citizen(rules: Rules, city, player):
    """The city after losing one citizen: the worked tile with the least food (then production) is freed."""
    from .yields import tile_yields
    cx, cy = city["center"]["x"], city["center"]["y"]
    tiles = [w for w in city["worked"] if (w["x"], w["y"]) != (cx, cy)]
    if not tiles:
        return city
    def value(w):
        y = tile_yields(rules, w, set(player["techs"]), set(player["civics"]))
        return (y["YIELD_FOOD"], y["YIELD_PRODUCTION"])
    weakest = min(tiles, key=value)
    return dict(city, worked=[w for w in city["worked"] if w is not weakest])


def city_turn_yields(rules, env_city, env_player, pop, routes, env_cities, amenity_pct, amenity_score=None):
    """Yields of a city for this turn with the simulator's population, after % modifiers: the amenity level on
    non-food yields, plus modifier percents (policies, governors, happiness-level bonuses) on their yield."""
    from .modifiers import engine_for
    from .yields import city_yield_percents, faith_conversions
    c = dict(env_city, population=pop)
    y, _ = city_yields(rules, c, env_player, routes=routes, cities_by_name=env_cities)
    engine = engine_for(rules, env_player, list(env_cities.values()) or [env_city])
    extra = city_yield_percents(rules, engine, c, amenity_score)
    out = {k: v * (1 + ((amenity_pct if k in YIELD_PCT_AFFECTED else 0) + extra.get(k, 0)) / 100) for k, v in y.items()}
    for k, v in faith_conversions(engine, c, out).items():
        out[k] = out.get(k, 0) + v
    return out


def completion_yields(rules, engine, city, item, cost):
    """One-time yields when `item` completes: EFFECT_GRANT_CITY_YIELD_PERCENT_BUILDING_CREATED_COST (buildings,
    wonders where IncludeWonder) and _UNIT_CREATED_COST, as a % of the production cost."""
    out = {}
    kind = item_kind(item)
    if kind == "BUILDING":
        wonder = rules.is_wonder(item)
        for a, m, s in engine.city_effects("EFFECT_GRANT_CITY_YIELD_PERCENT_BUILDING_CREATED_COST", city["name"]):
            if wonder and a.get("IncludeWonder") not in ("1", "true"):
                continue
            out[a["YieldType"]] = out.get(a["YieldType"], 0) + cost * float(a["BuildingProductionPercent"]) / 100
    elif kind == "UNIT":
        for a, m, s in engine.city_effects("EFFECT_GRANT_CITY_YIELD_PERCENT_UNIT_CREATED_COST", city["name"]):
            out[a["YieldType"]] = out.get(a["YieldType"], 0) + cost * float(a["UnitProductionPercent"]) / 100
    return out


def step(rules: Rules, acc: PlayerAcc, env, speed, research=None, civic=None, boosts=(), routes=(), log=None,
         after_settler=None):
    """Advance one turn. `env` is the export-format state of this turn (cities, player).
    after_settler(city_name) -> the city's environment once a settler has left (tiles worked by one
    fewer citizen); defaults to the same tiles."""
    log = log if log is not None else []
    env_cities = {c["name"]: c for c in env["cities"]}
    for c in env["cities"]:  # newly founded cities start at population 1 with an empty store
        acc.cities.setdefault(c["name"], CityAcc(population=c["population"]))
    sim_cities = [dict(c, population=acc.cities[c["name"]].population) for c in env["cities"]]
    placed, completed = {}, 0
    for c in env["cities"]:
        for d in c["districts"]:
            if d["type"] == "DISTRICT_CITY_CENTER":
                continue
            placed[d["type"]] = placed.get(d["type"], 0) + 1
            completed += 1 if d["complete"] and rules.district_requires_population(d["type"]) else 0
    scores = amenity_scores(rules, sim_cities, env["player"])
    totals = {"YIELD_GOLD": 0.0, "YIELD_SCIENCE": 0.0, "YIELD_CULTURE": 0.0, "YIELD_FAITH": 0.0}
    for c in env["cities"]:
        ca = acc.cities[c["name"]]
        nonfood_pct, growth_pct = happiness(rules, scores[c["name"]])
        y = city_turn_yields(rules, c, env["player"], ca.population, routes, env_cities, nonfood_pct, scores[c["name"]])
        for k in totals:
            totals[k] += y[k]
        # Production.
        item = c["production"]["current"]
        settled = False
        food_city = c  # the city as the growth step sees it (a building finished this turn already counts)
        if not item:  # nothing queued (e.g. the founding turn): production is held for the next item
            ca.overflow += y["YIELD_PRODUCTION"]
        if item:
            engine = engine_for(rules, env["player"], env["cities"])
            pct = item_production_percent(rules, engine, c, item)
            flat = item_production_flat(rules, engine, c, item)
            into = (y["YIELD_PRODUCTION"] + flat) * (1 + pct / 100)
            ca.progress[item] = ca.progress.get(item, 0.0) + into + ca.overflow
            ca.overflow = 0.0
            if item_kind(item) == "PROJECT":
                # while a project runs, part of the production rate is also paid out as another yield (district
                # projects: culture / faith / science / gold; Yongle's Lijia: food, gold, faith)
                for k, pc in rules.q("select YieldType, PercentOfProductionRate from Project_YieldConversions where ProjectType=?", item):
                    if k == "YIELD_FOOD":
                        y = dict(y, YIELD_FOOD=y["YIELD_FOOD"] + into * pc / 100)
                    elif k in totals:
                        totals[k] += into * pc / 100
            player_for_cost = dict(env["player"], unitsTrained=acc.units_trained, techs=sorted(acc.techs),
                                   civics=sorted(acc.civics),
                                   _districts_completed=completed if acc.districts_snapshot is None else acc.districts_snapshot)
            if item_kind(item) == "DISTRICT" and LOCK_DISTRICT_COST:
                # The cost is fixed when the district is placed (here: the first turn it is being built). The district
                # being placed is already on the map, so it counts in C.
                if item not in ca.locked_cost:
                    ca.locked_cost[item] = item_cost(rules, player_for_cost, item, speed, placed_counts=placed)
                cost = ca.locked_cost[item]
            else:
                cost = item_cost(rules, player_for_cost, item, speed, placed_counts=placed)
            if cost is not None and ca.progress[item] >= cost:
                ca.overflow = ca.progress[item] - cost
                ca.progress[item] = 0.0
                ca.locked_cost.pop(item, None)
                log.append(("completed", c["name"], item))
                # yields worth a % of the item's cost on completion (Ramses, Ayutthaya, Ambiorix, Basilikoi Paides)
                for k, v in completion_yields(rules, engine, c, item, cost).items():
                    if k in totals:
                        totals[k] += v
                if item_kind(item) == "BUILDING":
                    food_city = dict(c, buildings=list(c["buildings"]) + [item])
                if item_kind(item) == "UNIT":
                    acc.units_trained[item] = acc.units_trained.get(item, 0) + 1
                    pop_cost = rules.q("select PopulationCost from Units where UnitType=?", item)[0][0] or 0
                    if any(a.get("Enabled") in ("0", "false") for a, m, s_ in
                           engine.city_effects("EFFECT_ADJUST_CITY_SETTLER_CONSUME_POP", c["name"])) and item_kind(item) == "UNIT":
                        pop_cost = 0  # Magnus' Industrialist: settlers don't take a citizen here
                    if pop_cost:
                        ca.population -= pop_cost
                        settled = True
        # Food: after a settler leaves, the surplus uses the smaller population.
        food = y["YIELD_FOOD"]
        if food_city is not c:
            # production is resolved before growth in the game's turn: a granary finished this turn feeds this turn
            food = city_turn_yields(rules, food_city, env["player"], ca.population, routes, env_cities, nonfood_pct,
                                    scores[c["name"]])["YIELD_FOOD"]
        if settled:
            given = after_settler(c["name"]) if after_settler else None
            fewer = given if given and given["population"] == ca.population else drop_weakest_citizen(rules, c, env["player"])
            food = city_turn_yields(rules, fewer, env["player"], ca.population, routes, env_cities, 0)["YIELD_FOOD"]
        ca.food_store += food_added(rules, dict(food_city, population=ca.population), food, growth_pct, env["player"], env["cities"])
        need = growth_threshold(rules, ca.population, speed)
        if ca.food_store >= need:
            ca.food_store -= need
            ca.population += 1
            log.append(("grew", c["name"], ca.population))
    for k, v in player_yields(rules, engine_for(rules, env["player"], env["cities"]), totals).items():
        if k in totals:
            totals[k] += v
    if "units" in env:  # upkeep of buildings, districts and units
        from .production import maintenance
        units = env["units"]
        if isinstance(units, list):
            units = [u["type"] if isinstance(u, dict) else u for u in units]
        totals["YIELD_GOLD"] -= maintenance(rules, engine_for(rules, env["player"], env["cities"]), env["cities"], units)
    acc.gold += totals["YIELD_GOLD"]
    acc.faith += totals["YIELD_FAITH"]
    extra_boost = boost_percents(rules, env["player"], env["cities"])
    boosted_now = {}
    game_era = env["player"].get("gameEra") or "ERA_ANCIENT"
    for kind, amount, cur, prog, done, attr in (
            ("tech", totals["YIELD_SCIENCE"], research, acc.tech_progress, acc.techs, "tech_overflow"),
            ("civic", totals["YIELD_CULTURE"], civic, acc.civic_progress, acc.civics, "civic_overflow")):
        for b in boosts:
            # Only items with a row in Boosts get progress (the save also flags e.g. Code of Laws / Animal Husbandry).
            if (b.startswith("TECH_") == (kind == "tech")) and b not in done and rules.has_boost(b):
                amt = boost_amount(research_cost(rules, b, speed, game_era), extra_boost[0] if kind == "tech" else extra_boost[1])
                prog[b] = prog.get(b, 0.0) + amt
                boosted_now[b] = amt
        if cur and cur not in done:
            prog[cur] = prog.get(cur, 0.0) + amount + getattr(acc, attr)
            setattr(acc, attr, 0.0)
        for t in [t for t, v in prog.items() if t not in done and v >= research_cost(rules, t, speed, game_era)]:
            # what a boost puts past the cost is lost: only the turn's own research overflows (Portugal T14 Astrology)
            over = prog.pop(t) - research_cost(rules, t, speed, game_era)
            setattr(acc, attr, getattr(acc, attr) + max(0.0, over - boosted_now.get(t, 0.0)))
            done.add(t)
            log.append(("researched", t))
            acc.districts_snapshot = completed  # the discount's counts refresh when a tech or civic is finished
    if acc.districts_snapshot is None:
        acc.districts_snapshot = completed
    return log
