"""Gold upkeep: the gold stock change between probes minus the cities' gold, vs production.maintenance. Turns with
purchases (big drops) are skipped. Usage: maintenance_diff.py <rules> <export> <probe> [--offset N]"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.production import maintenance  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

r = Rules(sys.argv[1])
off = int(sys.argv[sys.argv.index("--offset") + 1]) if "--offset" in sys.argv else 0
ex = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[2])}
pr = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[3])}
c = Counter()
for t in sorted(pr):
    e, p, p2 = ex.get(t + off), pr[t], pr.get(t + 1)
    if not e or not p2:
        continue
    cg = sum(x["yields"].get("YIELD_GOLD", 0) for x in p["cities"])
    game = cg - (p2["gold"] - p["gold"])
    if game > 15 or game < -1:
        continue  # purchase or windfall
    ours = maintenance(r, engine_for(r, player_of(e), e["cities"]), e["cities"],
                       [u["type"] for u in e["units"]])
    ok = abs(game - ours) < 0.05
    c[ok] += 1
    if not ok:
        print(t, "game", round(game, 2), "ours", ours)
print("match", c[True], "/", c[True] + c[False])
