"""Coverage of the game's rules by the simulator, for the agreed scope: turns 1-100, economy (yields, growth,
housing, amenities, costs, purchases, trade, upkeep, research), every civilization.

Every EffectType used by a modifier in the rules database gets a status:
  implemented  - read by the simulator (found in civ6sim source as a string)
  out:<reason> - outside the scope, with the reason
  todo         - inside the scope and not implemented yet
Same for the tables that attach modifiers to their sources and the tables that carry yields/costs directly.
"""
import re
from collections import defaultdict
from pathlib import Path

SRC = Path(__file__).resolve().parent

# (regex on the effect name without EFFECT_, reason). First match wins; checked after "implemented".
OUT_OF_SCOPE = [
    (r"DIPLOMA|AGENDA|GRIEVANCE|GRIEVENCE|FAVOR|ALLIANCE|ALLIED|ALLY|WARMONGER|EMERGENC|BANNED_DIPLO|DELEGATION|"
     r"EMBASS|OPEN_BORDERS|TRIBUTARY|JOINTWAR|SUZERAIN_FAVOR|RESOLUTION|INTOLERANCE|DIPLO_VISIBILITY|"
     r"ENFORCE_BORDERS|CONDEMNS|LEVY|LEVIED|BECOME_CITY_SUZERAIN|SHARED_VIS", "diplomacy"),
    (r"TOURISM|ROCK_BAND|NATIONAL_PARK|GREAT_WORK_OBJECT|ALBUM|THEMED", "tourism"),
    (r"SPY|ESPIONAGE|COUNTERSPY", "espionage"),
    (r"COMBAT|STRENGTH|ATTACK|DEFEN|ZOC|HEAL|MOVEMENT|MOVE|SIGHT|SEE_|EXPERIENCE|PROMOTION|PILLAG|PLUNDER|WALL|"
     r"FLANK|SUPPORT_BONUS|RAID|BARBARIAN|CAPTURE|RETREAT|PARADROP|JUMP|TELEPORT|EMBARK|IGNORE_TERRAIN|"
     r"IGNORE_RIVERS|IGNORE_SHORES|IGNORE_CLIFF|VALID_TERRAIN|BYPASS|FORMATION|CORPS|ARMY|SIEGE|OUTER_|INNER_|"
     r"RANGED_STRIKE|AIR_DEFENSE|AIR_SLOTS|URBAN_DEFENSES|ESCORT|DAMAGE|ENTER_FOREIGN|START_MOVEMENT|"
     r"KILL|WAR_WEARINESS|CANNOT_ATTACK|NUM_ATTACKS|ESCAPE|HIDDEN_VISIBILITY|FORCE_RETREAT|UNIT_ADVANCED|"
     r"WMD|NO_REDUCTION|GRANT_ABILITY|GRANT_STRENGTH|NEIGHBOR_COMBAT|MILITARY_POLICIES|PER_LUXURY_ATTACK|"
     r"FIGHT_WHILE|EXERT_ZOC|ANTI_AIR|POST_COMBAT|RESTORE_UNIT|GRANT_HEAL|NO_OCCUPATION|POPULATION_AFTER_CONQUEST|"
     r"CONVERTS_BARBARIANS|UNIT_DAMAGE|PREVENT_STRUCTURAL|PREVENT_BYPASS|PREVENT_MELEE", "military"),
    (r"IDENTITY|LOYAL|MARTIAL_LAW", "loyalty"),
    (r"CO2|CARBON|SPACE_RACE|SCIENCE_VICTORY|DIPLOMATIC_VICTORY|RANDOM_EVENT|REACTOR|POWER|MOUNTAIN_PORTAL|"
     r"CLIFFS", "late game / climate"),
    (r"RELIGIOUS_COMBAT|RELIGIOUS_SPREAD|RELIGION_PRESSURE|PRESSURE|INQUISITION|EVICT|SPREAD_CHARGES|"
     r"FOREIGN_SPREAD|NO_FOREIGN_SPREAD|LAND_VICTORY_SPREAD|RELIGIOUS_HEAL|RELIGION_IGNORE|RELIGION_ON_CAPTURE|"
     r"RELIGION_EXTRA_PROMOTIONS|UNITS_RELIGIOUS_STRENGTH|RELIGIOUS_STRENGTH|AUTO_SPREAD|PRESSURE_BURST|"
     r"RELIC|CULTURE_BOMB", "religious spread / combat"),
    (r"ERA_SCORE|COMMEMORATION_QUEST", "era score (ages are read from the save; forward modelling later)"),
    (r"GREAT_PERSON_POINTS|GREAT_PEOPLE_POINTS|GREAT_PERSON_GUARANTEE|PATRONAGE|NO_GREAT_PERSON|"
     r"GREAT_PERSON_CHARGES|GREATPERSON_FAVOR|GREAT_PERSON_CLASS_IN_CITY|EXPENDED_GREAT_PERSON",
     "great person points (who earns which great person is outside the model)"),
    (r"GOODY_HUT", "goody huts (exploration is outside the model)"),
    (r"BOOST_ALL_SPIES|ESCAPE_BOOST|NO_BARB_XP|RANGED_VS_DISTRICT|SEA_ARTIFACTS|NAVAL_UNIT|BLOCK_UNIT_ENTRY|"
     r"UNIT_BUILD_DISABLED|VALID_UNIT_BUILD|EXTRA_UNIT_COPY|DISABLE_SETTLING|PLAYER_CAPITAL$|FORBID_LAND_ROUTE|"
     r"TRADE_ROUTE_DISABLE|WATER_RANGE|IMPROVED_ROUTE_LEVEL|GRANT_ROUTE|ROAD_TO_CAPITAL|CREATE_UNIT|GRANT_UNIT_OF_CLASS|"
     r"GRANT_UNIT_BY|GRANT_UNIT_IN_EACH|TERRAIN_WORK_IMPASSABLE|DEFERRED_CHARGES|INITIATION_YIELD|STEAL_TECH|"
     r"SKIP_FREE_CITY", "military / movement / unit details"),
    (r"INFLUENCE|TOKEN|ENVOY|SUZERAIN_BONUS_DISABLED|BONUS_TYPE_MODIFIER", "envoys / influence (envoy counts come from the save)"),
    (r"TO_PLAYERTYPE|TO_MINORCIVBONUSTYPE", "World Congress resolutions (diplomacy)"),
    (r"GREATWORK|GREAT_WORK|THEMED", "great works (great people are outside the model)"),
    (r"GREAT_PERSON|EARNED_GREAT_PERSON|HAPPINESS_GREAT_PERSON|BOOST_WITH_GREAT_PERSON", "great people (outside the model)"),
    (r"DO_NOTHING|TRIGGER_GAME_MECHANIC|EXPLORE_ENTIRE_MAP|UNIT_OPERATION_AVAILABILITY|SETTLED_FOREIGN|"
     r"GIFT_AND_APPLY", "no economic effect"),
    (r"STRATEGIC_RESOURCE|RESOURCE_STOCKPILE|ACCUMULATION|ACCUMALATION|RESOURCE_IMPORT|MOST_ADVANCED_STRATEGIC|"
     r"UNIT_IGNORE_RESOURCE|NO_CAP_RESOURCE|UPGRADE", "strategic resources / unit upgrades (military)"),
]


# Checked before IN_SCOPE: economy-looking names that are out of scope anyway.
EXPLICIT_OUT = {
    "EFFECT_ADJUST_TRADE_ROUTE_YIELD_CHANGE": "alliance trade routes (diplomacy)",
    "EFFECT_ADJUST_PLAYER_TRADE_ROUTE_DESTINATION_YIELD_FOR_SUZERAIN_ROUTE": "goes to the city-state, not to us",
    "EFFECT_ADJUST_PLAYER_ERA_SCORE_PER_CIVIC_BOOST": "era score (ages are read from the save)",
    "EFFECT_ADJUST_PLAYER_ERA_SCORE_PER_TECH_BOOST": "era score (ages are read from the save)",
    "EFFECT_GRANT_FREE_RESOURCE_EXTRACTED": "strategic resources (iron, oil, aluminum, uranium)",
    "EFFECT_GRANT_PLAYER_FREE_RESOURCE_EXTRACTED": "strategic resources (iron, oil, aluminum, uranium)",
    "EFFECT_ADJUST_GOLD_DISPERSAL": "barbarian camp gold (barbarians are outside the model)",
    "EFFECT_ADJUST_NUM_UNITS_SUPPORTED": "archaeologists (great works)",
    "EFFECT_GOVERNOR_ADJUST_IDENITITY_PER_TITLE": "loyalty",
    "EFFECT_ADJUST_DISTRICT_EXTRA_REGIONAL_RANGE": "regional buildings (factory, power plant, zoo, stadium) come after turn 100",
    "EFFECT_ADJUST_CITY_ALLOWED_INCOMING_REGIONAL_STACKING": "regional buildings (factory, power plant) come after turn 100",
    "EFFECT_TREAT_HOLY_SITE_AS_HOLY_CITY": "religious pressure",
    "EFFECT_ADD_BELIEF": "belief choice (founding a religion needs a great prophet; beliefs come from the save)",
    "EFFECT_ADD_RELIGIOUS_BUILDING": "worship building unlocked by a belief (beliefs come from the save; the buildings are read from it)",
    "EFFECT_ADJUST_PLAYER_DISTRICT_CREATE_YIELD": "Public Transport / neighborhoods (modern era)",
    "EFFECT_ADJUST_CITY_YIELD_PER_FLOOD": "floods (random events)",
    "EFFECT_ADJUST_CITY_POPULATION_UNIT_CREATED": "population lost training unique military units (Janissary, Gadrauht)",
    "EFFECT_ADJUST_FREE_CIVIC_BOOST_FIRST_TRADING_POST_EACH_CIV": "first trading post in each foreign civ (international routes are not planned; random boost)",
    "EFFECT_ADJUST_FREE_TECH_BOOST_FIRST_TRADING_POST_EACH_CIV": "first trading post in each foreign civ (international routes are not planned; random boost)",
    "EFFECT_GRANT_FREE_RESOURCE_IN_CITY": "great merchants' and Porcelain Tower's corporation luxuries (great people / modern era)",
    "EFFECT_ADJUST_PLAYER_TRADE_GAIN_TILES_EN_ROUTE": "Cree traders claiming tiles they walk over (needs trader paths)",
}

# Economy-related names that the out-of-scope patterns would otherwise catch (checked first).
IN_SCOPE = r"TRADE_ROUTE_YIELD|IMPROVEMENT_VALID_TERRAIN|REMOVE_FEATURE|HARVEST|CHOP|PURCHASE|HOUSING|AMENIT|" \
           r"CITY_YIELD|PLOT_YIELD|BUILDING_YIELD|DISTRICT_YIELD|_PRODUCTION$|PRODUCTION_MODIFIER|GROWTH|BOOST"


def implemented_effects():
    names = set()
    for f in SRC.glob("*.py"):
        if f.name == "coverage.py":
            continue
        names |= set(re.findall(r'"(EFFECT_[A-Z0-9_]+)"', f.read_text()))
    return names | {"EFFECT_ATTACH_MODIFIER", "EFFECT_ADJUST_PLOT_YIELD"}  # handled by the engine itself


def classify(effect, implemented, used=True):
    if effect in implemented:
        return "implemented"
    if not used:
        return "out:unused (no source attaches its modifiers)"
    if effect in EXPLICIT_OUT:
        return "out:" + EXPLICIT_OUT[effect]
    short = effect[len("EFFECT_"):]
    if re.search(IN_SCOPE, short) and not re.search(r"GOODY_HUT|STEAL_TECH|CORPS|ARMY|SPACE_RACE|TOURISM|POWERED|"
                                                        r"MILITARY|GREAT_PERSON|CULTURE_BOMB|SPY|SPIES|ESCAPE|GREATWORK", short):
        return "todo"
    for pattern, reason in OUT_OF_SCOPE:
        if re.search(pattern, short):
            return "out:" + reason
    return "todo"


def effect_table(rules):
    rows = rules.q("select d.EffectType, count(*) from Modifiers m join DynamicModifiers d on d.ModifierType=m.ModifierType "
                   "group by d.EffectType order by d.EffectType")
    impl = implemented_effects()
    roots = modifier_roots(rules)
    by_effect = defaultdict(set)
    for e, m in rules.q("select d.EffectType, m.ModifierId from Modifiers m join DynamicModifiers d "
                        "on d.ModifierType=m.ModifierType"):
        by_effect[e] |= roots.get(m, set())
    out = []
    for e, n in rows:
        tabs = by_effect[e]
        if e not in impl and tabs and tabs <= set(SOURCE_TABLES_OUT):
            out.append((e, n, "out:only from " + ", ".join(sorted({SOURCE_TABLES_OUT[t] for t in tabs}))))
        else:
            out.append((e, n, classify(e, impl, bool(tabs))))
    return out


def modifier_roots(rules):
    """ModifierId -> source tables that (directly or through attaching modifiers) give it to a player."""
    direct = defaultdict(set)
    for (t,) in rules.q("select name from sqlite_master where type='table' and name like '%Modifiers' "
                        "and name not in ('Modifiers','DynamicModifiers')"):
        if "modifierid" in [c[1].lower() for c in rules.q(f"pragma table_info({t})")]:
            for (m,) in rules.q(f"select ModifierId from {t}"):
                direct[m].add(t)
    parents = defaultdict(set)
    for m, v in rules.q("select ModifierId, Value from ModifierArguments"):
        parents[v].add(m)
    memo = {}

    def roots(m, seen=()):
        if m in memo:
            return memo[m]
        r = set(direct.get(m, ()))
        for p in parents.get(m, ()):
            if p not in seen:
                r |= roots(p, seen + (m,))
        memo[m] = r
        return r
    return {m: roots(m) for (m,) in rules.q("select ModifierId from Modifiers")}


def attached_modifiers(rules):
    """Modifier ids some source attaches: listed in a *Modifiers table or named as another modifier's argument."""
    out = {v for (v,) in rules.q("select Value from ModifierArguments")}
    for (t,) in rules.q("select name from sqlite_master where type='table' and name like '%Modifiers' "
                        "and name not in ('Modifiers','DynamicModifiers')"):
        if "modifierid" in [c[1].lower() for c in rules.q(f"pragma table_info({t})")]:
            out |= {m for (m,) in rules.q(f"select ModifierId from {t}")}
    return out


# Tables that attach modifiers to a source, and whether the engine reads them.
SOURCE_TABLES_DONE = {"TraitModifiers", "PolicyModifiers", "GovernmentModifiers", "BeliefModifiers",
                      "TechnologyModifiers", "CivicModifiers", "GameModifiers", "BuildingModifiers",
                      "DistrictModifiers", "ImprovementModifiers", "GovernorModifiers", "GovernorPromotionModifiers",
                      "CommemorationModifiers"}
SOURCE_TABLES_OUT = {"UnitPromotionModifiers": "military", "ComplimentModifiers": "diplomacy",
                     "AbstractModifiers": "scenario helper",
                     "GreatPersonIndividualActionModifiers": "great people (outside the model)",
                     "GreatPersonIndividualBirthModifiers": "great people (outside the model)",
                     "GreatPersonIndividualIconModifiers": "great people (outside the model)",
                     "GreatWorkModifiers": "great works (great people are outside the model)",
                     "UnitAbilityModifiers": "unit abilities (military)",
                     "ProjectCompletionModifiers": "project completion bonuses (late-game projects; Court Festival needs Humanism and only gives culture)"}


def source_tables(rules):
    tabs = [t for (t,) in rules.q("select name from sqlite_master where type='table' and name like '%Modifiers' "
                                  "and name not in ('Modifiers','DynamicModifiers')")]
    out = []
    for t in tabs:
        if t in SOURCE_TABLES_DONE:
            out.append((t, "implemented"))
        elif t in SOURCE_TABLES_OUT:
            out.append((t, "out:" + SOURCE_TABLES_OUT[t]))
        else:
            out.append((t, "todo"))
    return out


TABLES_OUT = {"Routes": "roads (movement)", "Routes_XP2": "roads (movement)", "Route_ResourceCosts": "railroads",
              "Building_GreatWorks": "great works", "GreatWorkObjectTypes": "great works",
              "GreatPersonClasses": "great people (outside the model)", "GreatPersonIndividuals": "great people (outside the model)",
              "DiplomaticActions": "diplomacy", "DiplomaticStates": "diplomacy", "DiplomaticStateActions": "diplomacy",
              "DiplomaticActions_XP2": "diplomacy", "CongressAiChanges": "World Congress AI",
              "LoyaltyLevels": "loyalty", "Maps": "map sizes (the map comes from the save)",
              "PseudoYields": "AI valuation weights", "ScoringLineItems": "game score", "WMDs": "nuclear weapons",
              "RandomEvents": "random events (floods, eruptions, storms)", "RandomEvent_Yields": "random events",
              "StartEras": "start-era defaults (the start state comes from the save)",
              "CivicRandomCosts": "future civics (random costs)", "TechnologyRandomCosts": "future techs (random costs)",
              "Buildings_XP2": "power, floods, canal / bridge wonders (industrial era and later)",
              "Building_YieldChangesBonusWithPower": "power (industrial era and later)",
              "Project_ResourceCosts": "late-game projects' strategic resources",
              "Units_XP2": "units' strategic resources and tourism bombs (military / late game)",
              "GameSpeed_Scalings": "half / slight speed scalings: no row in the database refers to them (only the engine might)"}


def yield_tables(rules):
    """Tables (other than the modifier machinery) with a yield / cost / housing / amenity column."""
    out = []
    code = "".join(f.read_text() for f in SRC.glob("*.py") if f.name != "coverage.py")
    for (t,) in rules.q("select name from sqlite_master where type='table'"):
        cols = [c[1] for c in rules.q(f"pragma table_info({t})")]
        if t.endswith("Modifiers") or t.startswith(("Civilopedia", "Ai", "AI", "Strategy")):
            continue
        if any(re.search(r"Yield|Cost|Housing|Entertainment|Maintenance|Amenit|CitizenSlots", c) for c in cols):
            if re.search(rf"\b{t}\b", code):
                out.append((t, "implemented"))
            elif t in TABLES_OUT:
                out.append((t, "out:" + TABLES_OUT[t]))
            elif not rules.q(f"select 1 from {t} limit 1"):
                out.append((t, "out:empty"))
            else:
                out.append((t, "todo"))
    return out
