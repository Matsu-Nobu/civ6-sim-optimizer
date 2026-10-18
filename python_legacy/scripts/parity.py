"""Compare the Rust simulator (optimizer run) with the Python one turn by turn on the same plan.

Usage: python3 python_legacy/scripts/parity.py <problem.json> <export prefix> <rules.sqlite> [--turn 30] [--genome recorded|base]
Prints the first turn where they differ and what differs.
"""
import json
import subprocess
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
from civ6sim import opt  # noqa: E402

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def tup(x):
    return tuple(tup(y) for y in x) if isinstance(x, list) else x


def main():
    problem, prefix, db = sys.argv[1], sys.argv[2], sys.argv[3]
    turn, which = arg("--turn", 30), arg("--genome", "recorded", str)
    import opt_experiment as ox
    ox.PREFIX, ox.DB, ox.TURN = prefix, db, turn
    planner, st0 = ox.build()
    rows = [json.loads(l) for l in open(f"{prefix}_p0.jsonl")]
    if which == "recorded":
        g = ox.recorded_genome(planner, st0, rows)
    elif which == "base":
        g = {}
    else:  # a search result: file.json[:name]
        f, _, name = which.partition(":")
        d = json.load(open(f))
        plan = d[name]["plan"] if name else d["plan"]
        g = {tup(k): tup(c) for k, c in plan}
        tmp = ROOT / "data/plans/_parity_genome.json"
        tmp.write_text(json.dumps(plan))
        which = str(tmp)
    import copy
    st = copy.deepcopy(st0)
    py = []
    ch = lambda s, k, o: g[k] if g.get(k) in o else planner.base(s, k, o)
    while st.turn <= turn:
        planner.advance(st, ch)
        py.append({"turn": st.turn - 1, "research": st.research, "civic": st.civic, "gold": st.acc.gold, "faith": st.acc.faith,
                   "output": [st.last_output.get(k, 0) for k in ("YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH")],
                   "cities": [{"name": c["name"], "pop": st.acc.cities[c["name"]].population, "food_store": st.acc.cities[c["name"]].food_store,
                               "item": c["production"]["current"], "worked": [[w["x"], w["y"]] for w in c["worked"]]} for c in st.cities],
                   "log": [(k, ch_) for k, t, ch_ in st.log if t == st.turn - 1]})
    out = subprocess.run([str(BIN), "run", problem, "--turn", str(turn), "--genome", which], capture_output=True, text=True)
    rs = [json.loads(l) for l in out.stdout.splitlines() if l.startswith('{"turn"')]
    for a, b in zip(py, rs):
        diffs = []
        for k in ("research", "civic"):
            if a[k] != b[k]:
                diffs.append(f"{k}: py {a[k]} rs {b[k]}")
        for k in ("gold", "faith"):
            if abs(a[k] - b[k]) > 1e-6:
                diffs.append(f"{k}: py {a[k]:.3f} rs {b[k]:.3f}")
        if any(abs(x - y) > 1e-6 for x, y in zip(a["output"], b["output"])):
            diffs.append(f"output: py {[round(x, 3) for x in a['output']]} rs {[round(x, 3) for x in b['output']]}")
        if len(a["cities"]) != len(b["cities"]):
            diffs.append(f"cities: py {[c['name'] for c in a['cities']]} rs {[c['name'] for c in b['cities']]}")
        for ca, cb in zip(a["cities"], b["cities"]):
            for k in ("name", "pop", "item"):
                if ca[k] != cb[k]:
                    diffs.append(f"{ca['name']} {k}: py {ca[k]} rs {cb[k]}")
            if abs(ca["food_store"] - cb["food_store"]) > 1e-6:
                diffs.append(f"{ca['name']} food_store: py {ca['food_store']:.3f} rs {cb['food_store']:.3f}")
            if ca["worked"] != cb["worked"]:
                diffs.append(f"{ca['name']} worked: py {ca['worked']} rs {cb['worked']}")
        if diffs:
            print(f"first difference at T{a['turn']}:")
            for d in diffs:
                print("  ", d)
            print("   python decisions this turn:", [(list(k), c) for k, c in a["log"]][:12])
            return
    print(f"identical for {min(len(py), len(rs))} turns; total py {sum(opt.objective(dict(zip(('YIELD_FOOD','YIELD_PRODUCTION'), x['output'][:2]))) for x in py):.1f}")


if __name__ == "__main__":
    main()
