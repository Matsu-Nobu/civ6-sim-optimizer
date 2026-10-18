"""Where improvements and districts may go: the rules database's placement columns and tables, plus the modifiers
that open or close options (city-state unique improvements, farms on tundra / desert, Vietnam's feature-only
districts, Gaul's no districts next to the city center, Egypt's government plaza on floodplains, Qin's canal).

plot: a map plot dict (terrain, feature, resource, improvement, river); near(xy) -> neighbouring plot dicts with
"districtType" set where a district stands; lake(xy) -> is the water plot a lake.
"""
from .yields import neighbour_xy

WATER = ("TERRAIN_COAST", "TERRAIN_OCEAN")


def _is_mountain(t):
    return t.endswith("_MOUNTAIN")


def improvement_options(rules, engine, plot, near, techs, civics, city=None, lake=lambda xy: False):
    """Improvements a builder may put on `plot` now."""
    out = []
    terrain, feature, resource = plot["terrain"], plot["feature"], plot["resource"]
    allowed = {a["ImprovementType"] for a, m, s in engine.player_effects("EFFECT_ADJUST_PLAYER_VALID_IMPROVEMENT")}
    if city is not None:
        allowed |= {a["ImprovementType"] for a, m, s in engine.city_effects("EFFECT_ADJUST_CITY_ALLOWED_IMPROVEMENT", city["name"])}
    extra_terrain = set()
    if city is not None:
        extra_terrain = {(a["ImprovementType"], a["TerrainType"]) for a, m, s in
                         engine.city_effects("EFFECT_ADJUST_IMPROVEMENT_VALID_TERRAIN", city["name"])}
    neighbours = near((plot["x"], plot["y"]))
    for row in rules.q("select ImprovementType, PrereqTech, PrereqCivic, TraitType, SameAdjacentValid, RequiresRiver, "
                       "MinimumAppeal, Coast, ValidAdjacentTerrainAmount, RequiresAdjacentBonusOrLuxury, "
                       "RequiresAdjacentLuxury, NoAdjacentSpecialtyDistrict, AdjacentToLand, OnePerCity, Domain "
                       "from Improvements where Buildable=1"):
        (imp, pt, pc, trait, same_ok, river, min_appeal, coast, adj_terrain_n, adj_bonus_lux, adj_lux, no_adj_spec,
         adj_land, one_per_city, domain) = row
        if not rules.q("select 1 from Improvement_ValidBuildUnits where ImprovementType=? and UnitType='UNIT_BUILDER'", imp):
            continue  # built by other units (Toa, Nau, military engineers...)
        if (pt and pt not in techs) or (pc and pc not in civics):
            continue
        if trait and not rules.civ_has_trait(engine.player.get("civilization"), engine.player.get("leader"), trait) and imp not in allowed:
            continue
        if resource and rules.resource_revealed(resource, techs):
            row = rules.q("select MustRemoveFeature from Improvement_ValidResources where ImprovementType=? and ResourceType=?",
                          imp, resource)
            if not row:
                continue  # a resource tile takes only an improvement for that resource
            if feature and row[0][0]:
                continue  # the feature has to go first (harvest / chop)
        else:
            tr = rules.q("select PrereqTech, PrereqCivic from Improvement_ValidTerrains where ImprovementType=? and TerrainType=?",
                         imp, terrain)
            ok_t = any((not a or a in techs) and (not b or b in civics) for a, b in tr) or (imp, terrain) in extra_terrain
            if feature:
                fr = rules.q("select PrereqTech, PrereqCivic from Improvement_ValidFeatures where ImprovementType=? and FeatureType=?",
                             imp, feature)
                if not any((not a or a in techs) and (not b or b in civics) for a, b in fr):
                    continue
                ok_t = ok_t or bool(fr) and not rules.q("select 1 from Improvement_ValidTerrains where ImprovementType=?", imp)
            if not ok_t:
                continue
        if river and not plot.get("river"):
            continue
        if not same_ok and any(n.get("improvement") == imp for n in neighbours):
            continue
        if coast and not any(n["terrain"] in WATER and not lake((n["x"], n["y"])) for n in neighbours):
            continue
        if adj_land and not any(n["terrain"] not in WATER for n in neighbours):
            continue
        if adj_terrain_n:
            want = {t for (t,) in rules.q("select TerrainType from Improvement_ValidAdjacentTerrains where ImprovementType=?", imp)}
            if sum(1 for n in neighbours if n["terrain"] in want) < adj_terrain_n:
                continue
        if adj_bonus_lux and not any(n.get("resource") and rules.resource_class(n["resource"]) in
                                     ("RESOURCECLASS_BONUS", "RESOURCECLASS_LUXURY") for n in neighbours):
            continue
        if adj_lux and not any(n.get("resource") and rules.resource_class(n["resource"]) == "RESOURCECLASS_LUXURY" for n in neighbours):
            continue
        if no_adj_spec and any(n.get("districtType") and rules.district_requires_population(n["districtType"]) for n in neighbours):
            continue
        bad_f = {f for (f,) in rules.q("select FeatureType from Improvement_InvalidAdjacentFeatures where ImprovementType=?", imp)}
        if bad_f and any(n.get("feature") in bad_f for n in neighbours):
            continue
        adj_res = {r_ for (r_,) in rules.q("select ResourceType from Improvement_ValidAdjacentResources where ImprovementType=?", imp)}
        if adj_res and not any(n.get("resource") in adj_res for n in neighbours):
            continue
        if one_per_city and city is not None and any(p.get("improvement") == imp for p in city["owned"]):
            continue
        if min_appeal:
            from .yields import appeal, appeal_modifiers
            plots = {(n["x"], n["y"]): n for n in neighbours}
            if appeal(rules, plot, plots, appeal_modifiers(engine, city["name"]) if city else None) < min_appeal:
                continue
        out.append(imp)
    return out


def district_ok(rules, engine, dist, plot, near, city, taken, lake=lambda xy: False):
    """May district `dist` go on `plot` for `city`? taken: plots already holding a district or wonder."""
    xy = (plot["x"], plot["y"])
    if xy in taken or plot.get("improvement"):
        return False
    terrain, feature, resource = plot["terrain"], plot["feature"], plot["resource"]
    (coast, no_adj_city, aqueduct, one_per_city, adj_land, trait) = rules.q(
        "select Coast, NoAdjacentCity, Aqueduct, OnePerCity, AdjacentToLand, TraitType from Districts where DistrictType=?", dist)[0]
    neighbours = near(xy)
    center = (city["center"]["x"], city["center"]["y"])
    if resource and rules.resource_class(resource) in ("RESOURCECLASS_LUXURY", "RESOURCECLASS_STRATEGIC") \
            and rules.resource_revealed(resource, engine.techs):
        return False  # bonus resources are removed; luxuries and strategics block
    if feature and (rules.is_natural_wonder(feature) or rules.q("select 1 from Features where FeatureType=? and Impassable=1", feature)):
        return False
    if _is_mountain(terrain):
        return False
    valid_t = {t for (t,) in rules.q("select TerrainType from District_ValidTerrains where DistrictType=?", dist)}
    if coast:
        if terrain not in WATER or not any(n["terrain"] not in WATER for n in neighbours):
            return False
    elif terrain in WATER:
        return False
    if valid_t and terrain not in valid_t:
        return False
    required_f = {f for (f,) in rules.q("select FeatureType from District_RequiredFeatures where DistrictType=?", dist)}
    if required_f and feature not in required_f:
        return False
    if no_adj_city and center in [(n["x"], n["y"]) for n in neighbours]:
        return False
    if aqueduct:
        if center not in [(n["x"], n["y"]) for n in neighbours]:
            return False
        fresh = plot.get("river") or any(lake((n["x"], n["y"])) or _is_mountain(n["terrain"]) or
                                         (n.get("feature") and rules.q("select 1 from Features where FeatureType=? and AddsFreshWater=1", n["feature"]))
                                         for n in neighbours)
        if not fresh:
            return False
    specialty = rules.district_requires_population(dist)
    # Vietnam: land specialty districts only on these features
    need_f = {a["FeatureType"] for a, m, s in engine.player_effects("EFFECT_ADJUST_PLAYER_FEAUTE_REQUIRED_FOR_SPECIALTY_DISTRICTS")}
    if need_f and specialty and not coast and feature not in need_f:
        return False
    # Gaul: no specialty district next to the city center
    if specialty and engine.player_effects("EFFECT_ADJUST_PLAYER_SPECIALTY_DISTRICT_CANNOT_BE_BUILT_ADJACENT_TO_CITY") \
            and center in [(n["x"], n["y"]) for n in neighbours]:
        return False
    # a feature must be removable (woods, rainforest, marsh are cleared) or allow districts (floodplains,
    # volcanic soil: Features_XP2.ValidDistrictPlacement); oases and the like block. Egypt's plaza-on-floodplains
    # modifier is then already the default.
    if feature and not need_f:
        removable = rules.q("select Removable from Features where FeatureType=?", feature)
        valid = rules.q("select ValidDistrictPlacement from Features_XP2 where FeatureType=?", feature)
        if not (removable and removable[0][0]) and not (valid and valid[0][0]):
            return False
    return True


# read by the coverage check: handled above as the default rule
_DEFAULT_RULE_EFFECTS = ("EFFECT_ADJUST_VALID_FEATURES_DISTRICTS",)


def wonder_ok(rules, engine, wonder, plot, near, city, taken, lake=lambda xy: False):
    """May world wonder `wonder` go on `plot` for `city`? Building_ValidTerrains / Valid- / RequiredFeatures and the
    Buildings placement columns (adjacent district / resource / improvement / capital / mountain, river, coast, lake)."""
    xy = (plot["x"], plot["y"])
    if xy in taken or plot.get("improvement"):
        return False
    (adj_d, adj_res, adj_imp, river, coast, lake_only, no_lake, adj_cap, adj_mtn, adj_river, adj_land) = rules.q(
        "select AdjacentDistrict, AdjacentResource, AdjacentImprovement, RequiresRiver, Coast, MustBeLake, MustNotBeLake, "
        "AdjacentCapital, AdjacentToMountain, RequiresAdjacentRiver, MustBeAdjacentLand from Buildings where BuildingType=?",
        wonder)[0]
    terrain, feature, resource = plot["terrain"], plot["feature"], plot["resource"]
    neighbours = near(xy)
    water = terrain in WATER
    if coast or lake_only:
        if not water:
            return False
        if lake_only and not lake(xy):
            return False
        if no_lake and lake(xy):
            return False
        if adj_land and not any(n["terrain"] not in WATER for n in neighbours):
            return False
    elif water:
        return False
    if _is_mountain(terrain):
        return False
    valid_t = {t for (t,) in rules.q("select TerrainType from Building_ValidTerrains where BuildingType=?", wonder)}
    if valid_t and terrain not in valid_t:
        return False
    keep_f = {f for (f,) in rules.q("select FeatureType from Building_ValidFeatures where BuildingType=?", wonder)}
    need_f = {f for (f,) in rules.q("select FeatureType from Building_RequiredFeatures where BuildingType=?", wonder)}
    if need_f and feature not in need_f:
        return False
    if feature and feature not in keep_f | need_f:
        removable = rules.q("select Removable from Features where FeatureType=?", feature)
        valid = rules.q("select ValidWonderPlacement from Features_XP2 where FeatureType=?", feature)
        if not (removable and removable[0][0]) and not (valid and valid[0][0]):
            return False
    if resource and rules.resource_class(resource) in ("RESOURCECLASS_LUXURY", "RESOURCECLASS_STRATEGIC") \
            and rules.resource_revealed(resource, engine.techs):
        return False
    if river and not plot.get("river"):
        return False
    if adj_river and not any(n.get("river") for n in neighbours) and not plot.get("river"):
        return False
    if adj_d and not any(n.get("districtType") and (rules.district_is(n["districtType"], adj_d)) for n in neighbours):
        return False
    if adj_res and not any(n.get("resource") == adj_res for n in neighbours):
        return False
    if adj_imp and not any(n.get("improvement") == adj_imp for n in neighbours):
        return False
    if adj_cap and not any((n["x"], n["y"]) == (city["center"]["x"], city["center"]["y"]) and city.get("capital") for n in neighbours):
        return False
    if adj_mtn and not any(_is_mountain(n["terrain"]) for n in neighbours):
        return False
    return True
