"""City growth: housing, the food needed for the next citizen, and how surplus food accumulates."""
import math

from .rules import Rules


def growth_threshold(rules: Rules, pop, speed):
    """Food needed to grow from `pop` to `pop + 1`."""
    n = pop - 1
    base = (rules.param("CITY_GROWTH_THRESHOLD") + rules.param("CITY_GROWTH_MULTIPLIER") * n
            + n ** rules.param("CITY_GROWTH_EXPONENT"))
    return math.floor(base * rules.game_speed_cost_multiplier(speed))  # rounded down like costs (7.5 -> 7 online)


def has_fresh_water(city):
    """River through the center, or an adjacent lake or oasis."""
    from .yields import neighbour_xy
    cx, cy = city["center"]["x"], city["center"]["y"]
    plots = {(p["x"], p["y"]): p for p in list(city.get("near", ())) + list(city["owned"])}
    center = plots.get((cx, cy))
    adj = [plots[xy] for xy in neighbour_xy(cx, cy) if xy in plots]
    return bool(center and center["river"]) or any(p.get("lake") or p["feature"] == "FEATURE_OASIS" for p in adj)


def is_coastal(city):
    from .yields import neighbour_xy
    plots = {(p["x"], p["y"]): p for p in list(city.get("near", ())) + list(city["owned"])}
    return any(plots[xy]["terrain"] == "TERRAIN_COAST" and not plots[xy].get("lake")
               for xy in neighbour_xy(city["center"]["x"], city["center"]["y"]) if xy in plots)


def housing(rules: Rules, city, player=None, cities=None):
    """Water + buildings + districts + improvements (each improvement gives Housing/TilesRequired) + modifiers
    from traits, policies, government, pantheon, buildings and districts (when `player` is given)."""
    fresh = has_fresh_water(city)
    e = None
    if player is not None:
        from .modifiers import engine_for
        e = engine_for(rules, player, cities or [city])
        if any(a.get("HasBonus") in ("1", "true") for a, m, s in e.city_effects("EFFECT_ADJUST_CITIES_FRESHWATER_HOUSING_BONUS", city["name"])
               + e.player_effects("EFFECT_ADJUST_CITIES_FRESHWATER_HOUSING_BONUS")):
            fresh = True  # Mohenjo-daro: every city housed as if by a river
    no_water = e is not None and any(a.get("NoHousing") in ("1", "true") for a, m, s in
                                     e.city_effects("EFFECT_ADJUST_NO_FRESH_WATER_HOUSING", city["name"]))
    if no_water:
        h = rules.param("CITY_POPULATION_NO_WATER")  # Maya: no extra housing from fresh water or coast
    elif fresh:
        h = rules.param("CITY_POPULATION_RIVER_LAKE")
    elif is_coastal(city):
        h = rules.param("CITY_POPULATION_COAST")
    else:
        h = rules.param("CITY_POPULATION_NO_WATER")
    done = {d["type"] for d in city["districts"] if d["complete"]}
    if "DISTRICT_AQUEDUCT" in done or "DISTRICT_BATH" in done:
        # an aqueduct brings a city without fresh water up to 6, or adds 2 with it
        h = h + 2 if fresh else max(h, 6)
    for d in city["districts"]:
        if d["complete"]:
            row = rules.q("select Housing from Districts where DistrictType=?", d["type"])
            h += (row[0][0] or 0) if row else 0
    for b in city["buildings"]:
        h += rules.q("select Housing from Buildings where BuildingType=?", b)[0][0]
    for p in city["owned"]:
        if p["improvement"] and not p.get("pillaged"):
            row = rules.q("select Housing, TilesRequired from Improvements where ImprovementType=?", p["improvement"])
            if row and row[0][0]:
                h += row[0][0] / max(row[0][1], 1)
    if e is not None:
        n = city["name"]
        for effect in ("EFFECT_ADJUST_POLICY_HOUSING", "EFFECT_ADJUST_BUILDING_HOUSING", "EFFECT_ADJUST_CITY_HOUSING_FROM_GREAT_PEOPLE"):
            h += sum(float(a.get("Amount") or 0) for a, m, s in e.city_effects(effect, n))
        for effect in ("EFFECT_ADJUST_DISTRICT_HOUSING", "EFFECT_ADJUST_IMPROVEMENT_HOUSING"):
            h += sum(float(a.get("Amount") or 0) for d, a, m, s in e.district_effects(effect, n))
            h += sum(float(a.get("Amount") or 0) for p, a, m, s in e.owned_plot_effects(effect, n))
        if not fresh:
            h += sum(float(a.get("Amount") or 0) for a, m, s in e.city_effects("EFFECT_ADJUST_WATER_HOUSING", n))
    return h


def housing_growth_multiplier(rules: Rules, housing_left):
    if housing_left <= rules.param("CITY_HOUSING_LEFT_ZERO_GROWTH"):
        return 0.0
    if housing_left <= rules.param("CITY_HOUSING_LEFT_25PCT_GROWTH"):
        return 0.25
    if housing_left <= rules.param("CITY_HOUSING_LEFT_50PCT_GROWTH"):
        return 0.5
    return 1.0


def growth_percent(rules: Rules, player, city, cities=None):
    """% faster growth from modifiers (e.g. Hanging Gardens), added to the amenity growth %."""
    if player is None:
        return 0.0
    from .modifiers import engine_for
    e = engine_for(rules, player, cities or [city])
    return sum(float(a.get("Amount") or 0) for a, m, s in e.city_effects("EFFECT_ADJUST_CITY_GROWTH", city["name"]))


def food_added(rules: Rules, city, food, growth_pct=0.0, player=None, cities=None):
    """Food stored this turn: surplus, sped up by growth modifiers and slowed by lack of housing."""
    surplus = food - rules.param("CITY_FOOD_CONSUMPTION_PER_POPULATION") * city["population"]
    if surplus <= 0:
        return surplus
    left = housing(rules, city, player, cities) - city["population"]
    growth_pct += growth_percent(rules, player, city, cities)
    return surplus * (1 + growth_pct / 100) * housing_growth_multiplier(rules, left)
