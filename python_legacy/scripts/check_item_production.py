"""Production toward the current item: progress(T+1) - progress(T) in the saves vs the game's city production at T
(probe) scaled by the item's own percent: game_P * (1 + (g + i)/100) / (1 + g/100), g = general %, i = item %.
Usage: check_item_production.py <rules> <export> <probe> [--ai] [--offset N]"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.amenities import happiness  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.production import item_production_percent  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yield_percents, percent_modifiers  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402


def main():
    r = Rules(sys.argv[1])
    args = sys.argv[4:]
    off = int(args[args.index("--offset") + 1]) if "--offset" in args else 0
    rows = {d["turn"]: d for d in map(json.loads, open(sys.argv[2]))}
    stats, bad = Counter(), []
    for line in open(sys.argv[3]):
        pr = json.loads(line)
        d, n = rows.get(pr["turn"] + off), rows.get(pr["turn"] + off + 1)
        if not d or not n:
            continue
        player = player_of(d)
        eng = engine_for(r, player, d["cities"])
        for pc in pr["cities"]:
            c = next((x for x in d["cities"] if (x["center"]["x"], x["center"]["y"]) == (pc["x"], pc["y"])), None)
            c2 = next((x for x in n["cities"] if c and x["name"] == c["name"]), None)
            if not c or not c2:
                continue
            item = c["production"]["current"]
            if not item or c2["production"]["current"] != item:
                continue
            prev = rows.get(pr["turn"] + off - 1)
            cp = next((x for x in (prev or {}).get("cities", []) if x["name"] == c["name"]), None)
            if cp is None or cp["production"]["current"] != item:
                stats["skipped: item just started (overflow)"] += 1
                continue
            # skip turns with a harvest or chop in the city (feature or resource gone): it adds production
            before = {(q["x"], q["y"]): (q["feature"], q["resource"]) for q in c["owned"]}
            if any(before.get((q["x"], q["y"]), (None, None)) != (q["feature"], q["resource"])
                   and (before[(q["x"], q["y"])][0] and not q["feature"] or before[(q["x"], q["y"])][1] and not q["resource"])
                   for q in c2["owned"] if (q["x"], q["y"]) in before):
                stats["skipped: harvest"] += 1
                continue
            p0, p1 = c["production"]["progress"].get(item, 0), c2["production"]["progress"].get(item, 0)
            if p1 < p0:
                continue
            # the AI difficulty bonus multiplies separately: it is not added to the item's percent
            g = percent_modifiers(happiness(r, pc["amenities"] - pc["amenitiesNeeded"])[0], False,
                                  city_yield_percents(r, eng, c))["YIELD_PRODUCTION"]
            i = item_production_percent(r, eng, c, item)
            pred = pc["yields"]["YIELD_PRODUCTION"] * (1 + (g + i) / 100) / (1 + g / 100)
            ok = abs((p1 - p0) - pred) < 0.06
            key = ("with item %" if i else "no item %", ok)
            stats[key] += 1
            if not ok:
                bad.append((pr["turn"], c["name"], item, round(p1 - p0, 2), round(pred, 2), i, g))
    for k, v in sorted(stats.items(), key=str):
        print(k, v)
    for b in bad[:25]:
        print("  T%d %s %s observed %.2f predicted %.2f (item %% %g, general %% %g)" % b)


if __name__ == "__main__":
    main()
