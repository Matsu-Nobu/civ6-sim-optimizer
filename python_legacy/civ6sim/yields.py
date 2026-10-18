"""Static yield calculation: given a city's state (from a save export), compute its yields."""
import math
from collections import defaultdict

from .envs import governor_here

from .rules import YIELDS, Rules


# AI difficulty bonus measured on the AI civs of an autoplay game (same for all three; % on top of other modifiers,
# food unaffected). Depends on the game's difficulty; human players get none.
# The AI's difficulty bonus: BBG/base TRAIT_LEADER_MAJOR_CIV's HIGH_DIFFICULTY_*_SCALING modifiers, whose Amount is
# LinearScaleFromDefaultHandicap (20 per level for production and gold, 8 for science / culture / faith); the
# game_auto2 AIs are one level above the default. The handicap is not read from the save yet, so the engine leaves
# these modifiers inert (their requirement REQUIREMENT_PLAYER_HANDICAP_AT_OR_ABOVE is unknown to it) and this
# table stands in for them.
AI_DIFFICULTY_BONUS = {"YIELD_PRODUCTION": 20, "YIELD_GOLD": 20, "YIELD_SCIENCE": 8, "YIELD_CULTURE": 8, "YIELD_FAITH": 8}


def percent_modifiers(happiness_pct, is_ai=False, extra=None):
    """Total % on each yield: amenity state (non-food), the AI difficulty bonus and modifier percents (extra)."""
    out = {}
    for k in ("YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"):
        out[k] = (happiness_pct if k != "YIELD_FOOD" else 0) + (AI_DIFFICULTY_BONUS.get(k, 0) if is_ai else 0) \
            + (extra or {}).get(k, 0)
    return out


# BBG "Better Balanced City Center" lobby option: minimum yields for a city center tile.
# mode: "capital" (BBCC applies to the capital only), "all", or "off".
# The minimums are BBG's own tables (Flat_CutOffYieldValues / Hill_CutOffYieldValues).
def bbcc_minimum(rules, hills):
    """BBCC's minimum city center yields; none when the game was set up without BBCC (its tables are absent)."""
    t = "Hill_CutOffYieldValues" if hills else "Flat_CutOffYieldValues"
    if not rules.q("select 1 from sqlite_master where type='table' and name=?", t):
        return {}
    return dict(rules.q(f"select YieldType, Amount from {t}"))


def hex_distance(a, b):
    """Distance between two (x, y) plots in odd-r offset coordinates (odd rows shifted right; no wrap)."""
    def cube(x, y):
        q = x - (y - (y & 1)) // 2
        return q, y, -q - y
    (q1, r1, s1), (q2, r2, s2) = cube(*a), cube(*b)
    return max(abs(q1 - q2), abs(r1 - r2), abs(s1 - s2))


def _plots_in_range(plot, ctx, lo=1, hi=1):
    plots = ctx.get("plots") or {}
    xy = (plot["x"], plot["y"])
    if hi == 1 and lo <= 1:
        return [plots[n] for n in neighbour_xy(*xy) if n in plots]
    return [p for k, p in plots.items() if lo <= hex_distance(xy, k) <= hi]


def _rng(a):
    return int(a.get("MinRange") or 1), int(a.get("MaxRange") or 1)


def _improved_class(rules, plot, cls):
    r = plot["resource"]
    return bool(r) and rules.resource_class(r) == cls and not plot.get("pillaged") and plot["improvement"] in rules.valid_improvements(r)


def _is_impassable(rules, plot):
    t = rules.q("select Impassable from Terrains where TerrainType=?", plot["terrain"])
    f = plot["feature"] and rules.q("select Impassable from Features where FeatureType=?", plot["feature"])
    return bool(t and t[0][0]) or bool(f and f[0][0])


def _requirementset_met(rules, plot, a, ctx):
    reqs, test_any = rules.requirement_set(a["RequirementSetId"])
    results = []
    for rtype, args, inverse in reqs:
        test = PLOT_REQUIREMENTS.get(rtype)
        if test is None:
            return False  # unknown nested condition: don't apply rather than guess
        results.append(bool(test(rules, plot, args, ctx)) != inverse)
    return any(results) if test_any else all(results)


# Plot requirements we can evaluate from a save export: requirement type -> test(rules, plot, args, ctx), where ctx
# carries the player's techs/civics, the plots we know (by (x, y)) and the plot's city.
PLOT_REQUIREMENTS = {
    "REQUIREMENT_PLOT_RESOURCE_TAG_MATCHES": lambda r, p, a, c: a["Tag"] in r.tags_of(p["resource"]),
    "REQUIREMENT_PLOT_RESOURCE_TYPE_MATCHES": lambda r, p, a, c: p["resource"] == a["ResourceType"],
    "REQUIREMENT_PLOT_RESOURCE_CLASS_TYPE_MATCHES": lambda r, p, a, c: bool(p["resource"]) and r.resource_class(p["resource"]) == a["ResourceClassType"],
    "REQUIREMENT_PLOT_IMPROVED_RESOURCE_CLASS_TYPE_MATCHES": lambda r, p, a, c: _improved_class(r, p, a["ResourceClassType"]),
    "REQUIREMENT_PLOT_RESOURCE_VISIBLE": lambda r, p, a, c: bool(p["resource"]) and r.resource_revealed(p["resource"], c.get("techs", ())),
    "REQUIREMENT_PLAYER_HAS_RESOURCE_VISIBILITY": lambda r, p, a, c: r.resource_revealed(a["ResourceType"], c.get("techs", ())),
    "REQUIREMENT_PLOT_IMPROVEMENT_TYPE_MATCHES": lambda r, p, a, c: p["improvement"] == a["ImprovementType"],
    "REQUIREMENT_PLOT_FEATURE_TYPE_MATCHES": lambda r, p, a, c: p["feature"] == a["FeatureType"],
    "REQUIREMENT_PLOT_TERRAIN_TYPE_MATCHES": lambda r, p, a, c: p["terrain"] == a["TerrainType"],
    "REQUIREMENT_PLOT_TERRAIN_CLASS_MATCHES": lambda r, p, a, c: bool(r.q(
        "select 1 from TerrainClass_Terrains where TerrainClassType=? and TerrainType=?", a["TerrainClass"], p["terrain"])),
    "REQUIREMENT_PLOT_IS_HILLS": lambda r, p, a, c: p["terrain"].endswith("_HILLS"),
    "REQUIREMENT_PLOT_IS_MOUNTAIN": lambda r, p, a, c: p["terrain"].endswith("_MOUNTAIN"),
    "REQUIREMENT_PLOT_IS_IMPASSABLE": lambda r, p, a, c: _is_impassable(r, p),
    "REQUIREMENT_PLOT_HAS_ANY_FEATURE": lambda r, p, a, c: p["feature"] is not None,
    "REQUIREMENT_PLOT_HAS_ANY_RESOURCE": lambda r, p, a, c: p["resource"] is not None,
    "REQUIREMENT_PLOT_HAS_ANY_IMPROVEMENT": lambda r, p, a, c: p["improvement"] is not None,
    "REQUIREMENT_PLOT_DISTRICT_TYPE_MATCHES": lambda r, p, a, c: p.get("districtType") == a["DistrictType"],
    "REQUIREMENT_PLOT_ADJACENT_TO_RIVER": lambda r, p, a, c: bool(p.get("river")),
    "REQUIREMENT_PLOT_IS_LAKE": lambda r, p, a, c: bool(p.get("lake")),
    "REQUIREMENT_PLOT_ADJACENT_IMPROVEMENT_TYPE_MATCHES": lambda r, p, a, c: any(
        q["improvement"] == a["ImprovementType"] and not q.get("pillaged") for q in _plots_in_range(p, c, *_rng(a))),
    "REQUIREMENT_PLOT_ADJACENT_DISTRICT_TYPE_MATCHES": lambda r, p, a, c: any(
        q.get("districtType") == a["DistrictType"] for q in _plots_in_range(p, c, *_rng(a))),
    "REQUIREMENT_PLOT_ADJACENT_FEATURE_TYPE_MATCHES": lambda r, p, a, c: any(
        q["feature"] == a["FeatureType"] for q in _plots_in_range(p, c)),
    "REQUIREMENT_PLAYER_HAS_TECHNOLOGY": lambda r, p, a, c: a["TechnologyType"] in c.get("techs", ()),
    "REQUIREMENT_PLAYER_HAS_CIVIC": lambda r, p, a, c: a["CivicType"] in c.get("civics", ()),
    "REQUIREMENT_CITY_HAS_BUILDING": lambda r, p, a, c: bool(c.get("city")) and a["BuildingType"] in c["city"]["buildings"],
    "REQUIREMENT_CITY_HAS_DISTRICT": lambda r, p, a, c: bool(c.get("city")) and any(
        d["type"] == a["DistrictType"] and d["complete"] for d in c["city"]["districts"]),
    "REQUIREMENT_CITY_FOLLOWS_PANTHEON": lambda r, p, a, c: True,
    "REQUIREMENT_PLOT_IS_APPEAL_BETWEEN": lambda r, p, a, c: (
        int(a.get("MinimumAppeal") or -999) <= appeal(r, p, c.get("plots"), c.get("appeal_mods")) <= int(a.get("MaximumAppeal") or 999)),
    "REQUIREMENT_REQUIREMENTSET_IS_MET": _requirementset_met,
    # internal: pins a single-plot effect to its plot
    "REQUIREMENT_PLOT_AT_XY": lambda r, p, a, c: (p["x"], p["y"]) == (a["X"], a["Y"]),
}
PLOT_YIELD_MODIFIER_TYPES_SINGLE = ("MODIFIER_SINGLE_PLOT_ADJUST_PLOT_YIELDS",)
PLOT_YIELD_MODIFIER_TYPES = ("MODIFIER_CITY_PLOT_YIELDS_ADJUST_PLOT_YIELD", "MODIFIER_PLAYER_ADJUST_PLOT_YIELD")


def plots_by_xy_of(cities):
    """(x, y) -> plot for every owned plot and the plots bordering them, with districtType filled in."""
    kinds = {(d["x"], d["y"]): d["type"] for c in cities for d in c["districts"]}
    done = {(d["x"], d["y"]): d["complete"] for c in cities for d in c["districts"]}
    out = {(p["x"], p["y"]): p for c in cities for p in c.get("near", ())}
    out.update({(p["x"], p["y"]): dict(p, districtType=kinds.get((p["x"], p["y"])), districtComplete=done.get((p["x"], p["y"])),
                                       wonderComplete=p.get("wonder") in c["buildings"] if p.get("wonder") else None)
                for c in cities for p in c["owned"]})
    return out


def _appeal_of(rules, table, col, key):
    if not key:
        return 0
    row = rules.q(f"select Appeal from {table} where {col}=?", key)
    return (row[0][0] or 0) if row else 0


def appeal(rules: Rules, plot, plots_by_xy, mods=None):
    """A plot's appeal: the sum of what its neighbours contribute (terrain such as coast or mountains, features,
    improvements, districts), plus 1 for a river along the plot (fitted on probe data). Neighbours we know
    nothing about count 0. mods (appeal_modifiers): the city's appeal changes from modifiers."""
    if plot["terrain"] in ("TERRAIN_COAST", "TERRAIN_OCEAN"):
        return 0  # water plots never meet appeal conditions (Earth Goddess: 0 of 403 Canadian water tiles)
    mods = mods or {}
    total = (1 if plot.get("river") else 0) + mods.get("city", 0)
    unimproved_feature_near = False
    for xy in neighbour_xy(plot["x"], plot["y"]):
        q = (plots_by_xy or {}).get(xy)
        if not q:
            continue
        total += _appeal_of(rules, "Terrains", "TerrainType", q["terrain"])
        total += _appeal_of(rules, "Features", "FeatureType", q["feature"]) + mods.get("feature", {}).get(q["feature"], 0)
        if q["feature"] and not q["improvement"] and not rules.is_natural_wonder(q["feature"]):
            unimproved_feature_near = True
        if not q.get("pillaged"):
            total += _appeal_of(rules, "Improvements", "ImprovementType", q["improvement"])
        if q.get("wonderComplete") is not False:  # a wonder still under construction adds nothing
            total += _appeal_of(rules, "Districts", "DistrictType", q.get("districtType"))
    if unimproved_feature_near:
        total += mods.get("near_unimproved_feature", 0)
    return total


def appeal_modifiers(engine, city_name):
    """Appeal changes from modifiers for a city's plots: city-wide (Eiffel Tower, Golden Gate, Wish You Were Here),
    per feature type (Amazon rainforest, Biosphere), next to unimproved features (Reyna's Forestry Management)."""
    if engine is None or city_name is None:
        return {}
    mods = {"city": 0.0, "feature": defaultdict(float), "near_unimproved_feature": 0.0}
    for a, m, s in engine.city_effects("EFFECT_ADJUST_CITY_APPEAL", city_name):
        mods["city"] += float(a["Amount"])
    for a, m, s in engine.city_effects("EFFECT_ADJUST_FEATURE_APPEAL_MODIFIER", city_name):
        mods["feature"][a["FeatureType"]] += float(a["Amount"])
    for a, m, s in engine.city_effects("EFFECT_ADJUST_FEATURE_NO_IMPROVEMENT_APPEAL_GOVERNOR", city_name):
        mods["near_unimproved_feature"] += float(a["Amount"])
    return mods


def city_plot_modifiers(rules: Rules, player, city, cities=()):
    """All plot-yield modifiers acting on this city's tiles: [(yield, amount, plot requirements, test_any)]."""
    from .modifiers import engine_for
    e = engine_for(rules, player, list(cities) or [city])
    return [m[:4] for m in e.plot_effects.get(city["name"], ())]


def governor_plot_modifiers(rules: Rules, player, city):
    """Plot-yield modifiers of an established governor (and its promotions) in this city."""
    mods = []
    for g in player.get("governors", []):
        if not governor_here(g, city, player) or not g["established"]:
            continue
        ids = [m for (m,) in rules.q("select ModifierId from GovernorModifiers where GovernorType=?", g["type"])]
        for p in g["promotions"]:
            ids += [m for (m,) in rules.q("select ModifierId from GovernorPromotionModifiers where GovernorPromotionType=?", p)]
        mods += [m for mid in ids for m in rules.plot_yield_modifier(mid)]
    return mods


def plot_modifier_yields(rules: Rules, plot, modifiers, ctx=None):
    y = defaultdict(float)
    ctx = ctx or {}
    for yield_type, amount, reqs, test_any in modifiers:
        results = []
        for req_type, args, inverse in reqs:
            results.append(bool(PLOT_REQUIREMENTS[req_type](rules, plot, args, ctx)) != inverse)
        if (any(results) if test_any else all(results)):
            y[yield_type] += amount
    return y


def neighbour_xy(x, y):
    d = [(1, 0), (-1, 0), (0, -1), (1, -1), (0, 1), (1, 1)] if y & 1 else [(1, 0), (-1, 0), (-1, -1), (0, -1), (-1, 1), (0, 1)]
    return [(x + dx, y + dy) for dx, dy in d]


def improvement_adjacency(rules: Rules, plot, plots_by_xy, techs, civics):
    """Adjacency yields of an improvement (e.g. BBG Kurgan +1 production per adjacent pasture)."""
    y = defaultdict(float)
    imp = plot.get("improvement")
    if not imp or plot.get("pillaged") or not plots_by_xy:
        return y
    neighbours = [plots_by_xy[xy] for xy in neighbour_xy(plot["x"], plot["y"]) if xy in plots_by_xy]
    for row in rules.improvement_adjacencies(imp):
        if row["PrereqTech"] and row["PrereqTech"] not in techs or row["PrereqCivic"] and row["PrereqCivic"] not in civics:
            continue
        if row["ObsoleteTech"] and row["ObsoleteTech"] in techs or row["ObsoleteCivic"] and row["ObsoleteCivic"] in civics:
            continue
        if row["AdjacentRiver"]:
            n = 1 if plot.get("river") else 0
        else:
            n = sum(1 for p in neighbours if _adjacent_match(rules, row, p)
                    and not (row["AdjacentImprovement"] and p.get("pillaged")))
        y[row["YieldType"]] += (n // max(row["TilesRequired"], 1)) * row["YieldChange"]
    return y


def tile_yields(rules: Rules, plot, techs=(), civics=(), city_center=False, bbcc=False, plot_modifiers=(), plots_by_xy=None,
                city=None, appeal_mods=None):
    y = defaultdict(float)
    ctx = {"techs": techs, "civics": civics, "plots": plots_by_xy, "city": city, "appeal_mods": appeal_mods}
    if plot.get("district") and not city_center:
        return y  # a district (even unfinished) replaces the tile's own yields
    if city_center:
        plot = dict(plot, districtType="DISTRICT_CITY_CENTER")
    resource = plot["resource"]
    if resource and not rules.resource_revealed(resource, techs):
        resource = None  # strategic resources yield nothing until their tech reveals them (iron: Bronze Working)
    # Impassable features (ice, impassable natural wonders) take away the terrain's yields.
    terrain = None if plot["feature"] and _is_impassable(rules, dict(plot, terrain="")) else plot["terrain"]
    for src, key in ((rules.terrain_yields, terrain), (rules.feature_yields, plot["feature"]),
                     (rules.resource_yields, resource)):
        if key:
            for k, v in src[key].items():
                y[k] += v
    for k, v in (plot.get("yieldChanges") or {}).items():  # left by floods / eruptions
        y[k] += v
    imp = plot["improvement"]
    if imp and not plot.get("pillaged"):
        for k, v in rules.improvement_yields[imp].items():
            y[k] += v
        for i, k, v, tech, civic in rules.improvement_bonus_yields:
            if i == imp and (tech is None or tech in techs) and (civic is None or civic in civics):
                y[k] += v
    if city_center:
        y["YIELD_FOOD"] = max(y["YIELD_FOOD"], rules.param("YIELD_FOOD_CITY_TERRAIN_REPLACE"))
        y["YIELD_PRODUCTION"] = max(y["YIELD_PRODUCTION"], rules.param("YIELD_PRODUCTION_CITY_TERRAIN_REPLACE"))
        if bbcc:
            for k, v in bbcc_minimum(rules, plot["terrain"].endswith("_HILLS")).items():
                y[k] = max(y[k], v)
    # Plot modifiers (pantheon etc.) apply on top of the city-center minimum: with Fertility Rites a
    # plains/maize center (raised to 2 food) yields 3.
    if not _is_impassable(rules, plot):  # natural wonders / ice take no modifier yields (e.g. Earth Goddess)
        for k, v in plot_modifier_yields(rules, plot, plot_modifiers, ctx).items():
            y[k] += v
        # natural wonders give yields to each adjacent plot (once per adjacent wonder tile)
        for xy in neighbour_xy(plot["x"], plot["y"]):
            q = (plots_by_xy or {}).get(xy)
            if q and q["feature"]:
                for k, v in rules.q("select YieldType, YieldChange from Feature_AdjacentYields where FeatureType=?", q["feature"]):
                    y[k] += v
    if imp and not plot.get("pillaged"):  # modifiers the improvement itself carries (e.g. BBG flat plantations +1 production)
        for k, v in plot_modifier_yields(rules, plot, rules.improvement_plot_modifiers(imp), ctx).items():
            y[k] += v
        for k, v in improvement_adjacency(rules, plot, plots_by_xy, techs, civics).items():
            y[k] += v
    return y


def specialty_district_count(rules: Rules, city):
    return sum(1 for d in city["districts"] if d["complete"] and rules.district_requires_population(d["type"]))


# City requirements we can evaluate from a save export: requirement type -> test(rules, city, args).
CITY_REQUIREMENTS = {
    "REQUIREMENT_CITY_HAS_DISTRICT": lambda rules, city, a: any(
        d["type"] == a["DistrictType"] and d["complete"] for d in city["districts"]),
    "REQUIREMENT_CITY_HAS_BUILDING": lambda rules, city, a: rules.city_has_building(city, a["BuildingType"]),
    "REQUIREMENT_CITY_HAS_X_SPECIALTY_DISTRICTS": lambda rules, city, a: specialty_district_count(rules, city) >= int(a["Amount"]),
    # Loyalty is not tracked; the cities in our games so far sit at full loyalty.
    "REQUIREMENT_CITY_HAS_HIGH_LOYALTY": lambda rules, city, a: True,
}


ADJ_COLS = ("ID", "Description", "YieldType", "YieldChange", "TilesRequired", "OtherDistrictAdjacent", "AdjacentSeaResource",
            "AdjacentTerrain", "AdjacentFeature", "AdjacentRiver", "AdjacentWonder", "AdjacentNaturalWonder",
            "AdjacentImprovement", "AdjacentDistrict", "PrereqCivic", "PrereqTech", "ObsoleteCivic", "ObsoleteTech",
            "AdjacentResource", "AdjacentResourceClass", "Self")


def rules_of(engine):
    return engine.r


def modifier_adjacency_rows(engine, city_name, district_type):
    """Adjacency rows granted by modifiers (e.g. BBG Machu Picchu, pantheons) for a district type in a city."""
    rows = []
    keys = {"EFFECT_FEATURE_ADJACENCY": ("AdjacentFeature", "FeatureType"),
            "EFFECT_TERRAIN_ADJACENCY": ("AdjacentTerrain", "TerrainType"),
            "EFFECT_IMPROVEMENT_ADJACENCY": ("AdjacentImprovement", "ImprovementType")}
    for effect, (col, arg) in keys.items():
        for a, mid, src in engine.city_effects(effect, city_name):
            if not a.get("DistrictType") or not rules_of(engine).district_is(district_type, a["DistrictType"]):
                continue  # a unique district counts as the one it replaces (Lavra for holy site)
            row = dict.fromkeys(ADJ_COLS)
            row.update(ID=mid, YieldType=a["YieldType"], YieldChange=float(a["Amount"]),
                       TilesRequired=int(a.get("TilesRequired") or 1), **{col: a.get(arg)})
            rows.append(row)
    # per adjacent district (Kongo/Lithuania holy sites, Japan, Ludwig's wonders, Theodora) and for a river (Dutch,
    # River Goddess, Ptolemy, Ramses)
    for effect, col in (("EFFECT_DISTRICT_ADJACENCY", "OtherDistrictAdjacent"), ("EFFECT_RIVER_ADJACENCY", "AdjacentRiver")):
        for a, mid, src in engine.city_effects(effect, city_name):
            if not a.get("DistrictType") or not rules_of(engine).district_is(district_type, a["DistrictType"]):
                continue  # a unique district counts as the one it replaces (Lavra for holy site)
            row = dict.fromkeys(ADJ_COLS)
            row.update(ID=mid, YieldType=a["YieldType"], YieldChange=float(a["Amount"]), TilesRequired=1, **{col: 1})
            rows.append(row)
    return rows


def appeal_district_yields(rules: Rules, city, plot, plots_by_xy, appeal_mods=None):
    """Adjacent_AppealYieldChanges: a tile next to a district (the Preserve) whose building is built gets yields by
    its appeal (Grove: charming +1 faith, breathtaking +2 ...), unimproved tiles only where the row says so."""
    y = defaultdict(float)
    near = {(d["x"], d["y"]): d for d in city["districts"] if d["complete"]}
    adj_types = {near[xy]["type"] for xy in neighbour_xy(plot["x"], plot["y"]) if xy in near}
    if not adj_types:
        return {}
    ap = None
    for dist, yt, hi, b, lo, ch, desc, unimproved in rules.q(
            "select DistrictType, YieldType, MaximumValue, BuildingType, MinimumValue, YieldChange, Description, Unimproved "
            "from Adjacent_AppealYieldChanges"):
        if not any(rules.district_is(t, dist) for t in adj_types) or (b and b not in city["buildings"]):
            continue
        if unimproved and plot.get("improvement"):
            continue
        if ap is None:
            ap = appeal(rules, plot, plots_by_xy, appeal_mods)
        if lo <= ap <= hi:
            y[yt] += ch
    return dict(y)


def terrain_from_adjacent_improvements(engine, city_name, plot, plots_by_xy):
    """Inca: a worked mountain gets +1 food per adjacent terrace farm (EFFECT_ADJUST_TERRAIN_YIELD_FROM_ADJACENT_IMPROVEMENTS)."""
    y = defaultdict(float)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_TERRAIN_YIELD_FROM_ADJACENT_IMPROVEMENTS", city_name):
        if plot["terrain"] != a["TerrainType"]:
            continue
        n = sum(1 for xy in neighbour_xy(plot["x"], plot["y"])
                if (q := (plots_by_xy or {}).get(xy)) and q.get("improvement") == a["ImprovementType"] and not q.get("pillaged"))
        if n:
            y[a["YieldType"]] += float(a["Amount"]) * n
    return dict(y)


def modifier_city_yields(rules: Rules, engine, city):
    """{label: {yield: amount}} of flat city yields from modifiers."""
    out = {}
    name = city["name"]

    def add(label, k, v):
        out.setdefault(label, defaultdict(float))[k] += v
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_CHANGE", name):
        add(f"{src} {mid}", a["YieldType"], float(a["Amount"]))
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_ALL_YIELDS_CHANGE", name):
        for k in YIELDS:
            add(f"{src} {mid}", k, float(a["Amount"]))
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_BUILDING_YIELD_CHANGE", name):
        if a.get("BuildingType") and rules.city_has_building(city, a["BuildingType"]):  # unique ones count as the base
            add(f"{src} {mid}", a["YieldType"], float(a["Amount"]))
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_PER_POPULATION", name):
        add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * city["population"])
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_PER_DISTRICT", name):
        add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * specialty_district_count(rules, city))
    for effect in ("EFFECT_ADJUST_DISTRICT_YIELD_CHANGE", "EFFECT_ADJUST_DISTRICT_BASE_YIELD_CHANGE"):
        for d, a, mid, src in engine.district_effects(effect, name):
            add(f"{src} {mid}", a["YieldType"], float(a["Amount"]))
    # % on one building's yield (Poland... "double culture" monument / shrine / temple)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_BUILDING_YIELD_MODIFIER", name):
        b = next((x for x in city["buildings"] if rules.building_is(x, a["BuildingType"])), None)
        if b is not None:
            base = rules.building_yields[b].get(a["YieldType"], 0)
            add(f"{src} {mid}", a["YieldType"], base * float(a["Amount"]) / 100)
    # % on the yields of a district's buildings (Free Market, Grand Opera, Rationalism, Simultaneum, Seondeok)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_BUILDING_YIELD_MODIFIERS_FOR_DISTRICT", name):
        base = sum(rules.building_yields[b].get(a["YieldType"], 0) for b in city["buildings"]
                   if rules.building_in_district(b, a["DistrictType"]))
        if base:
            add(f"{src} {mid}", a["YieldType"], base * float(a["Amount"]) / 100)
    # Vietnam: each building in a district standing on a feature
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_BUILDING_FEATURE_YIELD_CHANGE", name):
        n = 0
        for d in city["districts"]:
            if d["complete"] and d.get("plot", {}).get("feature") == a["FeatureType"]:
                n += sum(1 for b in city["buildings"] if rules.building_in_district(b, d["type"]))
        if n:
            add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * n)
    # per world wonder in the city (Divine Inspiration, BBG wonder food)
    wonders = sum(1 for b in city["buildings"] if rules.is_wonder(b))
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_WONDER_YIELD_CHANGE", name) + engine.player_effects("EFFECT_ADJUST_WONDER_YIELD_CHANGE"):
        if wonders:
            add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * wonders)
    owned = _city_plots(city)
    # per tile of a terrain (Navigation School: +0.5 science per coast tile)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_PER_TERRAIN_TYPE", name):
        n = sum(1 for t, *_ in owned if t == a["TerrainType"])
        add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * n)
    # Johannesburg: per class of improved resource (bonus, luxury, strategic) in the city
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_YIELD_BY_NUMBER_OF_RESOURCES", name):
        classes = {rules.resource_class(res) for t, res, imp, xy in owned
                   if res and imp and rules.resource_revealed(res, engine.techs)}
        add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * len(classes - {None}))
    # Sweden's Open-Air Museum: per terrain class among the cities with one (taken as this city's plots; unverified)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_PER_TERRAIN_CLASS_CITIES_IMPROVEMENT", name):
        if any(imp == a["ImprovementType"] for t, r_, imp, xy in owned):
            classes = {rules.terrain_class(t) for t, *_ in owned}
            add(f"{src} {mid}", a["YieldType"], float(a["Amount"]) * len(classes - {None}))
    # Reyna's Tax Collector: gold per citizen
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_GOLD_FROM_CITIZENS", name):
        add(f"{src} {mid}", "YIELD_GOLD", float(a["Amount"]) * city["population"])
    return {k: dict(v) for k, v in out.items()}


def happiness_type(rules: Rules, score):
    """The Happinesses row (HAPPINESS_HAPPY, ...) for an amenity score."""
    for h, lo, hi in rules.q("select HappinessType, MinimumAmenityScore, MaximumAmenityScore from Happinesses "
                             "where HappinessType <> 'HAPPINESS_UNHAPPY'"):
        if (lo is None or score >= lo) and (hi is None or score <= hi):
            return h
    return None


def governor_titles_in_city(engine, city):
    """Promotions (titles, the first included) of the governor established in the city."""
    return sum(len(g.get("promotions") or ()) for g in engine.player.get("governors", ())
               if g.get("established") and governor_here(g, city, engine.player))


def faith_conversions(engine, city, y):
    """Yields granted as a % of the city's faith (Menelik on hills / resources, Moksha's Patron Saint)."""
    out = defaultdict(float)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_MODIFIER_FROM_FAITH", city["name"]):
        out[a["YieldType"]] += y.get("YIELD_FAITH", 0) * float(a["Amount"]) / 100
    return out


def city_yield_percents(rules: Rules, engine, city, amenity_score=None):
    """{yield: %} on the city's yields from modifiers: EFFECT_ADJUST_CITY_YIELD_MODIFIER, and
    EFFECT_ADJUST_CITY_HAPPINESS_YIELD for the city's happiness level (needs the amenity score)."""
    out = defaultdict(float)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_MODIFIER", city["name"]):
        out[a["YieldType"]] += float(a["Amount"])
    # Saladin: the worship building of his religion adds a % to science, faith and culture
    if any(rules.q("select 1 from Buildings where BuildingType=? and EnabledByReligion=1", b) for b in city["buildings"]):
        for a, mid, src in engine.player_effects("EFFECT_ADD_RELIGIOUS_BUILDING_MULTIPLIER"):
            out[a["YieldType"]] += float(a["Multiplier"])
    titles = governor_titles_in_city(engine, city)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_MODIFIER_PER_GOVERNOR_TITLE", city["name"]):
        out[a["YieldType"]] += float(a["Amount"]) * titles
    if amenity_score is not None:
        h = happiness_type(rules, amenity_score)
        for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_HAPPINESS_YIELD", city["name"]):
            if a.get("HappinessType") == h:
                out[a["YieldType"]] += float(a["Amount"])
    return out


def _adjacent_match(rules: Rules, row, n):
    """Does neighbour plot n satisfy one Adjacency_YieldChanges row? (row is a dict of the table's columns)"""
    if row["OtherDistrictAdjacent"]:  # wonders and unfinished districts don't count
        return n.get("districtType") not in (None, "DISTRICT_WONDER") and n.get("districtComplete") is not False
    if row["AdjacentDistrict"]:
        return n.get("districtType") == row["AdjacentDistrict"]
    if row["AdjacentSeaResource"]:
        return n["resource"] is not None and n["terrain"] in ("TERRAIN_COAST", "TERRAIN_OCEAN")
    if row["AdjacentWonder"]:
        return n.get("districtType") == "DISTRICT_WONDER" and n.get("wonderComplete") is not False
    if row["AdjacentTerrain"]:
        return n["terrain"] == row["AdjacentTerrain"]
    if row["AdjacentFeature"]:
        return n["feature"] == row["AdjacentFeature"]
    if row["AdjacentImprovement"]:
        return n["improvement"] == row["AdjacentImprovement"]
    if row["AdjacentResource"] or row["AdjacentResourceClass"] not in (None, "NO_RESOURCECLASS"):
        # BBG's strategic-resource adjacency gives only the class (AdjacentResource is 0)
        return n["resource"] is not None and (row["AdjacentResourceClass"] in (None, "NO_RESOURCECLASS")
                                              or rules.resource_class(n["resource"]) == row["AdjacentResourceClass"])
    if row["AdjacentNaturalWonder"]:
        return n["feature"] is not None and rules.is_natural_wonder(n["feature"])
    return False


def excluded_adjacencies(rules: Rules, player):
    """Adjacency rows (Adjacency_YieldChanges.ID) the player's traits switch off (ExcludedAdjacencies; BBG lists every
    other civilization's traits against a unique row such as Arabia's campus-holy site bonus)."""
    if not player:
        return frozenset()
    out = set()
    for t in rules.traits_of(player.get("civilization"), player.get("leader")):
        out |= {i for (i,) in rules.q("select YieldChangeId from ExcludedAdjacencies where TraitType=?", t)}
    return frozenset(out)


def district_adjacency(rules: Rules, district, techs, civics, plots_by_xy=None, extra=(), excluded=frozenset()):
    """Adjacency yields of one completed district from its 6 neighbours (and river, for AdjacentRiver).
    plots_by_xy: known plots (with district types and wonder state) to use as neighbours; extra: adjacency
    rows added by modifiers (EFFECT_FEATURE_ADJACENCY etc.), in Adjacency_YieldChanges form."""
    y = defaultdict(float)
    if not district["complete"] or district["type"] == "DISTRICT_CITY_CENTER":
        return y
    neighbours = district["neighbours"]
    if plots_by_xy:
        neighbours = [plots_by_xy.get((n["x"], n["y"]), n) for n in district["neighbours"]]
    for row in list(rules.district_adjacencies(district["type"])) + list(extra):
        if row.get("ID") in excluded:
            continue
        if row["PrereqTech"] and row["PrereqTech"] not in techs or row["PrereqCivic"] and row["PrereqCivic"] not in civics:
            continue
        if row["ObsoleteTech"] and row["ObsoleteTech"] in techs or row["ObsoleteCivic"] and row["ObsoleteCivic"] in civics:
            continue
        if row["AdjacentRiver"]:
            count = 1 if district["plot"]["river"] else 0
        elif row["Self"]:
            continue  # not modelled yet
        else:
            count = sum(1 for n in neighbours if _adjacent_match(rules, row, n))
        y[row["YieldType"]] += (count // max(row["TilesRequired"], 1)) * row["YieldChange"]
    return y


def trade_route_yields(rules: Rules, destination, domestic=True):
    """Yields the origin city receives from one trade route, from the destination's completed districts.
    Trading-post gold uses the player's per-post bonus (Rome's trait); every own city has a post for Rome."""
    col = "YieldChangeAsDomesticDestination" if domestic else "YieldChangeAsInternationalDestination"
    y = defaultdict(float)
    for d in destination["districts"]:
        if d["complete"]:
            for k, v in rules.q(f"select YieldType, {col} from District_TradeRouteYields where DistrictType=?", d["type"]):
                y[k] += v
    return y


def is_city_state(rules: Rules, civilization):
    row = rules.q("select StartingCivilizationLevelType from Civilizations where CivilizationType=?", civilization)
    return bool(row) and row[0][0] == "CIVILIZATION_LEVEL_CITY_STATE"


def _city_plots(city):
    """[(terrain, resource, improvement, xy)] of the plots a city owns (ours: worked + owned; exported route
    endpoints: "plots")."""
    if "plots" in city:
        return [(t, r, i, None) for t, r, i in city["plots"]]
    seen = {}
    for p in list(city.get("worked", ())) + list(city.get("owned", ())):
        seen[(p["x"], p["y"])] = (p["terrain"], p.get("resource"), p.get("improvement"), (p["x"], p["y"]))
    return list(seen.values())


def _resource_count(rules, plots, cls, techs, improved=False, center=None, radius=None):
    n = 0
    for t, res, imp, xy in plots:
        if not res or rules.resource_class(res) != cls or not rules.resource_revealed(res, techs):
            continue
        if improved and not imp:
            continue
        if radius is not None and xy is not None and center is not None and hex_distance(xy, center) > radius:
            continue
        n += 1
    return n


# effect -> (whose resources, resource class, domestic?)
_RESOURCE_ROUTE_EFFECTS = {
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_BONUS_RESOURCE_FOR_DOMESTIC": ("LOCAL", "RESOURCECLASS_BONUS", True),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_BONUS_RESOURCE_FOR_INTERNATIONAL": ("LOCAL", "RESOURCECLASS_BONUS", False),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_LUXURY_RESOURCE_FOR_DOMESTIC": ("LOCAL", "RESOURCECLASS_LUXURY", True),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_LUXURY_RESOURCE_FOR_INTERNATIONAL": ("LOCAL", "RESOURCECLASS_LUXURY", False),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_STRATEGIC_RESOURCE_FOR_DOMESTIC": ("LOCAL", "RESOURCECLASS_STRATEGIC", True),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_LOCAL_STRATEGIC_RESOURCE_FOR_INTERNATIONAL": ("LOCAL", "RESOURCECLASS_STRATEGIC", False),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_BONUS_RESOURCE_FOR_DOMESTIC": ("DESTINATION", "RESOURCECLASS_BONUS", True),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_BONUS_RESOURCE_FOR_INTERNATIONAL": ("DESTINATION", "RESOURCECLASS_BONUS", False),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_LUXURY_RESOURCE_FOR_DOMESTIC": ("DESTINATION", "RESOURCECLASS_LUXURY", True),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_LUXURY_RESOURCE_FOR_INTERNATIONAL": ("DESTINATION", "RESOURCECLASS_LUXURY", False),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_STRATEGIC_RESOURCE_FOR_DOMESTIC": ("DESTINATION", "RESOURCECLASS_STRATEGIC", True),
    "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_PER_DESTINATION_STRATEGIC_RESOURCE_FOR_INTERNATIONAL": ("DESTINATION", "RESOURCECLASS_STRATEGIC", False),
}


def route_yields(rules: Rules, engine, route, origin_city):
    """Yields the origin city gets from one of its trade routes.

    route: {"origin": {...}, "destination": {name, playerId, civilization, districts: [completed types], plots,
    techCount, civicCount}} as exported. Base yields come from the destination's districts
    (District_TradeRouteYields, domestic or international column); modifiers add per-route amounts (policies,
    traits, city-state bonuses, governors), then percentage modifiers scale the route's total.

    Approximations: the path length is the distance between the two city centres (the engine counts the tiles the
    trader actually travels); "trading posts in foreign cities" counts only the destination. Trading posts are
    taken to exist in every destination (as they do from the start for Rome, EFFECT_GRANT_CITY_TRADING_POST, and
    after the first route for Mongolia, EFFECT_ADJUST_PLAYER_IMMEDIATE_TRADING_POST; for others after the first
    completed route there)."""
    dest = route["destination"]
    domestic = dest["playerId"] == route["origin"]["playerId"]
    col = "YieldChangeAsDomesticDestination" if domestic else "YieldChangeAsInternationalDestination"
    y = defaultdict(float)
    for t in dest["districts"]:
        for k, v in rules.q(f"select YieldType, {col} from District_TradeRouteYields where DistrictType=?", t):
            y[k] += v
    for d in origin_city["districts"]:
        if d["complete"]:
            for k, v in rules.q("select YieldType, YieldChangeAsOrigin from District_TradeRouteYields where DistrictType=?", d["type"]):
                y[k] += v
    specialty = sum(1 for t in dest["districts"] if rules.district_requires_population(t))
    origin_specialty = sum(1 for d in origin_city["districts"] if d["complete"] and rules.district_requires_population(d["type"]))
    cs = is_city_state(rules, dest.get("civilization"))
    techs = engine.techs
    o_center = (origin_city["center"]["x"], origin_city["center"]["y"])
    o_plots, d_plots = _city_plots(origin_city), _city_plots(dest)

    def effects(effect):
        return engine.player_effects(effect) + engine.city_effects(effect, origin_city["name"])

    def add(effect, times=1, origin_only=False):
        for a, mid, src in effects(effect):
            if origin_only and a.get("Destination") in ("1", "true") and a.get("Origin") not in ("1", "true"):
                continue  # the destination's share (it goes to the other city)
            n = times(a) if callable(times) else times
            for k, v in _split_args(a, "YieldType", "Amount"):
                y[k] += float(v) * n

    add("EFFECT_ADJUST_TRADE_ROUTE_YIELD")
    add("EFFECT_ADJUST_TRADE_ROUTE_YIELD_FOR_DOMESTIC" if domestic else "EFFECT_ADJUST_TRADE_ROUTE_YIELD_FOR_INTERNATIONAL")
    add("EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_FOR_DOMESTIC" if domestic else "EFFECT_ADJUST_CITY_TRADE_ROUTE_YIELD_FOR_INTERNATIONAL")
    # resources at the origin (Great Zimbabwe: bonus resources within 3 tiles of the city; Ethiopia; Ahiram) and
    # at the destination (Amsterdam, Antioch, Market Economy, Rockefeller)
    for effect, (where, cls, dom) in _RESOURCE_ROUTE_EFFECTS.items():
        if dom == domestic:
            if where == "LOCAL":
                add(effect, lambda a, cls=cls: _resource_count(rules, o_plots, cls, techs, center=o_center, radius=3))
            else:
                add(effect, lambda a, cls=cls: _resource_count(rules, d_plots, cls, techs))
    # mountains (or other terrain) in the origin city: Pachacuti
    add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_TERRAIN_FOR_DOMESTIC" if domestic else
        "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_TERRAIN_FOR_INTERNATIONAL",
        lambda a: sum(1 for t, *_ in o_plots if t == a["TerrainType"]), origin_only=True)
    # improvements in the destination (Cree camps / pastures, Portuguese feitorias), at the origin (Samarkand domes)
    add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_IMPROVEMENT_IN_TARGET_CITY",
        lambda a: sum(1 for _, _, i, _ in d_plots if i == a["ImprovementType"]), origin_only=True)
    if not domestic:
        add("EFFECT_ADJUST_PLAYER_INTERNATIONAL_TRADE_ROUTE_YIELD_PER_IMPROVEMENT_IN_ORIGIN_CITY",
            lambda a: sum(1 for _, _, i, _ in o_plots if i == a["ImprovementType"]))
        add("EFFECT_ADJUST_TRADE_ROUTE_YIELD_PER_SPECIALTY_DISTRICT_FOR_INTERNATIONAL", specialty)
        # Jakarta: per trading post in the foreign cities the route goes to or passes through (confirmed by the user);
        # route["foreignCitiesOnPath"] when the path is known, else the destination alone
        add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_POST_IN_FOREIGN_CITY",
            lambda a: route.get("foreignCitiesOnPath") or 1)
    # length of the route
    add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_PATH_TILE", lambda a: hex_distance(o_center, (dest["x"], dest["y"])))
    # followers of our religion in the origin city: Chinguetti
    add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_FOLLOWER", lambda a: _own_followers(engine, origin_city))
    if domestic:
        add("EFFECT_ADJUST_TRADE_ROUTE_YIELD_PER_SPECIALTY_DISTRICT_FOR_DOMESTIC", specialty)
        # a trading post in the (own) destination: Rome's All Roads Lead to Rome
        add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_POST_IN_OWN_CITY")
    if cs:
        add("EFFECT_ADJUST_CITY_STATE_TRADE_ROUTE_FLAT_YIELD")
        # Kumasi, Elizabeth: per specialty district in the origin city
        add("EFFECT_ADJUST_CITY_STATE_TRADE_ROUTE_DISTRICT_YIELD", origin_specialty)
        if any(e.get("suzerain") and e.get("civilization") == dest.get("civilization") for e in engine.player.get("envoys", ())):
            add("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_ORIGIN_YIELD_FOR_SUZERAIN_ROUTE")
        # Not added: the city-state's own "send trade route" bonus (scientific +1 science, industrial +1
        # production, see city_state_route_bonus) - routes to Johannesburg and Geneva in game_auto2 don't get it.
    elif not domestic:
        # Grand Embassy: +1 science per 3 techs, +1 culture per 3 civics the destination's owner is ahead
        for a, mid, src in effects("EFFECT_ADJUST_PLAYER_PROGRESS_DIFF_TRADE_BONUS"):
            per = int(a["TechCivicsPerYield"])
            y["YIELD_SCIENCE"] += max(0, dest.get("techCount", 0) - len(engine.techs)) // per
            y["YIELD_CULTURE"] += max(0, dest.get("civicCount", 0) - len(engine.civics)) // per
    # what the destination city hands to routes arriving there (governors such as Pingala or Moksha, banks); only
    # known when the destination is ours
    if dest["name"] in engine.by_name:
        for a, mid, src in engine.city_effects("EFFECT_ADJUST_TRADE_ROUTE_YIELD_TO_OTHERS", dest["name"]):
            if _route_kind_ok(a, domestic):
                y[a["YieldType"]] += float(a["Amount"])
    # percentages on the route's yields (Letters of Marque, Tokugawa, Portugal)
    pct = defaultdict(float)
    for a, mid, src in effects("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_MODIFIER"):
        if a.get("Origin") in ("1", "true") or a.get("Destination") not in ("1", "true"):
            for k, v in _split_args(a, "YieldType", "Amount"):
                pct[k] += float(v)
    if not domestic:
        for a, mid, src in effects("EFFECT_ADJUST_PLAYER_INTERNATIONAL_TRADE_ROUTE_YIELD_MODIFIER"):
            for k, v in _split_args(a, "YieldType", "Amount"):
                pct[k] += float(v)
    for k, v in pct.items():
        if k in y:
            y[k] *= 1 + v / 100
    return y


def _split_args(a, key, amount):
    """Arguments may list several yields: "YIELD_A, YIELD_B" with "1, 2"."""
    ks = [k.strip() for k in str(a[key]).split(",")]
    vs = [v.strip() for v in str(a[amount]).split(",")]
    if len(vs) == 1:
        vs = vs * len(ks)
    return list(zip(ks, vs))


def _own_followers(engine, city):
    """Followers in the city of our founded religion, else of the city's majority religion."""
    rel = city.get("religion") or {}
    ours = [r for r, x in engine.religions.items() if x.get("founder") == engine.player.get("playerId")]
    want = ours[0] if ours else rel.get("majority")
    return (rel.get("followers") or {}).get(want, 0) if want else 0


def city_state_route_bonus(rules: Rules, leader):
    """{yield: amount} a city-state's own trait gives to trade routes arriving in its cities."""
    out = defaultdict(float)
    if not leader:
        return out
    base = rules.q("select InheritFrom from Leaders where LeaderType=?", leader)
    leader = base[0][0] if base and base[0][0] else leader
    for mid, mtype, subj in rules.q(
            "select m.ModifierId, m.ModifierType, m.SubjectRequirementSetId from LeaderTraits lt "
            "join TraitModifiers tm on tm.TraitType=lt.TraitType join Modifiers m on m.ModifierId=tm.ModifierId "
            "where lt.LeaderType=? and m.ModifierType='MODIFIER_PLAYER_CITIES_ADJUST_TRADE_ROUTE_YIELD_TO_OTHERS'", leader):
        if subj:
            continue
        a = dict(rules.q("select Name, Value from ModifierArguments where ModifierId=?", mid))
        out[a["YieldType"]] += float(a["Amount"])
    return out


def _route_kind_ok(args, domestic):
    d = args.get("Domestic")
    return d is None or (d in ("1", "true", "True")) == domestic


def incoming_route_yields(rules: Rules, engine, route, dest_city):
    """Yields a city of ours gets from a trade route arriving there (EFFECT_ADJUST_TRADE_ROUTE_YIELD_FROM_OTHERS)."""
    domestic = route["destination"]["playerId"] == route["origin"]["playerId"]
    y = defaultdict(float)
    for a, mid, src in engine.city_effects("EFFECT_ADJUST_TRADE_ROUTE_YIELD_FROM_OTHERS", dest_city["name"]):
        if _route_kind_ok(a, domestic):
            y[a["YieldType"]] += float(a["Amount"])
    if not domestic:
        # Reyna: per foreign route into (or passing through: not modelled, no paths) the city
        for a, mid, src in engine.city_effects("EFFECT_ADJUST_CITY_YIELD_FROM_FOREIGN_TRADE_ROUTES_PASSING_THROUGH", dest_city["name"]):
            y[a["YieldType"]] += float(a["Amount"])
    if route["origin"]["playerId"] == engine.player.get("playerId"):
        # our own route ending here: the destination's share of our route bonuses (Cree gold per camp / pasture)
        dest = lambda a: a.get("Destination") in ("1", "true") and a.get("Origin") not in ("1", "true")
        plots = _city_plots(dest_city)
        for a, mid, src in engine.player_effects("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_PER_IMPROVEMENT_IN_TARGET_CITY"):
            if dest(a):
                y[a["YieldType"]] += float(a["Amount"]) * sum(1 for _, _, i, _ in plots if i == a["ImprovementType"])
        for a, mid, src in engine.player_effects("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_YIELD_MODIFIER"):
            if dest(a) and a["YieldType"] in y:
                y[a["YieldType"]] *= 1 + float(a["Amount"]) / 100
    return y


def _as_route(rules, origin_city, route, cities_by_name):
    """Accept (origin name, destination name) pairs from older callers."""
    if isinstance(route, dict):
        return route
    origin, dest = route
    d = (cities_by_name or {}).get(dest)
    if origin != origin_city["name"] or d is None:
        return None
    return {"origin": {"name": origin, "playerId": 0},
            "destination": {"name": dest, "playerId": 0, "civilization": None, "x": d["center"]["x"], "y": d["center"]["y"],
                            "plots": [x[:3] for x in _city_plots(d)],
                            "districts": [x["type"] for x in d["districts"] if x["complete"]]}}


def specialist_yields(rules: Rules, city, xy):
    """Yields of one specialist in the district at xy: the district's citizen yield plus each of its buildings'."""
    d = next((d for d in city["districts"] if (d["x"], d["y"]) == xy), None)
    y = defaultdict(float)
    if d is None:
        return y
    types = {d["type"]} | {b for (b,) in rules.q(
        "select ReplacesDistrictType from DistrictReplaces where CivUniqueDistrictType=?", d["type"])}
    for k, v in rules.q("select YieldType, YieldChange from District_CitizenYieldChanges where DistrictType=?", d["type"]):
        y[k] += v
    for b in city["buildings"]:
        pre = rules.q("select PrereqDistrict from Buildings where BuildingType=?", b)
        if pre and pre[0][0] in types:
            for k, v in rules.q("select YieldType, YieldChange from Building_CitizenYieldChanges where BuildingType=?", b):
                y[k] += v
    return y


def city_yields(rules: Rules, city, player, bbcc_mode="capital", routes=(), cities_by_name=None):
    """routes: [(origin, destination)] city names of active domestic trade routes."""
    """Returns (total, breakdown) where breakdown maps a source label to its yields."""
    techs, civics = set(player["techs"]), set(player["civics"])
    parts = {}
    cx, cy = city["center"]["x"], city["center"]["y"]
    bbcc = bbcc_mode == "all" or (bbcc_mode == "capital" and city["capital"])
    plot_mods = city_plot_modifiers(rules, player, city, (cities_by_name or {city["name"]: city}).values())
    plots_by_xy = plots_by_xy_of(list((cities_by_name or {}).values()) or [city])
    from .modifiers import engine_for
    engine = engine_for(rules, player, list((cities_by_name or {}).values()) or [city])
    amods = appeal_modifiers(engine, city["name"])
    for w in city["worked"]:
        is_center = (w["x"], w["y"]) == (cx, cy)
        w = dict(w, districtType="DISTRICT_CITY_CENTER" if is_center else None)
        label = "center" if is_center else f"tile({w['x']},{w['y']}) {w['terrain'][8:]}" + \
            "".join(f" {s}" for s in (w["feature"], w["resource"], w["improvement"]) if s)
        if w.get("district") and not is_center:
            # citizens in a district are specialists
            spec = specialist_yields(rules, city, (w["x"], w["y"]))
            n = w.get("workers") or 1
            parts[f"tile({w['x']},{w['y']}) specialists x{n}"] = {k: v * n for k, v in spec.items()}
            continue
        parts[label] = tile_yields(rules, w, techs, civics, city_center=is_center, bbcc=is_center and bbcc, appeal_mods=amods,
                                   plot_modifiers=plot_mods, plots_by_xy=plots_by_xy, city=city)
        extra = terrain_from_adjacent_improvements(engine, city["name"], w, plots_by_xy)
        if extra:
            parts[label + " (adjacent improvements)"] = extra
        if not is_center:
            extra = appeal_district_yields(rules, city, w, plots_by_xy, amods)
            if extra:
                parts[label + " (appeal next to a preserve)"] = extra
    for b in city["buildings"]:
        parts[b] = dict(rules.building_yields[b])
    # great works on display (GreatWork_YieldChanges)
    gw = defaultdict(float)
    for building, work in city.get("greatWorks") or ():
        for k, v in rules.q("select YieldType, YieldChange from GreatWork_YieldChanges where GreatWorkType=?", work):
            gw[k] += v
    if gw:
        parts["great works"] = dict(gw)
    district_pct = defaultdict(lambda: defaultdict(float))  # (x, y) -> yield -> % on the district's adjacency
    for d, a, mid, src in engine.district_effects("EFFECT_ADJUST_DISTRICT_YIELD_MODIFIER", city["name"]):
        district_pct[(d["x"], d["y"])][a["YieldType"]] += float(a["Amount"])
    for d in city["districts"]:
        adj = district_adjacency(rules, d, techs, civics, engine.plots, modifier_adjacency_rows(engine, city["name"], d["type"]),
                                 excluded_adjacencies(rules, player))
        if any(adj.values()):
            parts[f"{d['type']} adjacency"] = dict(adj)
            pct = district_pct.get((d["x"], d["y"]), {})
            # rounded down (Natural Philosophy +50% on a campus with 1 adjacency gives nothing; Mongolia, rome1)
            bonus = {k: math.floor(v * pct[k] / 100) for k, v in adj.items() if pct.get(k) and math.floor(v * pct[k] / 100)}
            if bonus:
                parts[f"{d['type']} adjacency %"] = bonus
            # buildings that copy the district's adjacency as another yield (Madrasa: campus science as faith)
            for b in city["buildings"]:
                if rules.building_in_district(b, d["type"]):
                    for old, new in rules.q("select OldYieldType, NewYieldType from Building_YieldDistrictCopies where BuildingType=?", b):
                        if adj.get(old):
                            parts[f"{b} copies {d['type']} adjacency"] = {new: adj[old]}
            # the adjacency of one yield granted again as another (Work Ethic: holy site faith as production)
            for dd, a, mid, src in engine.district_effects("EFFECT_ADJUST_DISTRICT_YIELD_BASED_ON_ADJACENCY_BONUS", city["name"]):
                if dd is not None and (dd["x"], dd["y"]) == (d["x"], d["y"]) and adj.get(a["YieldTypeToMirror"]):
                    parts[f"{d['type']} {mid}"] = {a["YieldTypeToGrant"]: adj[a["YieldTypeToMirror"]]}
        if d["complete"]:
            # Australia: districts on charming / breathtaking tiles
            for a, mid, src in engine.city_effects("EFFECT_ADJUST_DISTRICT_YIELD_BASED_ON_APPEAL", city["name"]):
                if rules.district_is(d["type"], a["DistrictType"]) and \
                        appeal(rules, d["plot"], engine.plots, amods) >= int(a["RequiredAppeal"]):
                    parts[f"{d['type']} {mid}"] = {a["YieldType"]: float(a["YieldChange"])}
    parts.update(modifier_city_yields(rules, engine, city))
    for i, route in enumerate(routes):
        route = _as_route(rules, city, route, cities_by_name)
        if route is None or route["origin"]["name"] != city["name"]:
            continue
        parts[f"trade route {i} -> {route['destination']['name']}"] = dict(route_yields(rules, engine, route, city))
    for i, route in enumerate(routes):
        if isinstance(route, dict) and route["destination"]["name"] == city["name"]:
            y = incoming_route_yields(rules, engine, route, city)
            if y:
                parts[f"trade route {i} <- {route['origin']['name']}"] = dict(y)
    pop = city["population"]
    # The game keeps yields in 1/256 steps: 0.3 culture per citizen is 76/256 (records show 1.296875 = 1 + 76/256).
    fixed = lambda v: math.floor(v * 256) / 256
    parts["population"] = {"YIELD_SCIENCE": fixed(pop * rules.param("SCIENCE_PERCENTAGE_YIELD_PER_POP") / 100),
                           "YIELD_CULTURE": fixed(pop * rules.param("CULTURE_PERCENTAGE_YIELD_PER_POP") / 100)}
    total = defaultdict(float)
    for y in parts.values():
        for k, v in y.items():
            total[k] += v
    return {k: total[k] for k in YIELDS}, parts


# Trading posts are taken to exist in every route destination (route_yields), which these make true from the start:
_TRADING_POST_EFFECTS = ("EFFECT_GRANT_CITY_TRADING_POST", "EFFECT_ADJUST_PLAYER_IMMEDIATE_TRADING_POST")


def player_yields(rules: Rules, engine, totals=None):
    """Yields the player gets outside any city: founder beliefs counted over the religion's followers and cities
    world-wide (Tithe, Church Property, Pilgrimage, World Church, Lay Ministry, Stewardship, Sacred Places,
    Cross-Cultural Dialogue), Arabia / Kailash per foreign city following our religion, Gandhi's faith per
    religion-founding civilization, Sejong's culture from science. `totals` are the cities' summed yields this turn
    (for the effects based on the current rate)."""
    y = defaultdict(float)
    me = engine.player.get("playerId")
    for a, mid, src in engine.player_effects("EFFECT_ADD_RELIGIOUS_BELIEF_YIELD"):
        rel = engine.religions.get(str(src).split("@", 1)[1]) if "@" in str(src) else None
        if rel is None:
            rel = next((r for r in engine.religions.values() if r.get("founder") == me), None)
        if rel is None:
            continue
        y[a["YieldType"]] += float(a["Amount"]) * _belief_count(rel, a, me)
    ours = next((r for r in engine.religions.values() if r.get("founder") == me), None)
    for a, mid, src in engine.player_effects("EFFECT_ADD_PLAYER_BELIEF_YIELD"):
        if ours is not None:
            y[a["YieldType"]] += float(a["Amount"]) * _belief_count(ours, a, me)
    founders = {r.get("founder") for r in engine.religions.values()}
    for a, mid, src in engine.player_effects("EFFECT_ADJUST_PLAYER_FAITH_PEACEFUL_FOUNDERS"):
        # civilizations met that founded a religion (wars are outside the model; meeting is assumed)
        y["YIELD_FAITH"] += float(a["Amount"]) * len(founders)
    from .greatpeople import prophet_faith
    y["YIELD_FAITH"] += prophet_faith(rules, engine, engine.cities)
    if totals:
        for a, mid, src in engine.player_effects("EFFECT_GRANT_YIELD_BASED_ON_CURRENT_YIELD_RATE"):
            y[a["YieldToGrant"]] += totals.get(a["YieldToBaseOn"], 0) * float(a["Multiplier"]) / 100
    return y


def _belief_count(rel, a, me):
    """How many times a founder belief pays: cities / followers (per PerXItems, rounded down) / districts / cities
    with a world wonder, among the cities following the religion (majority), foreign ones where asked."""
    kind = a.get("BeliefYieldType")
    per = max(1, int(float(a.get("PerXItems") or 1)))
    cities = rel.get("cityList") or []
    following = [c for c in cities if c["majority"]]
    foreign = lambda cs: [c for c in cs if c["playerId"] != me]
    if kind == "BELIEF_YIELD_PER_CITY":
        n = len(following) if cities else rel.get("cities", 0)
    elif kind == "BELIEF_YIELD_PER_FOREIGN_CITY":
        n = len(foreign(following))
    elif kind == "BELIEF_YIELD_PER_FOLLOWER":
        n = (sum(c["followers"] for c in cities) if cities else rel.get("followers", 0)) // per
    elif kind == "BELIEF_YIELD_PER_FOREIGN_FOLLOWER":
        n = sum(c["followers"] for c in foreign(cities)) // per
    elif kind == "BELIEF_YIELD_PER_DISTRICT":
        n = sum(c["districts"].count(a["DistrictType"]) for c in following)
    elif kind == "BELIEF_YIELD_PER_CITY_WITH_WONDER":
        n = sum(1 for c in following if c.get("wonders"))
    else:
        return 0
    return n
