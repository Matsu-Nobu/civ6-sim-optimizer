"""Amenities and the resulting happiness state (yield and growth modifiers)."""
import math

from .rules import Rules


LUXURY_PER_TYPE = True  # each luxury type serves 4 cities, extra copies add nothing (fitted on game_auto2)


def improved_luxuries(rules: Rules, cities, player=None):
    """Improved luxury resource types owned across the empire -> copies, plus those of city-states we are
    suzerain of."""
    out = {}
    for e in (player or {}).get("envoys", []):
        if e.get("suzerain"):
            for r in e.get("resources", ()):
                if rules.resource_class(r) == "RESOURCECLASS_LUXURY":
                    out[r] = out.get(r, 0) + 1
    for c in cities:
        for p in c["owned"]:
            r = p["resource"]
            if r and rules.resource_class(r) == "RESOURCECLASS_LUXURY" and not p.get("pillaged") \
                    and p["improvement"] in rules.valid_improvements(r):
                out[r] = out.get(r, 0) + 1
    return out


def amenities_needed(rules: Rules, pop):
    return max(0, math.ceil(pop / rules.param("CITY_POP_PER_AMENITY")) - int(rules.param("CITY_AMENITIES_FOR_FREE")))


def amenity_supply(rules: Rules, cities, player=None):
    """City name -> amenities supplied: buildings and districts that entertain, improved luxuries (each copy +1 to
    up to 4 cities, neediest first), and modifier effects (policies, traits, districts, improvements, governors,
    religion, city-states, great people) when `player` is given."""
    supply = {c["name"]: 0.0 for c in cities}
    for c in cities:
        for b in c["buildings"]:
            supply[c["name"]] += rules.q("select Entertainment from Buildings where BuildingType=?", b)[0][0] or 0
        for d in c["districts"]:
            if d["complete"]:
                row = rules.q("select Entertainment from Districts where DistrictType=?", d["type"])
                supply[c["name"]] += (row[0][0] or 0) if row else 0
    if player is not None:
        from .modifiers import engine_for
        e = engine_for(rules, player, cities)
        for c in cities:
            n = c["name"]
            for effect in ("EFFECT_ADJUST_POLICY_AMENITY", "EFFECT_ADJUST_TRAIT_AMENITY", "EFFECT_ADJUST_IMPROVEMENT_AMENITY",
                           "EFFECT_ADJUST_CITY_AMENITIES_FROM_GOVERNORS", "EFFECT_ADJUST_CITY_AMENITIES_FROM_RELIGION",
                           "EFFECT_ADJUST_CITY_AMENITIES_FROM_CITY_STATES", "EFFECT_ADJUST_CITY_AMENITIES_FROM_GREAT_PEOPLE"):
                for a, mid, src in e.city_effects(effect, n):
                    supply[n] += float(a.get("Amount") or 0)
            for effect in ("EFFECT_ADJUST_DISTRICT_AMENITY", "EFFECT_ADJUST_IMPROVEMENT_AMENITY"):
                for d, a, mid, src in e.district_effects(effect, n):
                    supply[n] += float(a.get("Amount") or 0)
                for p, a, mid, src in e.owned_plot_effects(effect, n):
                    supply[n] += float(a.get("Amount") or 0)
            # entertainment from modifiers: Thermal Bath, Pamukkale (city); Khmer aqueducts, Indonesian coastal
            # complexes, Australian charming city centers (district)
            for effect in ("EFFECT_ADJUST_CITY_ENTERTAINMENT", "EFFECT_ADJUST_NATURAL_WONDER_AMENITY"):
                for a, mid, src in e.city_effects(effect, n):
                    supply[n] += float(a.get("Amount") or 0)
            for d, a, mid, src in e.district_effects("EFFECT_ADJUST_DISTRICT_EXTRA_ENTERTAINMENT", n):
                if d is None or d.get("complete", True):
                    supply[n] += float(a.get("Amount") or 0)
            # Huey Teocalli: per lake tile next to the wonder
            for a, mid, src in e.city_effects("EFFECT_ADJUST_CITY_ENTERTAINMENT_FROM_WONDER_ADJACENT_TO_LAKE", n):
                if a["BuildingType"] in c["buildings"]:
                    from .yields import neighbour_xy
                    for xy, q in e.plots.items():
                        if q.get("wonder") == a["BuildingType"]:
                            supply[n] += sum(1 for nxy in neighbour_xy(*xy) if (e.plots.get(nxy) or {}).get("lake"))
            # Grand Bazaar: per improved luxury in the city
            lux_here = [p for p in c["owned"] if p["resource"] and rules.resource_class(p["resource"]) == "RESOURCECLASS_LUXURY"
                        and not p.get("pillaged") and p["improvement"] in rules.valid_improvements(p["resource"])]
            for a, mid, src in e.city_effects("EFFECT_ADJUST_CITY_EXTRA_AMENITY_FOR_LUXURY_DIVERSITY", n):
                supply[n] += float(a.get("Amount") or 0) * len(lux_here)
            # Maya: per luxury next to the city center
            for a, mid, src in e.city_effects("EFFECT_ADJUST_ADD_AMENITY_PER_ADJACENT_LUXURY", n):
                from .yields import neighbour_xy
                near = set(neighbour_xy(c["center"]["x"], c["center"]["y"]))
                supply[n] += float(a.get("AddAmenity") or 1) * sum(
                    1 for p in c["owned"] if (p["x"], p["y"]) in near and p["resource"]
                    and rules.resource_class(p["resource"]) == "RESOURCECLASS_LUXURY")
            for a, mid, src in e.city_effects("EFFECT_ADJUST_RELIGION_AMENITIES_FOR_MINIMUM_FOLLOWERS", n):
                rel = c.get("religion") or {}
                if rel.get("majority") and (rel.get("followers") or {}).get(rel["majority"], 0) >= int(a.get("Followers") or 0):
                    supply[n] += float(a.get("Amenities") or 0)
    need = {c["name"]: amenities_needed(rules, c["population"]) for c in cities}
    per_lux, types = 4, dict(improved_luxuries(rules, cities, player))
    if player is not None:
        # Montezuma: each luxury reaches extra cities; Buenos Aires: improved bonus resources count as luxuries
        per_lux += sum(int(float(a.get("Amount") or 0)) for a, m, s in e.player_effects("EFFECT_ADJUST_OWNED_LUXURY_EXTRA_AMENITIES"))
        if e.player_effects("EFFECT_ADJUST_OWNED_BONUS_RESOURCE_EXTRA_AMENITIES"):
            for c in cities:
                for p in c["owned"]:
                    r = p["resource"]
                    if r and rules.resource_class(r) == "RESOURCECLASS_BONUS" and not p.get("pillaged") \
                            and p["improvement"] in rules.valid_improvements(r):
                        types[r] = types.get(r, 0) + 1
    for lux, copies in types.items():
        order = sorted(cities, key=lambda c: supply[c["name"]] - need[c["name"]])
        for c in order[:per_lux * (1 if LUXURY_PER_TYPE else copies)]:
            supply[c["name"]] += 1
    return supply


def amenity_scores(rules: Rules, cities, player=None):
    """City name -> amenities minus amenities required (one per two citizens rounded up, minus
    CITY_AMENITIES_FOR_FREE: the game reports 1 needed at population 3)."""
    supply = amenity_supply(rules, cities, player)
    return {c["name"]: supply[c["name"]] - amenities_needed(rules, c["population"]) for c in cities}


def happiness(rules: Rules, score):
    """(non-food yield %, growth %) for an amenity score, from the Happinesses table."""
    for lo, hi, growth, nonfood in rules.q(
            "select MinimumAmenityScore, MaximumAmenityScore, GrowthModifier, NonFoodYieldModifier from Happinesses"):
        if (lo is None or score >= lo) and (hi is None or score <= hi):
            return nonfood, growth
    return 0.0, 0.0
