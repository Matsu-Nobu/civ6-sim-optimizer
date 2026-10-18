"""Compare the non-tile part of city yields with the game: game city yield / (1 + our %) - game worked-tile sum,
against our non-tile breakdown. Separates tile errors from building/district/modifier/% errors.

Usage: python3 python_legacy/scripts/nontile_diff.py <rules> <export.jsonl> <probe.jsonl> [--offset N] [--ai] [--city NAME] [--turn T]
"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.amenities import amenity_scores, happiness  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.production import item_production_flat  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_yield_percents, city_yields, faith_conversions, percent_modifiers  # noqa: E402
from civ6sim.envs import player_of, routes_from_probe  # noqa: E402

SH = {"YIELD_FOOD": "F", "YIELD_PRODUCTION": "P", "YIELD_GOLD": "G", "YIELD_SCIENCE": "S", "YIELD_CULTURE": "C", "YIELD_FAITH": "R"}


def main():
    rules = Rules(sys.argv[1])
    args = sys.argv[4:]
    off = int(args[args.index("--offset") + 1]) if "--offset" in args else 0
    only_city = args[args.index("--city") + 1] if "--city" in args else None
    only_turn = int(args[args.index("--turn") + 1]) if "--turn" in args else None
    rows = {d["turn"]: d for d in map(json.loads, open(sys.argv[2]))}
    agg = Counter()
    for line in open(sys.argv[3]):
        pr = json.loads(line)
        d = rows.get(pr["turn"] + off)
        if not d or (only_turn and pr["turn"] != only_turn):
            continue
        player = player_of(d)
        by_name = {c["name"]: c for c in d["cities"]}
        scores = amenity_scores(rules, d["cities"], player)
        eng = engine_for(rules, player, d["cities"])
        for pc in pr["cities"]:
            c = next((x for x in d["cities"] if (x["center"]["x"], x["center"]["y"]) == (pc["x"], pc["y"])), None)
            if not c or (only_city and c["name"] != only_city):
                continue
            # buildings the game reports with no yields at all while they normally have some are pillaged (war, outside
            # the model): leave them out
            by = pc.get("buildingYields") or {}
            pill = [b for b in c["buildings"] if b in by and not any(by[b].values()) and any(rules.building_yields[b].values())]
            if pill:
                c = dict(c, buildings=[b for b in c["buildings"] if b not in pill])
                by_name = dict(by_name, **{c["name"]: c})
            total, parts = city_yields(rules, c, player, routes=routes_from_probe(pr, d), cities_by_name=by_name)
            score = pc["amenities"] - pc["amenitiesNeeded"] if "--game-amenities" in args else scores[c["name"]]
            pct = percent_modifiers(happiness(rules, score)[0], "--ai" in args, city_yield_percents(rules, eng, c, score))
            # flat production toward the current item (e.g. "+2 toward units") shows in the city's yield
            item = (c.get("production") or {}).get("current")
            if item:
                flat = item_production_flat(rules, eng, c, item)
                if flat:
                    parts["production toward " + item] = {"YIELD_PRODUCTION": flat}
            ours_nontile = {k: sum(v.get(k, 0) for lab, v in parts.items() if lab != "center" and not lab.startswith("tile(")) for k in SH}
            game_tiles = {k: sum(t[2].get(k, 0) * t[3] for t in pc["tiles"] if t[3]) for k in SH}
            # yields granted as a % of the city's faith (Menelik, Moksha) come on top of the percents: take them off
            # the game's yield first
            conv = faith_conversions(eng, c, {"YIELD_FAITH": pc["yields"]["YIELD_FAITH"]})
            diff = {}
            for k in SH:
                g = (pc["yields"][k] - conv.get(k, 0)) / (1 + pct[k] / 100) - game_tiles[k]
                dv = round(g - ours_nontile[k], 2)
                if abs(dv) > 0.05:
                    diff[SH[k]] = dv
            key = (c["name"], tuple(sorted(diff.items())))
            agg[key] += 1
            if only_city or only_turn:
                print(pr["turn"], c["name"], "game-ours nontile", diff)
                if only_turn:
                    for lab, v in parts.items():
                        if lab != "center" and not lab.startswith("tile("):
                            print("    ", lab, {SH[k]: round(x, 2) for k, x in v.items() if x})
    if not (only_city or only_turn):
        for (name, diff), n in agg.most_common(30):
            print(f"{n:4d} {name:22s} {dict(diff)}")


if __name__ == "__main__":
    main()
