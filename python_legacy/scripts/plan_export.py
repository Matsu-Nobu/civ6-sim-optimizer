"""Run the planner on a recorded start and write the plan for the game to replay (Civ6EconPlan.lua) plus the
simulator's trace (expected per-turn state) for python_legacy/scripts/plan_diff.py.

Usage: python3 python_legacy/scripts/plan_export.py <export prefix> <rules.sqlite> [--start 2] [--horizon 31] [--plan plan.json]
  e.g. python3 python_legacy/scripts/plan_export.py data/exports/game_rome2 data/rules/gameplay_rome2.sqlite --horizon 31
Writes data/plans/<name>_trace.json and mods/Civ6EconAutoplay/Civ6EconPlan.lua.
"""
import json
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim.plan2 import FullPlanner, Scenario, Settings  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.world import Map, World  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def lua(v):
    if v is None:
        return "nil"
    if isinstance(v, bool):
        return "true" if v else "false"
    if isinstance(v, (int, float)):
        return repr(v)
    if isinstance(v, str):
        return json.dumps(v)
    if isinstance(v, dict):
        return "{" + ", ".join(f"[{lua(k)}] = {lua(x)}" for k, x in v.items()) + "}"
    return "{" + ", ".join(lua(x) for x in v) + "}"


def game_plan(trace):
    """Per turn, what the game must do: player choices, each city's item (with its plot) and worked plots,
    purchases, and unit orders (builders and settlers by the order they appear, matched in the game by place)."""
    turns = {}

    def at(t):
        return turns.setdefault(t, {"cities": [], "builders": [], "settlers": [], "buys": [], "governors": [],
                                    "routes": []})

    items = {}  # city -> (item, site), carried until changed
    for ev in trace:
        kind, t = ev[0], ev[1]
        if kind == "prod":
            items[ev[2]] = (ev[3], ev[4])
        elif kind == "turn":
            s = ev[2]
            d = at(t)
            d.update(research=s["research"], civic=s["civic"], government=s["government"], policies=s["policies"])
            for c in s["cities"]:
                item, site = items.get(c["name"], (None, None))
                d["cities"].append({"x": c["xy"][0], "y": c["xy"][1], "item": item,
                                    "sx": site[0] if site else None, "sy": site[1] if site else None,
                                    "worked": [list(w) for w in c["worked"]]})
        elif kind == "builder_new":
            at(t)["builders"].append({"op": "new", "id": ev[2], "x": ev[3][0], "y": ev[3][1]})
        elif kind == "builder_go":
            act = ev[3]
            at(t)["builders"].append({"op": "go", "id": ev[2], "x": act[2][0], "y": act[2][1]})
        elif kind == "builder_do":
            act = ev[3]
            at(t)["builders"].append({"op": act[0], "id": ev[2], "x": act[2][0], "y": act[2][1],
                                      "what": act[3] if isinstance(act[3], str) else None})
        elif kind == "settler":
            # the settler is built at the end of turn t and moves from turn t+1
            at(t + 1)["settlers"].append({"op": "go", "id": ev[2], "x": ev[4][0], "y": ev[4][1], "arrive": ev[5]})
        elif kind == "found":
            at(t)["settlers"].append({"op": "found", "id": ev[4], "x": ev[3][0], "y": ev[3][1]})
        elif kind == "buy":
            b = ev[2:]
            if b[0] == "tile":
                at(t)["buys"].append({"kind": "tile", "x": b[3][0], "y": b[3][1]})
            else:
                at(t)["buys"].append({"kind": "item", "city": b[1], "item": b[2],
                                      "yield": b[4] if len(b) > 4 else "YIELD_GOLD"})
        elif kind == "pantheon":
            at(t)["pantheon"] = ev[2]
        elif kind == "governor":
            at(t)["governors"].append(list(ev[2:]))
        elif kind == "trade":
            at(t)["routes"].append({"origin": ev[2], "dest": ev[3]})
    return turns


def observed_borders(probe_path):
    """(city centre, n) -> the n-th plot the city's border took in a game run (probe rows of the player); n counts
    from 0 for the first plot beyond the founding tiles, as State.border does."""
    seen, out = {}, {}
    for line in open(probe_path):
        r = json.loads(line)
        for c in r["cities"]:
            key = (c["x"], c["y"])
            tiles = {(t[0], t[1]) for t in c["tiles"]}
            if key not in seen:
                seen[key] = tiles
                continue
            for xy in sorted(tiles - seen[key]):
                n = sum(1 for k in out if k[0] == key)
                out[(key, n)] = xy
            seen[key] |= tiles
    return out


def main():
    prefix, db = sys.argv[1], sys.argv[2]
    start, horizon = arg("--start", 2), arg("--horizon", 31)
    rules = Rules(db)
    rows = [json.loads(line) for line in open(f"{prefix}_p0.jsonl")]
    game_map = Map(json.load(open(f"{prefix}_map_T{start}.json")))
    borders = observed_borders(arg("--borders", None, str)) if "--borders" in sys.argv else None
    planner = FullPlanner(rules, World(rules, game_map), Scenario(rows, rules), Settings(horizon=horizon, borders=borders))
    st0 = planner.initial_state(start)
    st0.trace = []
    plan = {}
    if "--plan" in sys.argv:
        plan = {tuple(k): v for k, v in json.load(open(arg("--plan", None, str)))}
    st, value = planner.run(st0, plan)
    name = Path(prefix).name.replace("game_", "")
    out = ROOT / "data/plans"
    out.mkdir(exist_ok=True)
    (out / f"{name}_trace.json").write_text(json.dumps(st.trace))
    # city names in the plan are the simulator's; the game finds a city by its centre plot
    centers = {ev[2]["cities"][i]["name"]: ev[2]["cities"][i]["xy"] for ev in st.trace if ev[0] == "turn"
               for i in range(len(ev[2]["cities"]))}
    # A save loaded at the start of a turn gives the human player nothing for that turn: game turn t+offset plays
    # the simulator's turn t (offset 1, measured on the rs2 runs: every city value matched one turn later).
    offset = arg("--game-offset", 1)
    turns = {t + offset: d for t, d in game_plan(st.trace).items()}
    for d in turns.values():
        for s in d["settlers"]:
            if "arrive" in s:
                s["arrive"] += offset
    for d in turns.values():
        for b in d["buys"]:
            if b["kind"] == "item":
                b["x"], b["y"] = centers[b.pop("city")]
        for r in d["routes"]:
            r["ox"], r["oy"] = centers[r.pop("origin")]
            dest = r.pop("dest")
            if dest in centers:
                r["dx"], r["dy"] = centers[dest]
            else:
                r["dest"] = dest
        for g in d["governors"]:
            if g[0] == "appoint":
                g[2] = centers[g[2]]
    text = (f"-- Written by python_legacy/scripts/plan_export.py from {prefix} (start {start}, horizon {horizon}). Do not edit.\n"
            f"ECONPLAN = {{ START = {start + offset}, LAST = {horizon + offset}, OFFSET = {offset}, TURNS = {lua(turns)} }}\n")
    (ROOT / "mods/Civ6EconAutoplay/Civ6EconPlan.lua").write_text(text)
    print(f"value {value:.1f}; {len(st.cities)} cities; trace {len(st.trace)} events; plan turns {min(turns)}..{max(turns)}")


if __name__ == "__main__":
    main()
