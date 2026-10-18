"""Compare search methods on one objective at equal rollout budgets, several seeds each.

Usage: nice -n 19 python3 python_legacy/scripts/opt_experiment.py [--turn 30] [--budget 2000] [--seeds 3] [--workers 4]
         [--methods nrpa,es] [--out data/search/opt_T30.json]
Prints the baselines (base policy; the recorded game played in the model, when it reaches the turn), then the best
value per method and seed, and saves the improvement curves.
"""
import json
import multiprocessing as mp
import sys
import time
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent.parent))
from civ6sim import opt  # noqa: E402
from civ6sim.plan2 import FullPlanner, Scenario, Settings, recorded_choices, run_recorded  # noqa: E402
from civ6sim.rules import Rules  # noqa: E402
from civ6sim.world import Map, World  # noqa: E402

PREFIX, DB = "data/exports/game_20261005", "data/rules/gameplay_2026-10-05.sqlite"


def arg(name, default, cast=int):
    return cast(sys.argv[sys.argv.index(name) + 1]) if name in sys.argv else default


TURN = arg("--turn", 30)


def build():
    rules = Rules(DB)
    rows = [json.loads(line) for line in open(f"{PREFIX}_p0.jsonl")]
    planner = FullPlanner(rules, World(rules, Map(json.load(open(f"{PREFIX}_map_T2.json")))), Scenario(rows, rules),
                          Settings(horizon=TURN))
    st0 = planner.initial_state(2)
    # the recorded game's sites and builder actions stay choosable, under its city names
    rec = recorded_choices(rows)
    rec_st, _ = run_recorded(planner, st0, rows)
    planner.s.extra_sites = [tuple(xy) for _, xy in rec["sites"]]
    planner.s.site_names = {tuple(xy): nm for nm, xy in rec["sites"]}
    planner.s.extra_builder = [c for k, _, c in rec_st.log if k[0] == "builder" and c and c[0] != "wait"]
    return planner, st0


def recorded_genome(planner, st0, rows):
    """The recorded game as a genome of ordinary decisions (its sites as settle choices)."""
    rec = recorded_choices(rows)
    st, _ = run_recorded(planner, st0, rows)
    g = {k: c for k, _, c in st.log if c is not None}
    for n, (_, xy) in enumerate(rec["sites"]):
        g[("settle", "", n)] = tuple(xy)
    return g


def main():
    budget, seeds, workers = arg("--budget", 2000), arg("--seeds", 3), arg("--workers", 4)
    methods = arg("--methods", "nrpa,es", str).split(",")
    out = arg("--out", f"data/search/opt_T{TURN}.json", str)
    planner, st0 = build()
    base, _ = opt.play(planner, st0, lambda s, k, o: planner.base(s, k, o), TURN)
    print(f"objective: food + production per turn, summed over turns 2..{TURN}")
    print(f"base policy: {base:.1f}")
    rows = [json.loads(line) for line in open(f"{PREFIX}_p0.jsonl")]
    # the recorded game as the starting plan (when it ends before TURN, its choices so far; the rest from the base)
    rec_genome = recorded_genome(planner, st0, rows)
    v, _ = opt.play(planner, st0, lambda s, k, o: rec_genome[k] if rec_genome.get(k) in o else planner.base(s, k, o), TURN)
    print(f"recorded game as a genome: {v:.1f}")
    results = {}
    with mp.Pool(workers, initializer=opt._winit, initargs=(build,)) as pool:
        for m in methods:
            for seed in range(seeds):
                t = time.time()
                kw = {"init": rec_genome} if m in ("es_rec", "nrpa_rec", "hybrid") else {}
                cnt = getattr(opt, m)(pool, TURN, budget, lam=workers, seed=seed, **kw)
                print(f"{m} seed {seed}: best {cnt.best:.1f} after {cnt.n} rollouts ({time.time() - t:.0f}s)", flush=True)
                results[f"{m}_{seed}"] = {"best": cnt.best, "curve": cnt.curve[::10] + cnt.curve[-1:],
                                          "plan": [(list(k), c) for k, c, _, _ in getattr(cnt, "best_seq", [])]}
    Path(out).parent.mkdir(parents=True, exist_ok=True)
    Path(out).write_text(json.dumps(results, default=str))
    print("saved", out)


if __name__ == "__main__":
    main()
