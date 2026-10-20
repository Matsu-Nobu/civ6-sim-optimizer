"""Writes a table (TSV) of the players' yields and counts from exported save rows (civ6opt export), one line per
player and save: game, player id, civilization, the turn the save ends (its turn - 1), food, production, science,
culture, gold, faith per turn, cities, population, techs, civics, game score.

  turn_stats.py <exports dir> <out.tsv> [--majors]

The files of the exports dir are named <game>_p<player id>.jsonl; --majors leaves out city-states, barbarians and
free cities.
"""
import json
import sys
from pathlib import Path

src, out = Path(sys.argv[1]), Path(sys.argv[2])
majors = "--majors" in sys.argv
cols = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_GOLD", "YIELD_FAITH"]
lines = ["game\tplayer\tcivilization\tturn_end\tfood\tproduction\tscience\tculture\tgold\tfaith\tcities\tpopulation\ttechs\tcivics\tscore"]
for f in sorted(src.glob("*_p*.jsonl")):
    game, pid = f.stem.rsplit("_p", 1)
    for line in f.read_text().splitlines():
        if not line.strip():
            continue
        r = json.loads(line)
        p = r["player"]
        if majors and ("MINOR" in p["leader"] or "BARBARIAN" in p["leader"] or "FREE_CITIES" in p["civilization"]):
            continue
        y = p["yields"]
        lines.append("\t".join(str(v) for v in [game, pid, p["civilization"], r["turn"] - 1]
                               + [round(y.get(c, 0), 1) for c in cols]
                               + [len(r["cities"]), sum(c["population"] for c in r["cities"]), len(p["techs"]), len(p["civics"]),
                                  p.get("score")]))
out.write_text("\n".join(lines) + "\n")
print(f"{len(lines) - 1} rows -> {out}")
