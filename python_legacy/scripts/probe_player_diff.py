"""Player-level checks against the extended probe: maintenance (the treasury's own total and split) and great person
points per turn (the game's live values). Usage: probe_player_diff.py <rules> <export> <probe> [--offset N]"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.envs import player_of  # noqa: E402
from civ6sim.greatpeople import great_person_points  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.production import maintenance  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402

r = Rules(sys.argv[1])
off = int(sys.argv[sys.argv.index("--offset") + 1]) if "--offset" in sys.argv else 0
ex = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[2])}
pid = next(iter(ex.values()))["player"]["playerId"]
m_ok = m_bad = g_ok = g_bad = 0
mpat, gpat = Counter(), Counter()
for line in open(sys.argv[3]):
    pr = json.loads(line)
    if pr.get("player") != pid or "treasury" not in pr:
        continue
    e = ex.get(pr["turn"] + off)
    if not e:
        continue
    eng = engine_for(r, player_of(e), e["cities"])
    t = pr["treasury"]
    ours = maintenance(r, eng, e["cities"], [u["type"] for u in e["units"]])
    if t.get("totalMaintenance") is not None:
        if abs(t["totalMaintenance"] - ours) < 0.01:
            m_ok += 1
        else:
            m_bad += 1
            mpat[(round(t["totalMaintenance"] - ours, 2), t.get("buildingMaintenance"), t.get("districtMaintenance"),
                  t.get("unitMaintenance"))] += 1
    game = {k: v[0] for k, v in (pr.get("greatPersonPoints") or {}).items() if v and v[0]}
    mine = {k: v for k, v in great_person_points(r, eng, e["cities"]).items() if v}
    d = {k[20:]: round(game.get(k, 0) - mine.get(k, 0), 2) for k in set(game) | set(mine)
         if abs(game.get(k, 0) - mine.get(k, 0)) > 0.01}
    if d:
        g_bad += 1
        gpat[tuple(sorted(d.items()))] += 1
    else:
        g_ok += 1
print(f"maintenance match {m_ok} / {m_ok + m_bad}")
for k, n in mpat.most_common(8):
    print("  ", n, "game-ours, building, district, unit:", k)
print(f"great person points match {g_ok} / {g_ok + g_bad}")
for k, n in gpat.most_common(8):
    print("  ", n, k)
