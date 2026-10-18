"""Show a district's neighbours and which adjacency rows count, for checking against the game.
Usage: adjacency_diag.py <rules> <export.jsonl> <turn> <city> <district>"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.envs import player_of  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import _adjacent_match, excluded_adjacencies, modifier_adjacency_rows  # noqa: E402

r = Rules(sys.argv[1])
turn, city, dist = int(sys.argv[3]), sys.argv[4], sys.argv[5]
d = next(json.loads(l) for l in open(sys.argv[2]) if json.loads(l)["turn"] == turn)
pl = player_of(d)
e = engine_for(r, pl, d["cities"])
c = next(x for x in d["cities"] if x["name"] == city)
dd = next(x for x in c["districts"] if x["type"] == dist)
print(dd["x"], dd["y"], dd["plot"]["terrain"], dd["plot"]["feature"], "river" if dd["plot"].get("river") else "")
nb = [e.plots.get((n["x"], n["y"]), n) for n in dd["neighbours"]]
for n in nb:
    print("  ", n["x"], n["y"], n["terrain"], n["feature"], n.get("resource"), n.get("improvement"), n.get("districtType"),
          n.get("districtComplete"), n.get("wonder"))
ex = excluded_adjacencies(r, pl)
for row in list(r.district_adjacencies(dist)) + modifier_adjacency_rows(e, city, dist):
    cnt = 1 if row["AdjacentRiver"] and dd["plot"].get("river") else sum(1 for n in nb if _adjacent_match(r, row, n))
    if cnt:
        print("  row", row["ID"], row["YieldChange"], "per", row["TilesRequired"], "count", cnt, "EXCLUDED" if row["ID"] in ex else "")
