"""Player-level faith (outside cities): the game's faith stock change minus its cities' faith, against
yields.player_yields. Turns with spending (big drops) are skipped.
Usage: python3 python_legacy/scripts/player_yield_diff.py <rules> <export.jsonl> <probe.jsonl> [--offset N]"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import player_yields  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

r = Rules(sys.argv[1])
off = int(sys.argv[sys.argv.index("--offset") + 1]) if "--offset" in sys.argv else 0
ex = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[2])}
pr = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[3])}
ok = bad = 0
for t in sorted(pr):
    if t + 1 not in pr or t + off not in ex:
        continue
    game = pr[t + 1]["faith"] - pr[t]["faith"] - sum(c["yields"].get("YIELD_FAITH", 0) for c in pr[t]["cities"])
    if game < -1:
        continue  # faith was spent
    e = ex[t + off]
    ours = player_yields(r, engine_for(r, player_of(e), e["cities"])).get("YIELD_FAITH", 0)
    if abs(game - ours) < 0.02:
        ok += 1
    else:
        bad += 1
        print(t, "game", round(game, 3), "ours", round(ours, 3))
print(f"match {ok} / {ok + bad}")
