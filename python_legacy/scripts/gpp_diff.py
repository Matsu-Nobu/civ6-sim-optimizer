"""Great person points per turn: model vs the save. Usage: python3 python_legacy/scripts/gpp_diff.py <rules> <export.jsonl>..."""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.greatpeople import great_person_points  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.envs import player_of  # noqa: E402

r = Rules(sys.argv[1])
ok = bad = 0
cnt, ex = Counter(), []
for f in sys.argv[2:]:
    for line in open(f):
        a = json.loads(line)
        game = {k: v["perTurn"] for k, v in a["player"]["greatPersonPoints"].items()}
        ours = great_person_points(r, engine_for(r, player_of(a), a["cities"]), a["cities"])
        diff = {k[20:]: round(ours.get(k, 0) - game.get(k, 0), 2) for k in set(ours) | set(game)
                if abs(ours.get(k, 0) - game.get(k, 0)) > 0.01}
        if diff:
            bad += 1
            cnt[tuple(sorted(diff.items()))] += 1
            ex.append((Path(f).stem, a["turn"], diff))
        else:
            ok += 1
print(f"match {ok} / {ok + bad}")
for k, n in cnt.most_common(10):
    print(n, k)
for e in ex[:15]:
    print(e)
