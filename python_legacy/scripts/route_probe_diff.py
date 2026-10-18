"""Trade routes: the game's own per-route origin yields (probe: route + path + modifiers, as the trade screen adds
them) vs yields.route_yields on the exported state. Usage: route_probe_diff.py <rules> <export> <probe> [--offset N]"""
import json
import sys
from collections import Counter
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.envs import player_of  # noqa: E402
from civ6sim.modifiers import engine_for  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.yields import route_yields  # noqa: E402

YS = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"]
r = Rules(sys.argv[1])
off = int(sys.argv[sys.argv.index("--offset") + 1]) if "--offset" in sys.argv else 0
ex = {json.loads(l)["turn"]: json.loads(l) for l in open(sys.argv[2])}
pid = next(iter(ex.values()))["player"]["playerId"]
ok = bad = 0
pat = Counter()
for line in open(sys.argv[3]):
    pr = json.loads(line)
    if pr.get("player") != pid:
        continue
    e = ex.get(pr["turn"] + off)
    if not e:
        continue
    eng = engine_for(r, player_of(e), e["cities"])
    world = {(c["playerId"], c["id"]): c for c in e.get("worldCities") or []}
    mine = {c["name"]: c for c in e["cities"]}
    for pc in pr["cities"]:
        name = pc["name"].replace("LOC_CITY_NAME_", "")
        city = mine.get(name)
        if city is None:
            continue
        for rt in pc.get("routes") or []:
            game = [0.0] * 6
            for part in ("route", "path", "modifiers"):
                vals = (rt.get("origin") or {}).get(part) or []
                for i, v in enumerate(vals[:6]):
                    game[i] += v or 0
            dkey = (rt["destPlayer"], rt["destCity"])
            dest = world.get(dkey)
            if dest is None:
                d = next((c for c in e["cities"] if c["id"] == rt["destCity"] and rt["destPlayer"] == pid), None)
                if d is None:
                    continue
                dest = {"name": d["name"], "id": d["id"], "playerId": pid, "civilization": e["player"]["civilization"],
                        "x": d["center"]["x"], "y": d["center"]["y"],
                        "districts": [x["type"] for x in d["districts"] if x["complete"]],
                        "plots": [(p["terrain"], p["resource"], p["improvement"]) for p in d["owned"]]}
            route = {"origin": {"name": name, "id": city["id"], "playerId": pid}, "destination": dest}
            if rt.get("path"):  # plot indices the trader walks: foreign city centers on it
                w = e.get("mapWidth") or 84
                on = {(i % w, i // w) for i in rt["path"]}
                route["foreignCitiesOnPath"] = sum(1 for c in e.get("worldCities") or () if (c["x"], c["y"]) in on)
            ours = route_yields(r, eng, route, city)
            diff = {k[6:9]: round(game[i] - ours.get(k, 0), 2) for i, k in enumerate(YS) if abs(game[i] - ours.get(k, 0)) > 0.01}
            if diff:
                bad += 1
                pat[(dest.get("civilization"), tuple(sorted(diff.items())))] += 1
            else:
                ok += 1
print(f"match {ok} / {ok + bad}")
for k, n in pat.most_common(15):
    print(n, k)
