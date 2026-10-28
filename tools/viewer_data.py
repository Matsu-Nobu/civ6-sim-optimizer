"""Writes the data of the replay viewer (viewer/): the map of a game data file and the games played on it (the record
or plans), turn by turn, as viewer/public/data/<name>.json, and adds it to viewer/public/data/index.json.

  viewer_data.py <name> <game data .json> <label>=<plan .json or "record"> [<label>=<plan> ...] -- [civ6opt options]

e.g. viewer_data.py rome21 data/games/rome_20261021.json あなた=record 探索=data/search/learn/rome21_best.json -- --turn 65
"""
import json
import sys
from pathlib import Path

import civ6opt
from plan_report import decisions_by_turn, ja, snapshots

OUT = civ6opt.ROOT / "viewer" / "public" / "data"


def play(game, plan, opts):
    decisions = civ6opt.run(["decisions", game, *opts, "--genome", plan], capture_output=True, text=True, check=True).stdout
    states = civ6opt.run(["run", game, *opts, "--genome", plan], capture_output=True, text=True, check=True).stdout
    snaps, final = snapshots(states)
    turns = []
    for t, s in sorted(snaps.items()):
        cities = []
        for c in s["cities"]:
            districts = []
            for d in c["districts"]:
                kind, at = d.split("@")
                xy, _, wonder = at.partition(":")
                x, y = map(int, xy.split(","))
                districts.append({"type": kind.rstrip("*"), "done": not kind.endswith("*"), "x": x, "y": y,
                                  "wonder": wonder or None})
            cities.append({"name": c["name"], "xy": c["xy"], "pop": c["pop"], "item": c["item"], "worked": c["worked"],
                           "yields": c["yields"], "buildings": c["buildings"], "districts": districts,
                           "tiles": c["tiles"]})
        turns.append({"turn": t, "output": s["output"], "gold": s["gold"], "techs": s["techs"], "civics": s["civics"],
                      "research": s["research"], "civic": s["civic"], "government": s["government"],
                      "policies": s["policies"], "governors": s["governors"], "units": s["units_trained"],
                      "builders": s["builder_at"], "settlers": s["settlers"], "strategic": s["strategic"],
                      "cities": cities})
    return {"turns": turns, "decisions": decisions_by_turn(decisions), "score": final["score"], "total": final["total"]}


def strings(x):
    if isinstance(x, str):
        yield x
    elif isinstance(x, list):
        for v in x:
            yield from strings(v)
    elif isinstance(x, dict):
        for v in x.values():
            yield from strings(v)


def main():
    if "--" not in sys.argv or len(sys.argv) < 5:
        sys.exit(__doc__)
    i = sys.argv.index("--")
    name, game, named, opts = sys.argv[1], sys.argv[2], sys.argv[3:i], sys.argv[i + 1:]
    g = json.loads(Path(game).read_text())
    m = g["map"]
    res = g["rules"]["Resources"]
    col = res["cols"].index("ResourceClassType")
    classes = {r[0]: r[col].replace("RESOURCECLASS_", "").lower() for r in res["rows"]}
    plots = [[p["x"], p["y"], p["terrain"], p["feature"], p["resource"], p["riverEdges"] if p["river"] else 0]
             for p in m["plots"]]
    plays = []
    for item in named:
        label, plan = item.split("=", 1)
        plays.append({"label": label, "plan": plan, **play(game, plan, opts)})
        print(label, f"score {plays[-1]['score']:.0f}", file=sys.stderr)
    # the Japanese names of the types and cities the data uses ("UNIT_KNIGHT:3" counts too)
    used = {s.split(":")[0] for s in strings([plots, [p["turns"] for p in plays]])}
    names = {k: ja(k) for k in sorted(used) if ja(k) != k}
    # a city a plan founds where the record has none is named CITY_<founding order> (planner.rs)
    names |= {k: f"新しい都市 {k[5:]}" for k in used if k.startswith("CITY_") and k[5:].isdigit()}
    data = {"name": name, "game": game, "options": opts, "width": m["width"], "height": m["height"], "plots": plots,
            "plays": plays, "names": names,
            "resource_classes": {k: v for k, v in classes.items() if k in used}}
    OUT.mkdir(parents=True, exist_ok=True)
    (OUT / f"{name}.json").write_text(json.dumps(data, ensure_ascii=False, separators=(",", ":")))
    index_path = OUT / "index.json"
    index = json.loads(index_path.read_text()) if index_path.exists() else []
    index = [x for x in index if x["name"] != name] + [{"name": name, "plays": [p["label"] for p in plays]}]
    index_path.write_text(json.dumps(index, ensure_ascii=False, indent=1))
    print("wrote", OUT / f"{name}.json", file=sys.stderr)


if __name__ == "__main__":
    main()
