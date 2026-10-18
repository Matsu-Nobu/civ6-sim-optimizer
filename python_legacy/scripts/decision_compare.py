"""How much does one decision matter? For each option of a decision in a plan, fix it and re-optimize the rest
(NRPA with the plan as the prior), then compare the best values.

Usage: nice -n 19 python3 python_legacy/scripts/decision_compare.py <search result json> <plan name> <kind> [--turn 60]
         [--n 0] [--entity ""] [--budget 1500] [--workers 12]
  e.g. python_legacy/scripts/decision_compare.py data/search/sum_T60.json nrpa_rec_0 governor --turn 60
"""
import copy
import json
import multiprocessing as mp
import sys
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
sys.path.insert(0, str(Path(__file__).resolve().parent))
import opt_experiment as ox  # noqa: E402  (build() reads --turn)
from civ6sim import opt  # noqa: E402


def tup(x):
    return tuple(tup(y) for y in x) if isinstance(x, list) else x


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


def main():
    path, name, kind = sys.argv[1], sys.argv[2], sys.argv[3]
    turn, n, entity = ox.TURN, arg("--n", 0), arg("--entity", "", str)
    budget, workers = arg("--budget", 1500), arg("--workers", 12)
    planner, st0 = ox.build()
    plan = {tup(k): tup(c) for k, c in json.load(open(path))[name]["plan"]}
    key = (kind, entity, n)
    seen = {}
    st = copy.deepcopy(st0)

    def chooser(s, k, options):
        c = plan[k] if plan.get(k) in options else planner.base(s, k, options)
        if k == key:
            seen.update(turn=s.turn, options=list(options), chosen=c)
        return c

    while st.turn <= turn:
        planner.advance(st, chooser)
    if not seen:
        print("decision", key, "does not occur in the plan")
        return
    print(f"decision {key} at T{seen['turn']}: the plan chose {seen['chosen']}; {len(seen['options'])} options")
    out = []
    with mp.Pool(workers, initializer=opt._winit, initargs=(ox.build,)) as pool:
        for o in seen["options"]:
            cnt = opt.nrpa(pool, turn, budget, lam=workers, seed=0, init=plan, fixed={key: o})
            out.append((cnt.best, o))
            print(f"  {cnt.best:8.1f}  {o}", flush=True)
    print("ranking:")
    for v, o in sorted(out, key=lambda t: -t[0]):
        print(f"  {v:8.1f}  {o}")


if __name__ == "__main__":
    main()
