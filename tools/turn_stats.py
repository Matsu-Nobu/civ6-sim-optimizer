"""Writes a table (TSV) of the players' yields and counts from exported save rows (civ6opt export), one line per
player and save: game, player id, civilization, the turn the save ends (its turn - 1), food, production, science,
culture, gold, faith per turn, cities, population, techs, civics, game score, the government plaza building
(BUILDING_GOV_*, the first one built), military units alive and their summed strength (each unit's melee or ranged
combat strength from the rules database, whichever is larger; units without either are not military).

  turn_stats.py <exports dir> <out.tsv> [--majors] [--rules <rules.sqlite>]

The files of the exports dir are named <game>_p<player id>.jsonl; --majors leaves out city-states, barbarians and
free cities.
"""
import json
import sqlite3
import sys
from pathlib import Path

src, out = Path(sys.argv[1]), Path(sys.argv[2])
majors = "--majors" in sys.argv
rules = sys.argv[sys.argv.index("--rules") + 1] if "--rules" in sys.argv else str(Path(__file__).resolve().parents[1] / "data/rules/gameplay_2026-10-19.sqlite")
strength = {u: max(c or 0, r or 0) for u, c, r in sqlite3.connect(rules).execute("select UnitType, Combat, RangedCombat from Units")}
cols = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_GOLD", "YIELD_FAITH"]
lines = ["game\tplayer\tcivilization\tturn_end\tfood\tproduction\tscience\tculture\tgold\tfaith\tcities\tpopulation\ttechs\tcivics\tscore\tgov_building\tmilitary_units\tmilitary_strength"]
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
        gov = next((b for c in r["cities"] for b in c["buildings"] if b.startswith("BUILDING_GOV_")), "")
        military = [strength[u["type"]] for u in r["units"] if strength.get(u["type"], 0) > 0]
        lines.append("\t".join(str(v) for v in [game, pid, p["civilization"], r["turn"] - 1]
                               + [round(y.get(c, 0), 1) for c in cols]
                               + [len(r["cities"]), sum(c["population"] for c in r["cities"]), len(p["techs"]), len(p["civics"]),
                                  p.get("score"), gov, len(military), sum(military)]))
out.write_text("\n".join(lines) + "\n")
print(f"{len(lines) - 1} rows -> {out}")
