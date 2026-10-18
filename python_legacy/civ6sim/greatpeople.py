"""Great person points per turn. Who recruits which great person is outside the model, but the points matter for
the economy in one way: once no great prophet is on offer, prophet points are paid out as faith
(FAITH_PER_UNUSED_GREAT_PERSON_POINT)."""
import math
from collections import defaultdict


def great_person_points(rules, engine, cities):
    """{GREAT_PERSON_CLASS_*: points per turn}: buildings and completed districts (Building_ / District_
    GreatPersonPoints), modifier points on districts and cities, each city's total raised by the city, class and
    government percents (Classical Republic +15%) and cut to 1/256, then the player's flat points.
    Checked against the save's per-turn points (python_legacy/scripts/gpp_diff.py)."""
    total = defaultdict(float)
    if engine.player_effects("EFFECT_ADJUST_NO_GREAT_PERSON_POINTS"):
        return total
    gov = government_bonus(engine, "GOVERNMENTBONUS_GREAT_PEOPLE")
    cls_pct = defaultdict(float)
    for a, mid, src in engine.player_effects("EFFECT_ADJUST_GREAT_PERSON_POINTS_PERCENT"):
        cls_pct[a["GreatPersonClassType"]] += float(a["Amount"])
    for c in cities:
        pts = defaultdict(float)
        for b in c["buildings"]:
            for cls, n in rules.q("select GreatPersonClassType, PointsPerTurn from Building_GreatPersonPoints where BuildingType=?", b):
                pts[cls] += n
        for d in c["districts"]:
            if d["complete"]:
                for cls, n in rules.q("select GreatPersonClassType, PointsPerTurn from District_GreatPersonPoints where DistrictType=?", d["type"]):
                    pts[cls] += n
        for d, a, mid, src in engine.district_effects("EFFECT_ADJUST_DISTRICT_GREAT_PERSON_POINTS", c["name"]):
            if d is None or d.get("complete", True):
                pts[a["GreatPersonClassType"]] += float(a["Amount"])
        for a, mid, src in engine.city_effects("EFFECT_ADJUST_GREAT_PERSON_POINTS", c["name"]):
            pts[a["GreatPersonClassType"]] += float(a["Amount"])
        pct = sum(float(a["Amount"]) for a, m, s in engine.city_effects("EFFECT_ADJUST_CITY_GREAT_PERSON_POINTS_MODIFIER", c["name"]))
        for cls, v in pts.items():
            total[cls] += math.floor(v * (1 + (pct + gov + cls_pct[cls]) / 100) * 256) / 256
    flat = defaultdict(float)
    offer = engine.player.get("greatPeopleOnOffer")
    for a, mid, src in engine.player_effects("EFFECT_ADJUST_GREAT_PERSON_POINTS"):
        if offer is not None and a["GreatPersonClassType"] not in offer:
            continue  # the game turns the player's own points of a class off while nobody of it is on offer
        flat[a["GreatPersonClassType"]] += float(a["Amount"])
    for cls, v in flat.items():  # the player's own points (Revelation) get the same percents
        total[cls] += math.floor(v * (1 + (gov + cls_pct[cls]) / 100) * 256) / 256
    return total


def government_bonus(engine, bonus_type):
    """% from the government's bonus (EFFECT_ADJUST_FLAT_BONUS: Classical Republic great people, Autocracy
    wonders, Merchant Republic districts, Theocracy faith purchases, Democracy gold purchases, ...)."""
    return sum(float(a["Amount"]) for a, m, s in engine.player_effects("EFFECT_ADJUST_FLAT_BONUS")
               if a.get("BonusType") == bonus_type)


def prophet_faith(rules, engine, cities):
    """Faith from prophet points once the player can't earn a great prophet any more: after recruiting one (one per
    player), or when none is left on offer. Points still bank, and are also paid 1:1 as faith
    (FAITH_PER_UNUSED_GREAT_PERSON_POINT); seen on all three AI players of game_auto2 from the turn after their
    prophet."""
    pl = engine.player
    if pl.get("prophetRecruitedTurn") is None and pl.get("prophetOnOffer", True):
        return 0.0
    return great_person_points(rules, engine, cities).get("GREAT_PERSON_CLASS_PROPHET", 0.0) * \
        float(rules.param("FAITH_PER_UNUSED_GREAT_PERSON_POINT"))
