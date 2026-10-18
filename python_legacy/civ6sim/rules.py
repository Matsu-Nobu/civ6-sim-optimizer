"""Game rules read from the game's own post-mod database (Cache/DebugGameplay.sqlite)."""
import sqlite3
from collections import defaultdict
from functools import cached_property

YIELDS = ("YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH")


def _table(rows):
    out = defaultdict(lambda: defaultdict(float))
    for key, yield_type, amount in rows:
        out[key][yield_type] += amount
    return out


class Rules:
    def __init__(self, db_path):
        self.db = sqlite3.connect(db_path)
        self._cache = {}

    def q(self, sql, *args):
        """Query the (read-only) rules database; results are cached since the simulator repeats lookups."""
        key = (sql, args)
        if key not in self._cache:
            self._cache[key] = self.db.execute(sql, args).fetchall()
        return self._cache[key]

    @cached_property
    def params(self):
        return {k: v for k, v in self.q("select Name, Value from GlobalParameters")}

    def param(self, name, cast=float):
        return cast(self.params[name])

    @cached_property
    def terrain_yields(self):
        return _table(self.q("select TerrainType, YieldType, YieldChange from Terrain_YieldChanges"))

    @cached_property
    def feature_yields(self):
        return _table(self.q("select FeatureType, YieldType, YieldChange from Feature_YieldChanges"))

    @cached_property
    def resource_yields(self):
        return _table(self.q("select ResourceType, YieldType, YieldChange from Resource_YieldChanges"))

    @cached_property
    def improvement_yields(self):
        return _table(self.q("select ImprovementType, YieldType, YieldChange from Improvement_YieldChanges"))

    @cached_property
    def improvement_bonus_yields(self):
        """[(improvement, yield, amount, prereq_tech, prereq_civic)]"""
        return self.q("select ImprovementType, YieldType, BonusYieldChange, PrereqTech, PrereqCivic from Improvement_BonusYieldChanges")

    @cached_property
    def building_yields(self):
        return _table(self.q("select BuildingType, YieldType, YieldChange from Building_YieldChanges"))

    def owner_requirements_met(self, reqset, player):
        """Owner (player) conditions: leader/civilization checks are evaluated, anything else is treated as unmet."""
        reqs, test_any = self.requirement_set(reqset)
        results = []
        for rtype, args, inverse in reqs:
            if player is None:
                return False
            if rtype == "REQUIREMENT_PLAYER_LEADER_TYPE_MATCHES":
                ok = player.get("leader") == args.get("LeaderType")
            elif rtype == "REQUIREMENT_PLAYER_CIVILIZATION_TYPE_MATCHES":
                ok = player.get("civilization") == args.get("CivilizationType")
            elif rtype == "REQUIREMENT_REQUIREMENTSET_IS_MET":
                ok = self.owner_requirements_met(args["RequirementSetId"], player)
            else:
                return False
            results.append(ok != inverse)
        return any(results) if test_any else all(results)

    def requirement_set(self, reqset):
        """([(requirementType, args, inverse)], test_any)"""
        rs_type = self.q("select RequirementSetType from RequirementSets where RequirementSetId=?", reqset)
        test_any = bool(rs_type) and rs_type[0][0] == "REQUIREMENTSET_TEST_ANY"
        reqs = [(rtype, dict(self.q("select Name, Value from RequirementArguments where RequirementId=?", rid)), bool(inv))
                for rid, rtype, inv in self.q(
                    "select r.RequirementId, r.RequirementType, r.Inverse from RequirementSetRequirements s "
                    "join Requirements r on r.RequirementId=s.RequirementId where s.RequirementSetId=?", reqset)]
        return reqs, test_any

    def district_adjacencies(self, district):
        cols = [c[1] for c in self.q("pragma table_info(Adjacency_YieldChanges)")]
        return [dict(zip(cols, r)) for r in self.q(
            "select a.* from District_Adjacencies d join Adjacency_YieldChanges a on a.ID=d.YieldChangeId where d.DistrictType=?",
            district)]

    def boost_class(self, item):
        col = "TechnologyType" if item.startswith("TECH_") else "CivicType"
        row = self.q(f"select BoostClass from Boosts where {col}=?", item)
        return row[0][0] if row else None

    def boost_rows(self):
        return self.q("select coalesce(TechnologyType, CivicType), BoostClass, NumItems, Unit1Type, BuildingType, "
                      "ImprovementType, BoostingTechType, ResourceType, DistrictType from Boosts")

    def traits_of(self, civ, leader):
        """The civilization's and leader's traits, including those of the leaders it inherits from (every major
        leader inherits LEADER_DEFAULT: TRAIT_LEADER_MAJOR_CIV, which carries e.g. BBG's Amani trade-route bonus)."""
        out = {t for (t,) in self.q("select TraitType from CivilizationTraits where CivilizationType=?", civ)}
        seen = set()
        while leader and leader not in seen:
            seen.add(leader)
            out |= {t for (t,) in self.q("select TraitType from LeaderTraits where LeaderType=?", leader)}
            row = self.q("select InheritFrom from Leaders where LeaderType=?", leader)
            leader = row[0][0] if row else None
        return out

    def civ_has_trait(self, civ, leader, trait):
        return trait in self.traits_of(civ, leader)

    def _replaced(self, table, col, item, civ, leader):
        """Is `item` replaced by a unique version this civ has (so the standard one is not buildable)?"""
        for (uniq,) in self.q(f"select CivUnique{col}Type from {table} where Replaces{col}Type=?", item):
            trait = self.q(f"select TraitType from {col}s where {col}Type=?", uniq)
            if trait and trait[0][0] and self.civ_has_trait(civ, leader, trait[0][0]):
                return True
        return False

    def unit_replaced(self, unit, civ, leader):
        return self._replaced("UnitReplaces", "Unit", unit, civ, leader)

    def building_replaced(self, b, civ, leader):
        return self._replaced("BuildingReplaces", "Building", b, civ, leader)

    def district_replaced(self, d, civ, leader):
        return self._replaced("DistrictReplaces", "District", d, civ, leader)

    def district_replaces_done(self, district, done):
        """A unique district counts as the base district it replaces (e.g. Rome's Bath for an Aqueduct)."""
        return any(self.q("select 1 from DistrictReplaces where CivUniqueDistrictType=? and ReplacesDistrictType=?", d, district)
                   for d in done)

    def building_prereqs_met(self, b, buildings):
        pre = [x for (x,) in self.q("select PrereqBuilding from BuildingPrereqs where Building=?", b)]
        return not pre or any(x in buildings for x in pre)

    def unit_era(self, unit):
        """Era of a unit = era of its prerequisite tech or civic (ancient if none)."""
        pt, pc = self.q("select PrereqTech, PrereqCivic from Units where UnitType=?", unit)[0]
        if pt:
            return self.q("select EraType from Technologies where TechnologyType=?", pt)[0][0]
        if pc:
            return self.q("select EraType from Civics where CivicType=?", pc)[0][0]
        return "ERA_ANCIENT"

    def has_boost(self, item):
        col = "TechnologyType" if item.startswith("TECH_") else "CivicType"
        return bool(self.q(f"select 1 from Boosts where {col}=?", item))

    def valid_improvements(self, resource):
        return {i for (i,) in self.q("select ImprovementType from Improvement_ValidResources where ResourceType=?", resource)}

    def improvement_adjacencies(self, imp):
        cols = [c[1] for c in self.q("pragma table_info(Adjacency_YieldChanges)")]
        return [dict(zip(cols, r)) for r in self.q(
            "select a.* from Improvement_Adjacencies i join Adjacency_YieldChanges a on a.ID=i.YieldChangeId where i.ImprovementType=?", imp)]

    def resource_revealed(self, resource, techs):
        row = self.q("select PrereqTech from Resources where ResourceType=?", resource)
        return not row or row[0][0] is None or row[0][0] in techs

    def resource_class(self, resource):
        row = self.q("select ResourceClassType from Resources where ResourceType=?", resource)
        return row[0][0] if row else None

    def is_natural_wonder(self, feature):
        row = self.q("select NaturalWonder from Features where FeatureType=?", feature)
        return bool(row and row[0][0])

    def district_is(self, district, base):
        """district is `base` or a unique district replacing it."""
        return district == base or bool(self.q(
            "select 1 from DistrictReplaces where CivUniqueDistrictType=? and ReplacesDistrictType=?", district, base))

    def building_in_district(self, building, district):
        """The building belongs in `district` (or in the district a unique one replaces)."""
        row = self.q("select PrereqDistrict from Buildings where BuildingType=?", building)
        return bool(row and row[0][0]) and (self.district_is(district, row[0][0]) or self.district_is(row[0][0], district))

    def building_is(self, building, base):
        """building is `base` or a unique building replacing it (Stave Church for Temple)."""
        return building == base or bool(self.q(
            "select 1 from BuildingReplaces where CivUniqueBuildingType=? and ReplacesBuildingType=?", building, base))

    def city_has_building(self, city, base):
        return any(self.building_is(b, base) for b in city["buildings"])

    def is_wonder(self, building):
        row = self.q("select IsWonder from Buildings where BuildingType=?", building)
        return bool(row and row[0][0])

    def terrain_class(self, terrain):
        row = self.q("select TerrainClassType from TerrainClass_Terrains where TerrainType=?", terrain)
        return row[0][0] if row else None

    def district_requires_population(self, district):
        row = self.q("select RequiresPopulation from Districts where DistrictType=?", district)
        return bool(row and row[0][0])

    @cached_property
    def _improvement_plot_modifiers(self):
        out = {}
        for imp, mid in self.q("select ImprovementType, ModifierId from ImprovementModifiers"):
            out.setdefault(imp, []).extend(self.plot_yield_modifier(mid))
        return out

    def improvement_plot_modifiers(self, improvement):
        return self._improvement_plot_modifiers.get(improvement, [])

    def tags_of(self, type_name):
        if not type_name:
            return set()
        return {t for (t,) in self.q("select Tag from TypeTags where Type=?", type_name)}

    def plot_yield_modifier(self, modifier_id, player=None):
        """[(yield, amount, [(requirementType, args, inverse)], test_any)] for a plot-yield modifier,
        following ATTACH chains. Modifiers whose plot requirements we can't evaluate are dropped.
        `player` (civilization/leader) lets owner conditions such as leader checks pass."""
        from .yields import PLOT_REQUIREMENTS, PLOT_YIELD_MODIFIER_TYPES, PLOT_YIELD_MODIFIER_TYPES_SINGLE
        row = self.q("select ModifierType, SubjectRequirementSetId, OwnerRequirementSetId from Modifiers where ModifierId=?", modifier_id)
        if not row:
            return []
        mtype, subj, owner = row[0]
        if owner and not self.owner_requirements_met(owner, player):
            return []
        args = dict(self.q("select Name, Value from ModifierArguments where ModifierId=?", modifier_id))
        if "ModifierId" in args and mtype.endswith("ATTACH_MODIFIER"):
            return self.plot_yield_modifier(args["ModifierId"], player)
        if mtype not in PLOT_YIELD_MODIFIER_TYPES + PLOT_YIELD_MODIFIER_TYPES_SINGLE or "YieldType" not in args:
            return []
        reqs, test_any = [], False
        if subj:
            rs_type = self.q("select RequirementSetType from RequirementSets where RequirementSetId=?", subj)
            test_any = bool(rs_type) and rs_type[0][0] == "REQUIREMENTSET_TEST_ANY"
            for rid, rtype, inverse in self.q(
                    "select r.RequirementId, r.RequirementType, r.Inverse from RequirementSetRequirements s "
                    "join Requirements r on r.RequirementId=s.RequirementId where s.RequirementSetId=?", subj):
                if rtype not in PLOT_REQUIREMENTS:
                    return []
                reqs.append((rtype, dict(self.q("select Name, Value from RequirementArguments where RequirementId=?", rid)), bool(inverse)))
        return [(args["YieldType"], float(args["Amount"]), reqs, test_any)]

    def game_speed_cost_multiplier(self, speed):
        return self.q("select CostMultiplier from GameSpeeds where GameSpeedType=?", speed)[0][0] / 100

    def cost(self, item):
        table, col = {"UNIT": ("Units", "UnitType"), "BUILDING": ("Buildings", "BuildingType"),
                      "DISTRICT": ("Districts", "DistrictType"), "PROJECT": ("Projects", "ProjectType")}[item.split("_")[0]]
        return self.q(f"select Cost from {table} where {col}=?", item)[0][0]
