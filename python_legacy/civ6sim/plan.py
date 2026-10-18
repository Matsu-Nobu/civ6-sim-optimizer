"""Production planner: a turn loop around `sim.step` plus a rollout search over production choices.

Fixed from the recorded game ("scenario"): research/civic order, policies, government, pantheon,
governors, envoys, boosts (by turn). Decided by the planner: what each city builds and where settlers
go. Automated: citizens, builders, border growth, district placement.
"""
import copy
import math
from dataclasses import dataclass, field

from .amenities import amenity_scores, happiness
from .production import item_cost, item_kind
from .sim import PlayerAcc, city_turn_yields, research_cost, step
from .world import DEFAULT_WEIGHTS, World
from .yields import district_adjacency

CANDIDATE_ITEMS = [
    "UNIT_SETTLER", "UNIT_BUILDER", "BUILDING_MONUMENT", "BUILDING_GRANARY", "BUILDING_WATER_MILL",
    "DISTRICT_CAMPUS", "DISTRICT_COMMERCIAL_HUB", "DISTRICT_GOVERNMENT", "BUILDING_LIBRARY", "BUILDING_MARKET",
]
CITY_NAMES = ["CITY_2", "CITY_3", "CITY_4", "CITY_5", "CITY_6", "CITY_7", "CITY_8"]


@dataclass
class Settings:
    horizon: int = 31  # last turn simulated (inclusive)
    max_cities: int = 6
    settle_search_radius: int = 9
    settle_extra_turns: int = 0  # added to distance/2 for the walk + settling
    score_weights: dict = field(default_factory=lambda: {"YIELD_PRODUCTION": 1.0, "YIELD_SCIENCE": 1.0,
                                                          "YIELD_CULTURE": 1.0, "YIELD_GOLD": 0.5, "YIELD_FOOD": 0.0})
    terminal_turns: int = 10  # the end state's per-turn output is counted this many more times
    sites: list = None  # [(name, (x, y))] settle here in this order instead of choosing sites


@dataclass
class PlanState:
    turn: int
    acc: PlayerAcc
    cities: list
    builders: list  # remaining charges per builder
    settlers: list  # [(arrival_turn, from_city)]
    border: dict  # city -> [points, tiles acquired]
    free_building_given: set
    gov_plaza_built: bool = False
    score: float = 0.0
    decisions: list = field(default_factory=list)  # (turn, city, item)


class Scenario:
    """The parts of the recorded game the planner does not decide, looked up by turn."""

    def __init__(self, rows, annotations):
        self.by_turn = {d["turn"]: d for d in rows}
        self.rows = rows
        self.ann = annotations
        last = rows[-1]["player"]
        # Research order = order of completion in the recorded game, then whatever was in progress.
        self.tech_order, self.civic_order = [], []
        for d in rows:
            for t in d["player"]["techs"]:
                if t not in self.tech_order:
                    self.tech_order.append(t)
            for t in d["player"]["civics"]:
                if t not in self.civic_order:
                    self.civic_order.append(t)
        self.tech_order += [t for t in last["techProgress"] if t not in self.tech_order]
        self.civic_order += [t for t in last["civicProgress"] if t not in self.civic_order]

    def player_env(self, turn, acc):
        d = self.by_turn.get(turn) or self.rows[-1]
        p = dict(d["player"])
        p["techs"], p["civics"] = sorted(acc.techs), sorted(acc.civics)
        p["unitsTrained"] = acc.units_trained
        return p

    def boosts(self, turn):
        a, b = self.by_turn.get(turn), self.by_turn.get(turn + 1)
        if not a or not b:
            return []
        old = set(a["player"]["techsBoosted"] + a["player"]["civicsInspired"])
        return [t for t in b["player"]["techsBoosted"] + b["player"]["civicsInspired"] if t not in old]


def next_in(order, done, prog):
    for t in order:
        if t not in done:
            return t
    return None


class Planner:
    def __init__(self, rules, world: World, scenario: Scenario, settings: Settings, speed="GAMESPEED_ONLINE"):
        self.rules, self.world, self.sc, self.settings, self.speed = rules, world, scenario, settings, speed

    # --- start state ---------------------------------------------------------------------------
    def initial_state(self, turn):
        d = self.sc.by_turn[turn]
        p = d["player"]
        acc = PlayerAcc(gold=p["gold"], tech_progress=dict(p["techProgress"]), civic_progress=dict(p["civicProgress"]),
                        techs=set(p["techs"]), civics=set(p["civics"]), units_trained=dict(p["unitsTrained"]))
        cities = copy.deepcopy(d["cities"])
        for c in cities:
            c["production"]["current"] = None  # the planner chooses
        acc.cities = {}
        from .sim import CityAcc
        for c in cities:
            acc.cities[c["name"]] = CityAcc(population=c["population"], food_store=c["foodStore"] or 0.0,
                                            progress=dict(c["production"]["progress"]))
        builders = [3 for u in d["units"] if u["type"] == "UNIT_BUILDER"]
        return PlanState(turn=turn, acc=acc, cities=cities, builders=builders, settlers=[],
                         border={c["name"]: [0.0, len(c["owned"]) - 7] for c in cities}, free_building_given=set(),
                         gov_plaza_built=any(x["type"] == "DISTRICT_GOVERNMENT" for c in cities for x in c["districts"]))

    # --- options -------------------------------------------------------------------------------
    def available_items(self, st: PlanState, city, player, extra=()):
        r = self.rules
        techs, civics = st.acc.techs, st.acc.civics
        out = []
        districts = {x["type"] for x in city["districts"] if x["complete"]}
        planned = {x["type"] for x in city["districts"]}
        specialty = sum(1 for x in city["districts"] if r.district_requires_population(x["type"]))
        allowed_specialty = 1 + (st.acc.cities[city["name"]].population - 1) // int(r.param("DISTRICT_POPULATION_REQUIRED_PER"))
        for item in list(CANDIDATE_ITEMS) + [x for x in extra if x not in CANDIDATE_ITEMS]:
            kind = item_kind(item)
            if kind == "UNIT":
                pt, pc = r.q("select PrereqTech, PrereqCivic from Units where UnitType=?", item)[0]
                if item == "UNIT_SETTLER" and (st.acc.cities[city["name"]].population < 2 or
                                               len(st.cities) + len(st.settlers) >= self.settings.max_cities):
                    continue
            elif kind == "BUILDING":
                pt, pc, pd = r.q("select PrereqTech, PrereqCivic, PrereqDistrict from Buildings where BuildingType=?", item)[0]
                if item in city["buildings"] or (pd and pd not in districts):
                    continue
            else:
                pt, pc = r.q("select PrereqTech, PrereqCivic from Districts where DistrictType=?", item)[0]
                if item in planned or specialty >= allowed_specialty:
                    continue
                if item == "DISTRICT_GOVERNMENT" and st.gov_plaza_built:
                    continue
                if self.place_district(st, city, item, dry=True) is None:
                    continue
            if (pt and pt not in techs) or (pc and pc not in civics):
                continue
            out.append(item)
        return out

    def base_policy(self, st, city, options, player):
        """Default choice: builder if none, settlers while expanding, then cheap buildings, then districts."""
        order = ["UNIT_BUILDER" if not st.builders else None, "UNIT_SETTLER", "BUILDING_MONUMENT", "DISTRICT_GOVERNMENT",
                 "DISTRICT_COMMERCIAL_HUB", "DISTRICT_CAMPUS", "BUILDING_GRANARY", "BUILDING_LIBRARY", "BUILDING_MARKET",
                 "BUILDING_WATER_MILL", "UNIT_BUILDER"]
        for item in order:
            if item in options:
                return item
        return options[0] if options else None

    # --- districts -----------------------------------------------------------------------------
    def neighbours_with_districts(self, st, xy):
        at = {(x["x"], x["y"]): x["type"] for c in st.cities for x in c["districts"]}
        out = []
        for n in self.world.map.neighbours(*xy):
            p = dict(self.world.map.plots[n])
            p["districtType"] = at.get(n)
            out.append(p)
        return out

    def place_district(self, st, city, district, dry=False):
        center = (city["center"]["x"], city["center"]["y"])
        taken = {(x["x"], x["y"]) for c in st.cities for x in c["districts"]}
        best = None
        for p in city["owned"]:
            xy = (p["x"], p["y"])
            if xy == center or xy in taken or not self.world.map.is_land(xy) or p["resource"]:
                continue
            d = {"type": district, "x": xy[0], "y": xy[1], "complete": True, "plot": p,
                 "neighbours": self.neighbours_with_districts(st, xy)}
            adj = sum(district_adjacency(self.rules, d, st.acc.techs, st.acc.civics).values())
            if best is None or adj > best[0]:
                best = (adj, xy, d)
        if best is None:
            return None
        if not dry:
            d = best[2]
            d["complete"] = False
            city["districts"].append(d)
        return best[1]

    # --- one turn ----------------------------------------------------------------------------
    def advance(self, st: PlanState, choose):
        """Simulate turn st.turn -> st.turn + 1. `choose(st, city, options)` picks production when needed."""
        r, w, T = self.rules, self.world, st.turn
        player = self.sc.player_env(T, st.acc)
        # Settlers arriving found cities.
        for s in [s for s in st.settlers if s[0] <= T]:
            st.settlers.remove(s)
            origin = next(c for c in st.cities if c["name"] == s[1])
            o = (origin["center"]["x"], origin["center"]["y"])
            n_new = len(st.cities) - 1
            if self.settings.sites and n_new < len(self.settings.sites):
                name, best = self.settings.sites[n_new][0], tuple(self.settings.sites[n_new][1])
            else:
                cands = [xy for xy in w.map.within(o, self.settings.settle_search_radius)]
                best = max(cands, key=lambda xy: w.site_score(xy, st.cities, player) - 0.5 * w.map.distance(o, xy))
                if w.site_score(best, st.cities, player) <= 0:
                    continue
                name = CITY_NAMES[n_new] if n_new < len(CITY_NAMES) else f"CITY_{len(st.cities) + 1}"
            c = w.found_city(name, best, st.cities, player)
            st.cities.append(c)
            from .sim import CityAcc
            st.acc.cities[name] = CityAcc(population=1)
            st.border[name] = [0.0, 0]
        # Rome (BBG): cheapest building free in every city once Early Empire is known. The trait is named
        # NON_CAPITAL, but the recorded Rome got its Monument for free at T24 as well.
        if "CIVIC_EARLY_EMPIRE" in st.acc.civics and player["civilization"] == "CIVILIZATION_ROME":
            for c in st.cities:
                if c["name"] not in st.free_building_given:
                    for b in ("BUILDING_MONUMENT", "BUILDING_GRANARY"):
                        if b not in c["buildings"]:
                            c["buildings"].append(b)
                            break
                    st.free_building_given.add(c["name"])
        # Builders: one improvement per builder per turn.
        for i, charges in enumerate(list(st.builders)):
            best = w.best_improvement(st.cities, player)
            if not best:
                break
            city, plot, imp, _ = best
            plot["improvement"] = imp
            st.builders[i] -= 1
        st.builders = [b for b in st.builders if b > 0]
        # Citizens and production choices.
        for c in st.cities:
            c["population"] = st.acc.cities[c["name"]].population
            w.assign_citizens(c, player)
            if c["production"]["current"] is None:
                options = self.available_items(st, c, player, extra=getattr(self, "extra_items", ()))
                item = choose(st, c, options) if options else None
                if item and item_kind(item) == "DISTRICT":
                    self.place_district(st, c, item)
                c["production"]["current"] = item
                st.decisions.append((T, c["name"], item))
        # Score this turn's output before stepping (same yields the step uses).
        scores = amenity_scores(r, [dict(c, population=st.acc.cities[c["name"]].population) for c in st.cities])
        env_cities = {c["name"]: c for c in st.cities}
        out = {}
        for c in st.cities:
            y = city_turn_yields(r, c, player, st.acc.cities[c["name"]].population, (), env_cities,
                                 happiness(r, scores[c["name"]])[0])
            for k, v in y.items():
                out[k] = out.get(k, 0) + v
            st.border[c["name"]][0] += y["YIELD_CULTURE"]
        st.score += sum(self.settings.score_weights.get(k, 0) * v for k, v in out.items())
        research = next_in(self.sc.tech_order, st.acc.techs, st.acc.tech_progress)
        civic = next_in(self.sc.civic_order, st.acc.civics, st.acc.civic_progress)
        env = {"cities": st.cities, "player": player}
        log = step(r, st.acc, env, self.speed, research=research, civic=civic, boosts=self.sc.boosts(T))
        for ev in log:
            if ev[0] == "completed":
                _, name, item = ev
                c = env_cities[name]
                c["production"]["current"] = None
                if item == "UNIT_BUILDER":
                    st.builders.append(r.q("select BuildCharges from Units where UnitType='UNIT_BUILDER'")[0][0])
                elif item == "UNIT_SETTLER":
                    o = (c["center"]["x"], c["center"]["y"])
                    st.settlers.append((T + 1 + 3 + self.settings.settle_extra_turns, name))  # ~5-6 tiles at 2 moves
                elif item_kind(item) == "BUILDING":
                    c["buildings"].append(item)
                elif item_kind(item) == "DISTRICT":
                    for x in c["districts"]:
                        if x["type"] == item:
                            x["complete"] = True
                    if item == "DISTRICT_GOVERNMENT":
                        st.gov_plaza_built = True
        # Border growth.
        for c in st.cities:
            b = st.border[c["name"]]
            while b[0] >= w.border_cost(b[1]):
                b[0] -= w.border_cost(b[1])
                b[1] += 1
                w.grow_border(c, st.cities, player)
        st.turn += 1
        st.last_output = out
        return st

    def terminal_value(self, st):
        out = getattr(st, "last_output", {})
        return self.settings.terminal_turns * sum(self.settings.score_weights.get(k, 0) * v for k, v in out.items())

    def run(self, st, plan):
        """Simulate to the horizon. `plan` maps decision index -> item; other decisions use the base policy."""
        st = copy.deepcopy(st)
        idx = [0]

        def choose(s, city, options):
            k = idx[0]
            idx[0] += 1
            want = plan.get(k)
            return want if want in options else self.base_policy(s, city, options, None)

        while st.turn <= self.settings.horizon:
            self.advance(st, choose)
        return st, st.score + self.terminal_value(st)

    def rollout_search(self, st0, passes=1, log=print):
        """Rollout improvement: fix decisions one at a time, each chosen by simulating every option to the horizon."""
        plan = {}
        final, best_val = self.run(st0, plan)
        log(f"base policy: value {best_val:.1f}, {len(final.decisions)} decisions")
        for p in range(passes):
            k = 0
            while k < len(final.decisions):
                turn, city, current = final.decisions[k]
                # Options available at that point: replay up to it to read them.
                options = self._options_at(st0, plan, k)
                best_item, best = current, best_val
                for item in options:
                    if item == current:
                        continue
                    trial = dict(plan)
                    trial[k] = item
                    _, v = self.run(st0, trial)
                    if v > best + 1e-6:
                        best_item, best = item, v
                if best_item != current:
                    plan[k] = best_item
                    final, best_val = self.run(st0, plan)
                    log(f"  T{turn} {city}: {current} -> {best_item}  value {best_val:.1f}")
                else:
                    plan[k] = current
                k += 1
        return plan, final, best_val

    def _options_at(self, st0, plan, k):
        st = copy.deepcopy(st0)
        idx = [0]
        found = []

        def choose(s, city, options):
            i = idx[0]
            idx[0] += 1
            if i == k:
                found.extend(options)
            want = plan.get(i)
            return want if want in options else self.base_policy(s, city, options, None)

        while st.turn <= self.settings.horizon and not found:
            self.advance(st, choose)
        return found


def recorded_queues(rows):
    """Per city, the items the player worked on, in order (consecutive repeats collapsed)."""
    queues = {}
    for d in rows:
        for c in d["cities"]:
            item = c["production"]["current"]
            q = queues.setdefault(c["name"], [])
            if item and (not q or q[-1] != item):
                q.append(item)
    return queues


def recorded_sites(rows):
    """Cities founded after the first, in order: [(name, (x, y))]."""
    seen, out = set(), []
    for d in rows:
        for c in d["cities"]:
            if c["name"] not in seen:
                seen.add(c["name"])
                if not c["capital"]:
                    out.append((c["name"], (c["center"]["x"], c["center"]["y"])))
    return out


def run_recorded(planner, st0, rows):
    """Evaluate the recorded production order (and settling sites) inside the planner's model."""
    queues = {k: list(v) for k, v in recorded_queues(rows).items()}
    planner.extra_items = sorted({i for q in queues.values() for i in q})
    st = copy.deepcopy(st0)

    def choose(s, city, options):
        q = queues.get(city["name"], [])
        while q:
            item = q.pop(0)
            if item in options:
                return item
        return planner.base_policy(s, city, options, None)

    while st.turn <= planner.settings.horizon:
        planner.advance(st, choose)
    planner.extra_items = ()
    return st, st.score + planner.terminal_value(st)
