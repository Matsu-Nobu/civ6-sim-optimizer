"""Aggregate tile-yield differences (game - ours) by tile kind across turns, most frequent first.

Usage: python3 python_legacy/scripts/tile_diff_summary.py <rules.sqlite> <export.jsonl>:<probe.jsonl>[:offset] ...
"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import city_plot_modifiers, plots_by_xy_of, tile_yields  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

SH = {"YIELD_FOOD": "F", "YIELD_PRODUCTION": "P", "YIELD_GOLD": "G", "YIELD_SCIENCE": "S", "YIELD_CULTURE": "C", "YIELD_FAITH": "R"}


def main():
    r = Rules(sys.argv[1])
    agg = Counter()
    example = {}
    total = 0
    for spec in sys.argv[2:]:
        parts = spec.split(":")
        export, probe = parts[0], parts[1]
        off = int(parts[2]) if len(parts) > 2 else 0
        rows = {d["turn"]: d for d in (json.loads(l) for l in open(export))}
        civ = None
        for line in open(probe):
            pr = json.loads(line)
            d = rows.get(pr["turn"] + off)
            if not d:
                continue
            civ = d["player"]["civilization"]
            player = player_of(d)
            techs, civics = set(player["techs"]), set(player["civics"])
            all_plots = plots_by_xy_of(d["cities"])
            for pc in pr["cities"]:
                c = next((x for x in d["cities"] if (x["center"]["x"], x["center"]["y"]) == (pc["x"], pc["y"])), None)
                if not c:
                    continue
                mods = city_plot_modifiers(r, player, c, d["cities"])
                owned = {(p["x"], p["y"]): p for p in c["owned"]}
                for x, y, ty, workers in pc["tiles"]:
                    p = owned.get((x, y))
                    is_c = (x, y) == (pc["x"], pc["y"])
                    if not p or (p.get("district") and not is_c):
                        continue  # district plots: the game shows the district's adjacency there, checked as city yields
                    p = all_plots.get((x, y), p)
                    total += 1
                    ours = tile_yields(r, p, techs, civics, city_center=is_c, bbcc=c["capital"] and is_c,
                                       plot_modifiers=mods, plots_by_xy=all_plots, city=c)
                    diff = tuple(sorted((SH[k], round(v - ours.get(k, 0), 2)) for k, v in ty.items() if abs(v - ours.get(k, 0)) > 0.01))
                    if not diff:
                        continue
                    kind = (civ[13:], "CENTER" if is_c else "district" if p.get("district") else "",
                            p["terrain"][8:], (p["feature"] or "")[8:], (p["resource"] or "")[9:], (p["improvement"] or "")[12:])
                    agg[(kind, diff)] += 1
                    example.setdefault((kind, diff), (pr["turn"], x, y))
    print(f"tiles checked {total}, differing {sum(agg.values())}")
    for (kind, diff), n in agg.most_common(45):
        print(f"{n:5d}  {'/'.join(k for k in kind if k):55s} {dict(diff)}  e.g. T{example[(kind, diff)][0]} {example[(kind, diff)][1:]}")


if __name__ == "__main__":
    main()
