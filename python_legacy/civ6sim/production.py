"""Production toward a city's current item: percentage modifiers and item costs."""
import math

from .envs import governor_here
from .rules import Rules

# Modifier type -> which item kinds it speeds up.
PRODUCTION_PERCENT_MODIFIERS = {
    "MODIFIER_SINGLE_CITY_ADJUST_DISTRICT_PRODUCTION_MODIFIER": ("DISTRICT",),
    "MODIFIER_SINGLE_CITY_ADJUST_UNIT_PRODUCTION_MODIFIER": ("UNIT",),
    "MODIFIER_SINGLE_CITY_ADJUST_BUILDING_PRODUCTION_MODIFIER": ("BUILDING",),
}




def item_kind(item):
    return item.split("_", 1)[0]


def policy_production_percent(rules: Rules, player, item):
    """% production toward `item` from slotted policies (Ilkum, Colonization, Agoge, Corvee, ...)."""
    pct = 0.0
    kind = item_kind(item)
    unit_class = unit_era = None
    if kind == "UNIT":
        unit_class = rules.q("select PromotionClass from Units where UnitType=?", item)[0][0]
        unit_era = rules.unit_era(item)
    for pol in player.get("policies", []):
        for (mid,) in rules.q("select ModifierId from PolicyModifiers where PolicyType=?", pol):
            mtype, owner = rules.q("select ModifierType, OwnerRequirementSetId from Modifiers where ModifierId=?", mid)[0]
            if owner and not rules.owner_requirements_met(owner, player):
                continue
            a = dict(rules.q("select Name, Value from ModifierArguments where ModifierId=?", mid))
            if mtype == "MODIFIER_PLAYER_UNITS_ADJUST_UNIT_PRODUCTION" and a.get("UnitType") == item:
                pct += float(a["Amount"])
            elif mtype == "MODIFIER_PLAYER_CITIES_ADJUST_UNIT_TAG_ERA_PRODUCTION" and kind == "UNIT" \
                    and a.get("UnitPromotionClass") == unit_class and a.get("EraType") == unit_era:
                pct += float(a["Amount"])
            elif mtype == "MODIFIER_PLAYER_CITIES_ADJUST_WONDER_ERA_PRODUCTION" and kind == "BUILDING" \
                    and rules.q("select IsWonder from Buildings where BuildingType=?", item)[0][0]:
                pct += float(a["Amount"])
    return pct


def production_percent(rules: Rules, player, city, item, require_established=True):
    """Total % bonus on production toward `item` in `city` (governor promotions and policies)."""
    pct = policy_production_percent(rules, player, item)
    for g in player.get("governors", []):
        if not governor_here(g, city, player) or (require_established and not g["established"]):
            continue
        ids = [m for (m,) in rules.q("select ModifierId from GovernorModifiers where GovernorType=?", g["type"])]
        for p in g["promotions"]:
            ids += [m for (m,) in rules.q("select ModifierId from GovernorPromotionModifiers where GovernorPromotionType=?", p)]
        for mid in ids:
            mtype, owner = rules.q("select ModifierType, OwnerRequirementSetId from Modifiers where ModifierId=?", mid)[0]
            if owner and not rules.owner_requirements_met(owner, player):
                continue
            kinds = PRODUCTION_PERCENT_MODIFIERS.get(mtype)
            if kinds and item_kind(item) in kinds:
                pct += float(dict(rules.q("select Name, Value from ModifierArguments where ModifierId=?", mid))["Amount"])
    return pct


def game_progress(rules: Rules, techs, civics):
    """floor(100 * max(techs / all techs, civics / all civics)): completed ones only (CivFanatics formula thread)."""
    n_tech = rules.q("select count(*) from Technologies")[0][0]
    n_civic = rules.q("select count(*) from Civics")[0][0]
    return math.floor(100 * max(len(techs) / n_tech, len(civics) / n_civic))


def game_progress_cost(rules: Rules, base, param, techs, civics):
    """COST_PROGRESSION_GAME_PROGRESS: base * (1 + (Param1% - 1) * progress%) (trader 40 / 400, aqueduct)."""
    return base * (1 + (param / 100 - 1) * game_progress(rules, techs, civics) / 100)


def harvest_yield(rules: Rules, engine, kind, what, techs, civics, city, speed):
    """Yields of harvesting a feature (chop) or a bonus resource: base (Feature_Removes / Resource_Harvests) x
    (1 + 9 x game progress) x game speed (CivFanatics "When to Chop/Harvest": 20 -> 200 over the game), raised by
    Groundbreaker (EFFECT_ADJUST_CITY_RESOURCE_HARVEST_BONUS)."""
    if kind == "feature":
        rows = rules.q("select YieldType, Yield from Feature_Removes where FeatureType=?", what)
    else:
        rows = [(k, v) for k, v, pt in rules.q("select YieldType, Amount, PrereqTech from Resource_Harvests where ResourceType=?", what)]
    p = game_progress(rules, techs, civics) / 100
    bonus = 0.0
    if engine is not None and city is not None:
        bonus = sum(float(a["Amount"]) for a, m, s in engine.city_effects("EFFECT_ADJUST_CITY_RESOURCE_HARVEST_BONUS", city["name"]))
    # rounded down at the end (4 clean AI chops in game_auto2: 14.9 -> 14, 45.1 -> 45, 39.7 -> 39, 44.2 -> 44)
    return {k: math.floor(v * (1 + 9 * p) * rules.game_speed_cost_multiplier(speed) * (1 + bonus / 100)) for k, v in rows}


def gold_price(exact_cost, discount_pct=0.0):
    """Gold purchase price: 4 x the production cost (unrounded for buildings and units, the rounded cost for
    districts), less purchase discounts, rounded down to a multiple of 5 (the game's
    GetPurchaseCost on online speed: settler 70 -> 280, builder 31 -> 120, trader 21.x -> 85)."""
    return math.floor(4 * exact_cost * (1 - discount_pct / 100) / 5) * 5


def faith_price(exact_cost, discount_pct=0.0):
    """Faith purchase price: 2 x the production cost (1 faith = 0.5 production = 2 gold, Civ6 wiki "Faith"), less
    discounts, rounded down to a multiple of 5 like gold. The rounding is assumed from the gold rule (not yet
    checked against the game)."""
    return math.floor(2 * exact_cost * (1 - discount_pct / 100) / 5) * 5


def purchase_yields(rules: Rules, engine, city, item):
    """Currencies `item` can be bought with in `city`: the table's PurchaseYield, plus faith or gold opened by
    modifiers (Monumentality civilians, Jesuit Education campus/theater buildings, Mali commercial hub, Moksha's
    districts for faith, Reyna's Contractor districts for gold, ...). Empty when it can't be bought."""
    name, kind = city["name"], item_kind(item)
    eff = lambda e: engine.city_effects(e, name) + engine.player_effects(e)
    out = []
    if kind == "UNIT":
        row = rules.q("select PurchaseYield, MustPurchase from Units where UnitType=?", item)[0]
        if row[0]:
            out.append(row[0])
        tags = {t for (t,) in rules.q("select Tag from TypeTags where Type=?", item)}
        if any(a.get("Tag") in tags for a, m, s in eff("EFFECT_ENABLE_UNIT_FAITH_PURCHASE")):
            out.append("YIELD_FAITH")
    elif kind == "BUILDING":
        row = rules.q("select PurchaseYield, PrereqDistrict from Buildings where BuildingType=?", item)[0]
        if row[0]:
            out.append(row[0])
        districts = {row[1]} | {d for (d,) in rules.q(
            "select CivUniqueDistrictType from DistrictReplaces where ReplacesDistrictType=?", row[1])} if row[1] else set()
        if row[1] is None:
            districts = {"DISTRICT_CITY_CENTER"}
        if any(a.get("DistrictType") in districts for a, m, s in eff("EFFECT_ENABLE_BUILDING_FAITH_PURCHASE")) or \
                any(a.get("BuildingType") == item for a, m, s in eff("EFFECT_ENABLE_SPECIFIC_BUILDING_FAITH_PURCHASE")):
            out.append("YIELD_FAITH")
    elif kind == "DISTRICT":
        if any(a.get("CanPurchase") in ("1", "true") for a, m, s in eff("EFFECT_ADJUST_CITY_CAN_PURCHASE_DISTRICTS")):
            out.append("YIELD_GOLD")
        if any(a.get("CanPurchase") in ("1", "true") for a, m, s in eff("EFFECT_ADJUST_CAN_FAITH_PURCHASE_DISTRICTS")):
            out.append("YIELD_FAITH")
    return list(dict.fromkeys(out))


def unit_cost(rules: Rules, player, unit, speed, exact=False):
    cost, model, param = rules.q(
        "select Cost, CostProgressionModel, CostProgressionParam1 from Units where UnitType=?", unit)[0]
    if model == "COST_PROGRESSION_PREVIOUS_COPIES":
        cost += param * player.get("unitsTrained", {}).get(unit, 0)
    elif model == "NO_COST_PROGRESSION":
        # base x speed is rounded down before anything else (slinger 35 -> 17, priced 65 = floor5(4 x 17))
        return int(cost * rules.game_speed_cost_multiplier(speed))
    elif model == "COST_PROGRESSION_GAME_PROGRESS":
        c = game_progress_cost(rules, cost, param, player.get("techs", ()), player.get("civics", ())) \
            * rules.game_speed_cost_multiplier(speed)
        return c if exact else math.floor(c)
    elif model != "NO_COST_PROGRESSION":
        return None  # game-progress models not calibrated yet
    return int(cost * rules.game_speed_cost_multiplier(speed))  # rounded down (builder 62 -> 31)


def building_cost(rules: Rules, building, speed, exact=False):
    """Production cost (rounded down); exact=True gives the unrounded cost the gold price is based on
    (granary 65 x 0.5 = 32.5 -> 130 gold, confirmed on 21491 building prices)."""
    c = rules.q("select Cost from Buildings where BuildingType=?", building)[0][0] * rules.game_speed_cost_multiplier(speed)
    return c if exact else int(c)


def _unused_building_cost(rules: Rules, building, speed):
    return int(rules.q("select Cost from Buildings where BuildingType=?", building)[0][0] * rules.game_speed_cost_multiplier(speed))


# Districts (COST_PROGRESSION_NUM_UNDER_AVG_PLUS_TECH):
#   cost = base * speed * (1 + K * max(techs / all techs, civics / all civics))
#   The community gives K = 9; the three observations so far (Rome's Government Plaza 25, the Scythia Encampment's
#   prices 135 / 140 at T20 / T21, Cumae's Commercial Hub between 41 and 54) need K in 7.7..8.5. K = 8.1 is
#   provisional until the probe has more district costs.
#   Discount (community analysis + BBG's base.sql setting CostProgressionParam1 to 35 / 20): with A = specialty
#   district types unlocked, B = specialty districts completed, C = districts of this type placed or completed,
#   the district is Param1% cheaper when B >= A and B / A > C. The community also says the cost is locked when
#   the district is placed, but Rome's Government Plaza only fits a cost computed at completion: LOCK is off.
# Fitted on 66 district placements of a 104-turn autoplay game (costs read from the district records in the
# saves): K = 8.84 with denominators 78 / 61 and rounding gives 38 exact and the rest within 1-4. The cost
# recorded for a district never changes while it is built, so it is locked when placed.
DISTRICT_PROGRESS_K = 8.84
DISTRICT_TECH_TOTAL, DISTRICT_CIVIC_TOTAL = 78, 61
LOCK_DISTRICT_COST = True


def district_cost(rules: Rules, district, speed, techs=(), civics=(), placed_counts=None, unlocked=None,
                  completed=None, exact=False):
    """District production cost (community analysis, CivFanatics "District cost actual numbers" / "District
    Discount Mechanism", fitted on 1703 game costs: 1675 exact):
      p = floor(100 * max(techs / all techs, civics / all civics)) / 100     (completed ones only)
      cost = floor(floor(base * f) * (1 + 9 p) * speed)
    f = 1 - Param1% (BBG 35, Government Plaza / Diplomatic Quarter 20) when B >= A and B / A > C, where A = specialty
    district types unlocked, B = specialty districts completed (both as of the last tech/civic finished), C =
    districts of this type placed or completed; else f = 1. exact=True leaves the last rounding out (gold price)."""
    base, param, model = rules.q("select Cost, CostProgressionParam1, CostProgressionModel from Districts where DistrictType=?",
                                 district)[0]
    if model == "COST_PROGRESSION_GAME_PROGRESS":  # aqueduct, dam, canal, neighborhood
        c = game_progress_cost(rules, base, param, techs, civics) * rules.game_speed_cost_multiplier(speed)
        return c if exact else math.floor(c)
    if model != "COST_PROGRESSION_NUM_UNDER_AVG_PLUS_TECH":
        c = base * rules.game_speed_cost_multiplier(speed)
        return c if exact else math.floor(c)
    n_tech = rules.q("select count(*) from Technologies")[0][0]
    n_civic = rules.q("select count(*) from Civics")[0][0]
    progress = math.floor(100 * max(len(techs) / n_tech, len(civics) / n_civic)) / 100
    f = 1.0
    if placed_counts is not None and unlocked:
        a = len(unlocked)
        b = completed if completed is not None else 0
        if b >= a and b / a > placed_counts.get(district, 0):
            f = 1 - param / 100
    cost = math.floor(base * f) * (1 + 9 * progress) * rules.game_speed_cost_multiplier(speed)
    return cost if exact else math.floor(cost)


def unlocked_districts(rules: Rules, techs, civics, civilization=None):
    out = []
    for d, pt, pc, trait in rules.q("select DistrictType, PrereqTech, PrereqCivic, TraitType from Districts "
                                    "where RequiresPopulation=1 and DistrictType <> 'DISTRICT_CITY_CENTER'"):
        if trait:
            continue  # unique districts replace a base one; not counted separately here
        if (pt is None or pt in techs) and (pc is None or pc in civics):
            out.append(d)
    return out


def item_cost(rules: Rules, player, item, speed, placed_counts=None, exact=False):
    kind = item_kind(item)
    if exact and kind == "UNIT":
        return unit_cost(rules, player, item, speed, exact=True)
    if exact and kind == "BUILDING":
        return building_cost(rules, item, speed, exact=True)
    if kind == "DISTRICT":
        techs, civics = player.get("techs", ()), player.get("civics", ())
        return district_cost(rules, item, speed, techs, civics, placed_counts,
                             unlocked_districts(rules, set(techs), set(civics)),
                             completed=player.get("_districts_completed"), exact=exact)
    if kind == "UNIT":
        return unit_cost(rules, player, item, speed)
    if kind == "BUILDING":
        return building_cost(rules, item, speed)
    if kind == "PROJECT":
        return project_cost(rules, player, item, speed, exact)
    return None


def project_cost(rules: Rules, player, project, speed, exact=False):
    """Project cost: like units and districts, base scaled by game progress (COST_PROGRESSION_GAME_PROGRESS) and
    game speed."""
    row = rules.q("select Cost, CostProgressionModel, CostProgressionParam1 from Projects where ProjectType=?", project)
    if not row:
        return None
    cost, model, param = row[0]
    if model == "COST_PROGRESSION_GAME_PROGRESS":
        c = game_progress_cost(rules, cost, param, player.get("techs", ()), player.get("civics", ())) \
            * rules.game_speed_cost_multiplier(speed)
    else:
        c = cost * rules.game_speed_cost_multiplier(speed)
    return c if exact else math.floor(c)


def item_completed(item, city_before, city_after, player_before, player_after):
    """Did `item` finish between two saves (as opposed to the player switching away from it)?"""
    kind = item_kind(item)
    if kind == "UNIT":
        return player_after.get("unitsTrained", {}).get(item, 0) > player_before.get("unitsTrained", {}).get(item, 0)
    if kind == "BUILDING":
        return item in city_after["buildings"] and item not in city_before["buildings"]
    if kind == "DISTRICT":
        done = lambda c: any(d["type"] == item and d["complete"] for d in c["districts"])
        return done(city_after) and not done(city_before)
    return False


def builder_charges(rules, engine, unit, city_name):
    """Build charges of a unit trained now in city_name: the unit's own plus EFFECT_ADJUST_UNIT_BUILD_CHARGES
    (Serfdom / Public Works +2 for units trained while slotted, Pyramids +1, Liang's Guildmaster +1 in her city)."""
    base = rules.q("select BuildCharges from Units where UnitType=?", unit)[0][0] or 0
    return base + sum(int(float(a.get("Amount") or 0)) for a, m, s in engine.unit_effects("EFFECT_ADJUST_UNIT_BUILD_CHARGES", unit, city_name))


# Effects that speed up production, by what they apply to. Amounts are percents and add up with the city's
# general production percents.
def _era_index(rules, era):
    row = rules.q("select ChronologyIndex from Eras where EraType=?", era)
    return row[0][0] if row else None


def item_production_percent(rules, engine, city, item):
    """% extra production toward `item` in `city` from modifiers (policies, governors, city-states, wonders...)."""
    name = city["name"]
    kind = item_kind(item)
    pct = 0.0
    eff = lambda e: engine.city_effects(e, name)
    if kind == "UNIT":
        cls, domain = rules.q("select PromotionClass, Domain from Units where UnitType=?", item)[0]
        era = rules.unit_era(item)
        military = cls is not None and item not in ("UNIT_BUILDER", "UNIT_SETTLER", "UNIT_TRADER")
        for a, m, s in (eff("EFFECT_ADJUST_UNIT_PRODUCTION") + engine.player_effects("EFFECT_ADJUST_UNIT_PRODUCTION")
                        + engine.unit_effects("EFFECT_ADJUST_UNIT_PRODUCTION", item, name)):
            if a.get("UnitType") == item:
                pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_UNIT_TAG_ERA_PRODUCTION"):
            if a.get("UnitPromotionClass") == cls and a.get("EraType") == era:
                pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_ALL_UNIT_PRODUCTION_MODIFIER"):
            pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_UNIT_DOMAIN_PRODUCTION"):
            if a.get("Domain") == domain:
                pct += float(a["Amount"])
        if military:
            from .greatpeople import government_bonus
            pct += government_bonus(engine, "GOVERNMENTBONUS_UNIT_PRODUCTION")
            ei = _era_index(rules, era)
            for a, m, s in eff("EFFECT_ADJUST_CITY_ALL_MILITARY_UNITS_PRODUCTION"):
                lo, hi = _era_index(rules, a.get("StartEra")), _era_index(rules, a.get("EndEra"))
                if (lo is None or ei >= lo) and (hi is None or ei <= hi) and a.get("PromotionClass") in (None, cls):
                    pct += float(a["Amount"])
    elif kind == "BUILDING":
        wonder, district = rules.q("select IsWonder, PrereqDistrict from Buildings where BuildingType=?", item)[0]
        for a, m, s in eff("EFFECT_ADJUST_BUILDING_PRODUCTION"):
            if a.get("BuildingType") == item or (a.get("DistrictType") and a.get("DistrictType") == district):
                pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_ALL_BUILDING_PRODUCTION_MODIFIER"):
            if a.get("IsWonder") in (None, "1" if wonder else "0", "true" if wonder else "false"):
                pct += float(a["Amount"])
        if wonder:
            era = rules.q("select EraType from Technologies where TechnologyType=(select PrereqTech from Buildings where BuildingType=?)", item) or \
                rules.q("select EraType from Civics where CivicType=(select PrereqCivic from Buildings where BuildingType=?)", item)
            ei = _era_index(rules, era[0][0]) if era else None
            for a, m, s in eff("EFFECT_ADJUST_WONDER_PRODUCTION"):
                pct += float(a["Amount"])
            for a, m, s in eff("EFFECT_ADJUST_WONDER_ERA_PRODUCTION"):
                lo, hi = _era_index(rules, a.get("StartEra")), _era_index(rules, a.get("EndEra"))
                if ei is not None and (lo is None or ei >= lo) and (hi is None or ei <= hi):
                    pct += float(a["Amount"])
            from .greatpeople import government_bonus
            pct += government_bonus(engine, "GOVERNMENTBONUS_WONDER_CONSTRUCTION")
            pct += _placement_percent(rules, engine, city, _item_plot(engine, city, item), "WONDER")
        elif district:
            for a, m, s in eff("EFFECT_ADJUST_DISTRICT_BUILDING_PRODUCTION"):
                pct += float(a["Amount"])
            # Hungary: buildings in a district across a river from the city center
            d = next((x for x in city["districts"] if rules.building_in_district(item, x["type"])), None)
            if d is not None and d["plot"].get("river") and _center_river(engine, city):
                for a, m, s in eff("EFFECT_ADJUST_ADJACENT_CITY_RIVER_BUILDING_PRODUCTION"):
                    pct += float(a["Amount"])
    elif kind == "DISTRICT":
        base = {item} | {b for (b,) in rules.q("select ReplacesDistrictType from DistrictReplaces where CivUniqueDistrictType=?", item)}
        for a, m, s in eff("EFFECT_ADJUST_DISTRICT_PRODUCTION"):
            if a.get("DistrictType") in base:
                pct += float(a["Amount"])
        for e in ("EFFECT_ADJUST_ALL_DISTRICT_PRODUCTION_MODIFIER", "EFFECT_ADJUST_ALL_DISTRICTS_PRODUCTION"):
            for a, m, s in eff(e):
                pct += float(a["Amount"])
        from .greatpeople import government_bonus
        pct += government_bonus(engine, "GOVERNMENTBONUS_DISTRICT_PRODUCTION")
        plot = _item_plot(engine, city, item)
        pct += _placement_percent(rules, engine, city, plot, "DISTRICT")
        if plot is not None and plot.get("river") and _center_river(engine, city):
            for a, m, s in eff("EFFECT_ADJUST_ADJACENT_CITY_RIVER_DISTRICT_PRODUCTION"):
                pct += float(a["Amount"])
    elif kind == "PROJECT":
        for a, m, s in eff("EFFECT_ADJUST_PROJECT_PRODUCTION"):
            if a.get("ProjectType") == item:
                pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_ALL_PROJECTS_PRODUCTION"):
            pct += float(a["Amount"])
    return pct


def _item_plot(engine, city, item):
    """Where a district or wonder being built stands (placed when its production started)."""
    if item_kind(item) == "DISTRICT":
        d = next((x for x in city["districts"] if x["type"] == item), None)
        return d["plot"] if d is not None else None
    return next((q for q in engine.plots.values() if q.get("wonder") == item), None)


def _center_river(engine, city):
    return bool((engine.plots.get((city["center"]["x"], city["center"]["y"])) or {}).get("river"))


def _placement_percent(rules, engine, city, plot, kind):
    """% for where a district / wonder stands: next to a river (Egypt), next to a natural wonder (Ik Kil).
    "Across a river from the city center" (Hungary) is taken as both on a river (river edges are not exported)."""
    if plot is None:
        return 0.0
    from .yields import neighbour_xy
    pct = 0.0
    if plot.get("river"):
        for a, m, s in engine.city_effects(f"EFFECT_ADJUST_RIVER_{kind}_PRODUCTION", city["name"]):
            pct += float(a["Amount"])
    near = {(engine.plots.get(xy) or {}).get("feature") for xy in neighbour_xy(plot["x"], plot["y"])}
    for a, m, s in engine.city_effects(f"EFFECT_ADJUST_{kind}_ADJACENT_NATURAL_WONDER_PRODUCTION", city["name"]):
        if a.get("FeatureType") in near:
            pct += float(a["Amount"])
    return pct


# literal names for the coverage check (built with f-strings above)
_PLACEMENT_EFFECTS = ("EFFECT_ADJUST_RIVER_DISTRICT_PRODUCTION", "EFFECT_ADJUST_RIVER_WONDER_PRODUCTION",
                      "EFFECT_ADJUST_DISTRICT_ADJACENT_NATURAL_WONDER_PRODUCTION",
                      "EFFECT_ADJUST_WONDER_ADJACENT_NATURAL_WONDER_PRODUCTION")


def item_production_flat(rules, engine, city, item):
    """Flat production added to the city while it builds this kind of item (EFFECT_ADJUST_CITY_PRODUCTION_UNIT /
    _BUILDING / _DISTRICT, e.g. industrial city-states' envoy bonus in the capital). Shows in the city's yield."""
    effect = {"UNIT": "EFFECT_ADJUST_CITY_PRODUCTION_UNIT", "BUILDING": "EFFECT_ADJUST_CITY_PRODUCTION_BUILDING",
              "DISTRICT": "EFFECT_ADJUST_CITY_PRODUCTION_DISTRICT"}.get(item_kind(item))
    return sum(float(a["Amount"]) for a, m, s in engine.city_effects(effect, city["name"])) if effect else 0.0


def purchase_discount_percent(rules, engine, city, item, gold=True):
    """% off the gold price of `item` in `city` (EFFECT_ADJUST_*_PURCHASE_COST; amounts are discounts), plus
    Democracy's gold purchase bonus."""
    name, kind = city["name"], item_kind(item)
    eff = lambda e: engine.city_effects(e, name) + engine.player_effects(e)
    pct = 0.0
    if gold:
        from .greatpeople import government_bonus
        pct += government_bonus(engine, "GOVERNMENTBONUS_GOLD_PURCHASES")
    if kind == "UNIT":
        domain, cls = rules.q("select Domain, PromotionClass from Units where UnitType=?", item)[0]
        civilian = cls is None
        for a, m, s in eff("EFFECT_ADJUST_UNIT_PURCHASE_COST"):
            if a.get("UnitType") == item:
                pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_ALL_UNITS_PURCHASE_COST"):
            if (a.get("UnitDomain") in (None, domain)) and (not civilian or a.get("IncludeCivilian") in ("1", "true", "True")):
                pct += float(a["Amount"])
    elif kind == "BUILDING":
        for a, m, s in eff("EFFECT_ADJUST_BUILDING_PURCHASE_COST"):
            if a.get("BuildingType") == item:
                pct += float(a["Amount"])
        for a, m, s in eff("EFFECT_ADJUST_ALL_BUILDINGS_PURCHASE_COST"):
            pct += float(a["Amount"])
    elif kind == "DISTRICT":
        for a, m, s in eff("EFFECT_ADJUST_ALL_DISTRICTS_PURCHASE_COST"):
            pct += float(a["Amount"])
    return pct


def faith_discount_percent(rules, engine, city, item):
    """% off the faith price: the gold discounts, Theocracy's bonus, plus Saladin's worship building at one tenth
    (any player buying the worship building of his religion; here only for Saladin himself)."""
    from .greatpeople import government_bonus
    pct = purchase_discount_percent(rules, engine, city, item, gold=False) + government_bonus(engine, "GOVERNMENTBONUS_FAITH_PURCHASES")
    if item_kind(item) == "BUILDING" and rules.q("select 1 from Buildings where BuildingType=? and EnabledByReligion=1", item):
        for a, m, s in engine.player_effects("EFFECT_ADJUST_RELIGION_BUILDING_DISCOUNT"):
            pct += float(a["Discount"])
    return min(pct, 100.0)


def maintenance(rules: Rules, engine, cities, units):
    """Gold upkeep per turn: buildings and completed districts (their Maintenance), plus units (Maintenance less
    EFFECT_ADJUST_UNIT_MAINTENANCE_DISCOUNT per unit, Conscription / Levee en Masse, not below 0).
    units: [unit type] or {type: count}."""
    total = 0.0
    for c in cities:
        for b in c["buildings"]:
            row = rules.q("select Maintenance from Buildings where BuildingType=?", b)
            total += (row[0][0] or 0) if row else 0
        for d in c["districts"]:
            if d["complete"]:
                row = rules.q("select Maintenance from Districts where DistrictType=?", d["type"])
                total += (row[0][0] or 0) if row else 0
    discount = sum(float(a["Amount"]) for a, m, s in engine.player_effects("EFFECT_ADJUST_UNIT_MAINTENANCE_DISCOUNT"))
    counts = units if isinstance(units, dict) else {u: units.count(u) for u in set(units)}
    for u, n in counts.items():
        row = rules.q("select Maintenance from Units where UnitType=?", u)
        m = (row[0][0] or 0) if row else 0
        if m:
            total += n * max(0.0, m - discount)
    return total


def trade_route_capacity(engine):
    """Trade routes the player may run: EFFECT_ADJUST_TRADE_ROUTE_CAPACITY from every source (Foreign Trade civic,
    commercial hub / harbor districts, wonders, traits such as BBG Joao III's medieval +1)."""
    cap = sum(float(a.get("Amount") or 0) for subj, a, m, s in engine.effects.get("EFFECT_ADJUST_TRADE_ROUTE_CAPACITY", ()))
    met = engine.player.get("majorsMet") or 0  # Joao III: +1 per major civilization met
    cap += met * sum(float(a.get("Amount") or 0) for a, m, s in engine.player_effects("EFFECT_ADJUST_PLAYER_TRADE_ROUTE_CAPACITY_ON_MEETING"))
    return int(cap)


def policy_slots(rules, engine, government):
    """Policy slots of the government plus modifier changes: wonder / trait slots of a type (EFFECT_ADJUST_PLAYER_
    GOVERNMENT_SLOT_TYPE, +1 each), wildcard gains or losses (EFFECT_ADJUST_GOVERNMENT_SLOTS) and slot types
    turned into another (EFFECT_REPLACE_PLAYER_GOVERNMENT_SLOT_TYPE: one, or all with ReplacesAll)."""
    slots = []
    for slot, n in rules.q("select GovernmentSlotType, NumSlots from Government_SlotCounts where GovernmentType=?", government):
        slots += [slot] * n
    rows = lambda e: [a for subj, a, m, s in engine.effects.get(e, ())]
    for a in rows("EFFECT_ADJUST_PLAYER_GOVERNMENT_SLOT_TYPE"):
        slots.append(a["GovernmentSlotType"])
    for a in rows("EFFECT_ADJUST_GOVERNMENT_SLOTS"):
        n = int(float(a.get("Amount") or 0))
        for _ in range(abs(n)):
            if n > 0:
                slots.append(a["GovernmentSlotType"])
            elif a["GovernmentSlotType"] in slots:
                slots.remove(a["GovernmentSlotType"])
    for a in rows("EFFECT_REPLACE_PLAYER_GOVERNMENT_SLOT_TYPE"):
        old, new = a["ReplacedGovernmentSlotType"], a["AddedGovernmentSlotType"]
        if a.get("ReplacesAll") in ("1", "true", "True"):
            slots = [new if x == old else x for x in slots]
        elif old in slots:
            slots[slots.index(old)] = new
    return slots


def tile_price(rules, engine, city, plot, speed, techs, civics):
    """Gold to buy a plot (community formula, CivFanatics "Formula Thread"): base 50 (two tiles from the center) or
    75 (three), times (1 + 4 * game progress%) and game speed, less plot purchase discounts (EFFECT_ADJUST_PLOT_
    PURCHASE_COST, and _TERRAIN for traits keyed on the plot's terrain such as Canada's tundra/snow), rounded down
    to a multiple of 5 before the discount."""
    from .yields import hex_distance
    d = hex_distance((city["center"]["x"], city["center"]["y"]), (plot["x"], plot["y"]))
    base = 50 if d <= 2 else 75
    pct = sum(float(a.get("Amount") or 0) for a, m, s in engine.city_effects("EFFECT_ADJUST_PLOT_PURCHASE_COST", city["name"]))
    pct += sum(float(a.get("Amount") or 0) for a, m, s in engine.city_effects("EFFECT_ADJUST_PLOT_PURCHASE_COST_TERRAIN", city["name"])
               if a.get("TerrainType") == plot["terrain"])
    v = base * (1 + 4 * game_progress(rules, techs, civics) / 100) * rules.game_speed_cost_multiplier(speed)
    # rounded down to 5 first; a discount applies to that and is rounded down to a whole number (Canada's tundra
    # -50%: 25 -> 12)
    return math.floor(5 * math.floor(round(v, 6) / 5) * (1 + pct / 100))
