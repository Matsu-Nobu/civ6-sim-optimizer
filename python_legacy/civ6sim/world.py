"""The planner's world: map geometry, territory growth, citizen placement, builders and settling.

City environments use the same dict shape as the save export, so `sim.step` runs on them unchanged.
"""
import copy
import math

from .rules import Rules
from .yields import tile_yields

# Border growth: culture of the n-th tile gained by culture (purchases don't count) =
# floor(0.5 * (10 + (6n)^1.3)) on online speed; the city's culture output accumulates with carry-over.
# Confirmed with the game's GetNextPlotCultureCost (17 at n=2, 26 at n=3, 5 at n=0).
BORDER_COST_SCALE = 0.5
WORKABLE_RADIUS = 3


class Map:
    def __init__(self, map_json):
        self.width, self.height = map_json["width"], map_json["height"]
        self.plots = {(p["x"], p["y"]): p for p in map_json["plots"]}

    def neighbours(self, x, y):
        d = [(1, 0), (-1, 0), (0, -1), (1, -1), (0, 1), (1, 1)] if y & 1 else [(1, 0), (-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1)]
        return [((x + dx) % self.width, y + dy) for dx, dy in d if 0 <= y + dy < self.height]

    # The save stores, per plot, a river flag for each of its six edges; bit -> index into neighbours() (checked on
    # the rs2 map: every set bit is matched by the opposite bit on that neighbour).
    RIVER_BIT_OF_NEIGHBOUR = {5: 0, 0: 1, 3: 2, 2: 3, 1: 4, 4: 5}

    def river_between(self, a, b):
        p = self.plots.get(a)
        edges = p.get("riverEdges") if p else None
        if not edges:
            return False
        for j, n in enumerate(self._neighbours_all(*a)):
            if n == b:
                return bool(edges >> self.RIVER_BIT_OF_NEIGHBOUR[j] & 1)
        return False

    def _neighbours_all(self, x, y):
        """neighbours() with the same direction order, keeping off-map slots as None (for the river bits)."""
        d = [(1, 0), (-1, 0), (0, -1), (1, -1), (0, 1), (1, 1)] if y & 1 else [(1, 0), (-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1)]
        return [((x + dx) % self.width, y + dy) if 0 <= y + dy < self.height else None for dx, dy in d]

    def _cube(self, x, y):
        return x - (y - (y & 1)) // 2, y

    def distance(self, a, b):
        best = None
        for wrap in (-self.width, 0, self.width):  # the map wraps east-west
            q1, r1 = self._cube(*a)
            q2, r2 = self._cube(b[0] + wrap, b[1])
            dq, dr = q1 - q2, r1 - r2
            d = (abs(dq) + abs(dr) + abs(dq + dr)) // 2
            best = d if best is None else min(best, d)
        return best

    def within(self, center, radius):
        """Plots within `radius` steps (breadth-first over neighbours; cached)."""
        key = (center, radius)
        cache = self.__dict__.setdefault("_within", {})
        if key not in cache:
            seen, frontier = {center}, [center]
            for _ in range(radius):
                nxt = []
                for xy in frontier:
                    for n in self.neighbours(*xy):
                        if n not in seen:
                            seen.add(n)
                            nxt.append(n)
                frontier = nxt
            cache[key] = sorted(seen)
        return cache[key]

    def is_lake(self, xy):
        """A water plot in a body of at most 9 water plots (the game's lake size)."""
        cache = self.__dict__.setdefault("_lake", {})
        if xy not in cache:
            if not self.plots[xy]["terrain"].endswith(("COAST", "OCEAN")):
                cache[xy] = False
                return False
            seen, frontier = {xy}, [xy]
            while frontier and len(seen) <= 9:
                nxt = []
                for a in frontier:
                    for n in self.neighbours(*a):
                        if n not in seen and self.plots[n]["terrain"].endswith(("COAST", "OCEAN")):
                            seen.add(n)
                            nxt.append(n)
                frontier = nxt
            small = len(seen) <= 9
            for a in seen:
                cache[a] = small
        return cache[xy]

    def is_land(self, xy):
        t = self.plots[xy]["terrain"]
        return not (t.endswith("OCEAN") or t.endswith("COAST") or t.endswith("MOUNTAIN"))


def move_cost(rules, game_map, a, b, left, full, road_type="ROUTE_ANCIENT_ROAD", cities=()):
    """Movement points a land unit spends to step from plot a to b with `left` of `full` points, or None if it cannot
    enter b this turn. Rules (user, 2026-10-07): a step costing more than the points left is not possible, except with
    full points; crossing a river costs 3 or all points left, whichever is less; terrain and feature costs from the
    database (hills 2, forest/jungle/marsh +1); a step from a road plot to a road plot costs the era's road cost
    whatever the terrain (`road_type`: the road of the player's era; bridges from the medieval road on); mountains,
    water and impassable features cannot be entered; entering a city costs CITY_MOVEMENT_COST whatever the terrain
    (checked on rome2: 705 + these of 743 recorded one-turn moves)."""
    p = game_map.plots.get(b)
    if p is None or p["terrain"].endswith("MOUNTAIN") or p["terrain"] in ("TERRAIN_COAST", "TERRAIN_OCEAN"):
        return None
    if p["feature"] and rules.q("select Impassable from Features where FeatureType=?", p["feature"])[0][0]:
        return None
    pa = game_map.plots[a]
    road = pa.get("route") is not None and p.get("route") is not None
    road_cost, bridges = rules.q("select MovementCost, SupportsBridges from Routes where RouteType=?", road_type)[0]
    # no river crossing cost into or out of a city (rome2: AI units crossed a river into / out of a city and moved on)
    if game_map.river_between(a, b) and not (road and bridges) and a not in cities and b not in cities:
        cost = min(3, left)
    elif road:
        cost = road_cost
    elif b in cities:
        cost = float(rules.param("CITY_MOVEMENT_COST"))
    else:
        cost = rules.q("select MovementCost from Terrains where TerrainType=?", p["terrain"])[0][0]
        if p["feature"]:
            cost += rules.q("select MovementChange from Features where FeatureType=?", p["feature"])[0][0]
    if cost <= left or left == full:
        return min(cost, left)
    return None


def travel_turns(rules, game_map, start, goal, moves, must_have_moves=True, road_type="ROUTE_ANCIENT_ROAD", cities=()):
    """Turns until a unit with `moves` points per turn, starting on `start` with full points, stands on `goal` with
    points left to act (0 = this turn). None if unreachable within 30 turns."""
    import heapq
    if start == goal:
        return 0
    best = {start: (0, -moves)}
    heap = [(0, -moves, start)]
    while heap:
        t, neg_left, at = heapq.heappop(heap)
        left = -neg_left
        if best.get(at, (99, 0)) < (t, neg_left):
            continue
        if at == goal:
            return t if (left > 0 or not must_have_moves) else t + 1
        for n in game_map.neighbours(*at):
            for tt, ll in ((t, left), (t + 1, moves)):
                if ll <= 0:
                    continue
                c = move_cost(rules, game_map, at, n, ll, moves, road_type, cities)
                if c is None:
                    continue
                state = (tt, -(ll - c))
                if tt <= 30 and state < best.get(n, (99, 0)):
                    best[n] = state
                    heapq.heappush(heap, (tt, -(ll - c), n))
                break  # waiting a turn only helps when the step is not possible now
    return None


def plot_value(rules, plot, player, weights):
    y = tile_yields(rules, plot, set(player["techs"]), set(player["civics"]))
    return sum(weights.get(k, 0) * v for k, v in y.items())


DEFAULT_WEIGHTS = {"YIELD_FOOD": 1.0, "YIELD_PRODUCTION": 1.0, "YIELD_GOLD": 0.5, "YIELD_SCIENCE": 0.8,
                   "YIELD_CULTURE": 0.6, "YIELD_FAITH": 0.2}


class World:
    def __init__(self, rules: Rules, game_map: Map, player_id=0):
        self.rules, self.map = rules, game_map
        # Tiles owned by other civs / city-states at the start; we never take those.
        self.foreign = {xy for xy, p in game_map.plots.items() if p.get("owner") is not None and p["owner"] != player_id}

    # --- territory --------------------------------------------------------------------------
    def border_cost(self, n):
        return math.floor(BORDER_COST_SCALE * (self.rules.param("CULTURE_COST_FIRST_PLOT")
                                    + (self.rules.param("CULTURE_COST_LATER_PLOT_MULTIPLIER") * n)
                                    ** self.rules.param("CULTURE_COST_LATER_PLOT_EXPONENT")))

    def owned_by_us(self, cities):
        return {(p["x"], p["y"]) for c in cities for p in c["owned"]}

    def grow_border(self, city, cities, player, weights=DEFAULT_WEIGHTS, xy=None):
        """Add the best unowned tile adjacent to the city's territory within the workable radius (or `xy`)."""
        best = xy or self.next_border_tile(city, cities, player, weights)
        if best is None:
            return False
        city["owned"].append(copy.deepcopy(self.map.plots[best]))
        return True

    def next_border_tile(self, city, cities, player, weights=DEFAULT_WEIGHTS):
        """The tile the border would take next (best yields, then closest), or None."""
        mine = self.owned_by_us(cities)
        center = (city["center"]["x"], city["center"]["y"])
        own = {(p["x"], p["y"]) for p in city["owned"]}
        cands = set()
        for xy in own:
            for n in self.map.neighbours(*xy):
                if n not in mine and n not in self.foreign and self.map.distance(center, n) <= WORKABLE_RADIUS:
                    cands.add(n)
        if not cands:
            return None
        # ties: the smallest (x, y) (a set's order is not reproducible elsewhere; the Rust simulator does the same)
        return max(sorted(cands), key=lambda xy: (plot_value(self.rules, self.map.plots[xy], player, weights), -self.map.distance(center, xy)))

    # --- citizens ----------------------------------------------------------------------------
    def workable(self, xy, center, player_engine=None):
        """Citizens work plots within 3 of the center that aren't mountains (unless a trait allows: Inca) or
        impassable features."""
        if self.map.distance(xy, center) > WORKABLE_RADIUS:
            return False
        p = self.map.plots[xy]
        if p["terrain"].endswith("MOUNTAIN"):
            if player_engine is None:
                return False
            ok = {a["TerrainType"] for a, m, s in player_engine.player_effects("EFFECT_ADJUST_PLAYER_TERRAIN_WORK_IMPASSABLE_MODIFIER")}
            if p["terrain"] not in ok:
                return False
        if p["feature"] and self.rules.q("select 1 from Features where FeatureType=? and Impassable=1", p["feature"]):
            return False
        return True

    def assign_citizens(self, city, player, weights=DEFAULT_WEIGHTS, min_surplus=1, cities=None):
        """Work the best tiles by their full yields (modifiers included): highest weighted yield, but keep at least
        `min_surplus` food if possible."""
        from .modifiers import engine_for
        from .yields import appeal_modifiers, city_plot_modifiers, plots_by_xy_of
        cities = cities or [city]
        center = (city["center"]["x"], city["center"]["y"])
        district_tiles = {(d["x"], d["y"]) for d in city["districts"]}
        eng = engine_for(self.rules, player, cities)
        techs, civics = set(player["techs"]), set(player["civics"])
        mods = city_plot_modifiers(self.rules, player, city, cities)
        pxy = plots_by_xy_of(cities)
        amods = appeal_modifiers(eng, city["name"])
        ty = lambda p, c=False: tile_yields(self.rules, p, techs, civics, city_center=c, plot_modifiers=mods,
                                            plots_by_xy=pxy, city=city, appeal_mods=amods)
        tiles = [p for p in city["owned"] if (p["x"], p["y"]) != center and (p["x"], p["y"]) not in district_tiles
                 and self.workable((p["x"], p["y"]), center, eng)]
        scored = []
        for p in tiles:
            y = ty(p)
            scored.append((sum(weights.get(k, 0) * v for k, v in y.items()), y["YIELD_FOOD"], p))
        scored.sort(key=lambda t: (-t[0], -t[1]))
        food_of = {(t[2]["x"], t[2]["y"]): t[1] for t in scored}
        chosen = [t[2] for t in scored[:city["population"]]]
        # Food check: swap in food tiles while the surplus is too low.
        center_plot = next(p for p in city["owned"] if (p["x"], p["y"]) == center)
        center_food = ty(center_plot, True)["YIELD_FOOD"]

        def surplus(sel):
            return center_food + sum(food_of[(p["x"], p["y"])] for p in sel) - 2 * city["population"]
        spare = sorted([t[2] for t in scored[city["population"]:]], key=lambda p: -food_of[(p["x"], p["y"])])
        while spare and chosen and surplus(chosen) < min_surplus:
            worst = min(chosen, key=lambda p: food_of[(p["x"], p["y"])])
            better = spare.pop(0)
            if food_of[(better["x"], better["y"])] <= food_of[(worst["x"], worst["y"])]:
                break
            chosen[chosen.index(worst)] = better
        city["worked"] = [dict(center_plot, workers=1)] + [dict(p, workers=1) for p in chosen]

    # --- builders ----------------------------------------------------------------------------
    def valid_improvements(self, plot, techs):
        r = self.rules
        out = []
        for imp, prereq in r.q("select ImprovementType, PrereqTech from Improvements where Buildable=1"):
            if prereq and prereq not in techs:
                continue
            if plot["resource"]:
                row = r.q("select MustRemoveFeature from Improvement_ValidResources where ImprovementType=? and ResourceType=?",
                          imp, plot["resource"])
                if row:
                    if not plot["feature"] or not row[0][0]:
                        out.append(imp)
                    continue
            terrain_ok = bool(r.q("select 1 from Improvement_ValidTerrains where ImprovementType=? and TerrainType=?", imp, plot["terrain"]))
            feature_ok = plot["feature"] is None or bool(
                r.q("select 1 from Improvement_ValidFeatures where ImprovementType=? and FeatureType=?", imp, plot["feature"]))
            if terrain_ok and feature_ok and imp in ("IMPROVEMENT_FARM", "IMPROVEMENT_MINE"):
                out.append(imp)
        return out

    def best_improvement(self, cities, player, weights=DEFAULT_WEIGHTS):
        """(city, plot, improvement, gain) for the most valuable improvement on a worked tile, else any owned tile."""
        techs, civics = set(player["techs"]), set(player["civics"])
        best = None
        for c in cities:
            worked = {(w["x"], w["y"]) for w in c["worked"]}
            center = (c["center"]["x"], c["center"]["y"])
            districts = {(d["x"], d["y"]) for d in c["districts"]}
            for p in c["owned"]:
                xy = (p["x"], p["y"])
                if p["improvement"] or xy == center or xy in districts or not self.map.is_land(xy):
                    continue
                base = sum(weights.get(k, 0) * v for k, v in tile_yields(self.rules, p, techs, civics).items())
                for imp in self.valid_improvements(p, techs):
                    after = dict(p, improvement=imp)
                    gain = sum(weights.get(k, 0) * v for k, v in tile_yields(self.rules, after, techs, civics).items()) - base
                    gain += 0.3 if imp in ("IMPROVEMENT_PLANTATION", "IMPROVEMENT_PASTURE", "IMPROVEMENT_CAMP") else 0  # housing / amenity
                    gain *= 1.0 if xy in worked else 0.5
                    if gain > 0 and (best is None or gain > best[3]):
                        best = (c, p, imp, gain)
        return best

    # --- settling ----------------------------------------------------------------------------
    def found_city(self, name, site, cities, player, capital=False):
        mine = self.owned_by_us(cities)
        owned = [copy.deepcopy(self.map.plots[site])]
        for n in self.map.neighbours(*site):
            if n not in mine and n not in self.foreign:
                owned.append(copy.deepcopy(self.map.plots[n]))
        # extra tiles on founding (Russia's Mother Russia +5, BBG Religious Settlements +2): the best free tiles in
        # reach of the new city
        from .modifiers import engine_for
        extra = sum(int(float(a["Amount"])) for a, m, s in engine_for(self.rules, player, cities or [])
                    .player_effects("EFFECT_ADJUST_PLAYER_CITY_TILES")) if cities else 0
        if extra:
            have = {(p["x"], p["y"]) for p in owned}
            cand = [xy for xy in self.map.within(site, WORKABLE_RADIUS) if xy not in have and xy not in mine
                    and xy not in self.foreign and self.map.distance(xy, site) == 2]
            cand.sort(key=lambda xy: -plot_value(self.rules, self.map.plots[xy], player, DEFAULT_WEIGHTS))
            owned += [copy.deepcopy(self.map.plots[xy]) for xy in cand[:extra]]
        city = {
            "name": name, "id": (len(cities) + 1) << 16 | len(cities), "capital": capital, "slot": len(cities),
            "center": {"x": site[0], "y": site[1]}, "population": 1, "foodStore": 0.0,
            "worked": [], "owned": owned, "production": {"current": None, "progress": {}},
            "buildings": [], "districts": [{"type": "DISTRICT_CITY_CENTER", "x": site[0], "y": site[1], "complete": True,
                                             "plot": copy.deepcopy(self.map.plots[site]), "neighbours": []}],
        }
        self.assign_citizens(city, player)
        return city

    def site_score(self, site, cities, player, weights=DEFAULT_WEIGHTS):
        """Value of the best 6 tiles in ring 1-2 around a candidate site (0 if not settleable)."""
        if not self.map.is_land(site) or site in self.foreign:
            return 0
        if any(self.map.distance(site, (c["center"]["x"], c["center"]["y"])) < 4 for c in cities):
            return 0
        vals = sorted((plot_value(self.rules, self.map.plots[xy], player, weights)
                       for xy in self.map.within(site, 2) if xy != site and self.map.is_land(xy)), reverse=True)
        fresh = 2 if self.map.plots[site]["river"] else 0
        return sum(vals[:6]) + fresh
