"""Broad strategy search: MAP-Elites (quality-diversity) over full decision sequences.

A plan ("genome") is the dict of choices the planner made, keyed by decision (kind, entity, n). A rollout follows
the genome where it can and decides the rest with a randomized base policy, so changing a few choices yields a
new, still coherent plan. Every rollout is filed into archives: one per objective, each a grid of strategy
descriptors (e.g. number of cities x districts), keeping the best plan per cell. Keeping the best of every
*kind* of strategy, rather than only the overall best, stops the search from collapsing onto one style early;
a style that is weak at turn 30 but strong at turn 60 survives in its own cell.

Objectives (per-turn yields of all cities, food and production weighted equally):
  A: food + production at turn 30          B: food + production at turn 60
"""
import copy
import json
import random
import time
from dataclasses import dataclass, field

OBJECTIVES = {
    "A_T30": {"turn": 30, "weights": {"YIELD_FOOD": 1.0, "YIELD_PRODUCTION": 1.0}},
    "B_T60": {"turn": 60, "weights": {"YIELD_FOOD": 1.0, "YIELD_PRODUCTION": 1.0}},
}
SPECIALTY = None  # set lazily: districts that need population (campus, holy site, ...)


@dataclass
class Result:
    genome: dict
    snaps: dict  # turn -> {yield: per-turn total}
    desc: dict  # descriptor name -> value
    summary: dict = field(default_factory=dict)

    def value(self, obj):
        o = OBJECTIVES[obj]
        s = self.snaps.get(o["turn"])
        return None if s is None else sum(w * s.get(k, 0) for k, w in o["weights"].items())


def _describe(planner, st):
    global SPECIALTY
    if SPECIALTY is None:
        SPECIALTY = {d for (d,) in planner.r.q("select DistrictType from Districts where RequiresPopulation=1")}
    districts = sum(1 for c in st.cities for d in c["districts"] if d["complete"] and d["type"] in SPECIALTY)
    return {"cities": len(st.cities), "districts": districts,
            "builders": st.acc.units_trained.get("UNIT_BUILDER", 0),
            "settlers": st.acc.units_trained.get("UNIT_SETTLER", 0)}


def rollout(planner, st0, genome, rng, eps, horizon, snap_turns=(30, 60)):
    """Play to `horizon`: genome choices where valid, else the base policy with probability 1-eps or a random
    option. Returns a Result whose genome holds every choice actually made."""
    st = copy.deepcopy(st0)

    def chooser(s, key, options):
        want = genome.get(key)
        if want is not None and want in options:
            return want
        if rng.random() < eps:
            return rng.choice(options)
        return planner.base(s, key, options)

    snaps, descs = {}, {}
    while st.turn <= horizon:
        t = st.turn
        planner.advance(st, chooser)
        if t in snap_turns:
            snaps[t] = dict(st.last_output)
            descs[t] = _describe(planner, st)
    made = {k: c for k, _, c in st.log if c is not None}
    first_turn = {}
    for k, t, c in st.log:
        first_turn.setdefault(k, t)
    res = Result(genome=made, snaps=snaps, desc={f"{n}@{t}": v for t, d in descs.items() for n, v in d.items()})
    res.summary = summarize(st)
    res.order = sorted(made, key=lambda k: first_turn.get(k, 0))
    return res


def summarize(st):
    prod = [(t, k[1], c) for k, t, c in st.log if k[0] == "prod" and c]
    return {
        "cities": [(c["name"], c["center"]["x"], c["center"]["y"]) for c in st.cities],
        "production": prod[:60],
        "research": [c for k, t, c in st.log if k[0] == "research" and c][:20],
        "civics": [c for k, t, c in st.log if k[0] == "civic" and c][:12],
        "pantheon": st.player.get("pantheon"), "government": st.player.get("government"),
        "policies": list(st.player.get("policies", [])),
    }


# Archive cells: objective -> descriptor names (bucketed values)
CELLS = {
    "A_T30": ("cities@30", "builders@30"),
    "B_T60": ("cities@60", "districts@60"),
}


def cell_of(res, obj):
    return tuple(min(res.desc.get(n, 0), 8) for n in CELLS[obj])


class Archive:
    def __init__(self):
        self.cells = {obj: {} for obj in OBJECTIVES}

    def add(self, res):
        improved = 0
        for obj in OBJECTIVES:
            v = res.value(obj)
            if v is None:
                continue
            c = cell_of(res, obj)
            cur = self.cells[obj].get(c)
            if cur is None or v > cur.value(obj):
                self.cells[obj][c] = res
                improved += 1
        return improved

    def elites(self):
        return [r for cells in self.cells.values() for r in cells.values()]

    def best(self, obj):
        cells = self.cells[obj]
        return max(cells.values(), key=lambda r: r.value(obj)) if cells else None


def mutate(res, rng, n_drop):
    """Forget n_drop choices (and, with some probability, everything after one of them) so they are re-decided."""
    g = dict(res.genome)
    order = [k for k in getattr(res, "order", list(g)) if k in g]
    if not order:
        return g
    if rng.random() < 0.3:
        cut = rng.randrange(len(order))  # re-decide the tail from a random point
        for k in order[cut:]:
            g.pop(k, None)
        return g
    for k in rng.sample(order, min(n_drop, len(order))):
        g.pop(k, None)
    return g


# ------------------------------------------------------------------ parallel driver
_P = {}


def _init(build):
    planner, st0, horizon = build()
    _P.update(planner=planner, st0=st0, horizon=horizon)


def _work(args):
    genome, seed, eps = args
    rng = random.Random(seed)
    return rollout(_P["planner"], _P["st0"], genome, rng, eps, _P["horizon"])


def map_elites(build, budget, workers=15, init=300, batch=60, seed=1, log=print, out=None):
    """build() -> (planner, st0, horizon) is called once per worker process."""
    import multiprocessing as mp
    rng = random.Random(seed)
    arch = Archive()
    done = 0
    t0 = time.time()
    with mp.get_context("fork").Pool(workers, initializer=_init, initargs=(build,)) as pool:
        # random starts: wide range of randomness so very different styles appear
        jobs = [({}, rng.randrange(1 << 30), rng.choice([0.05, 0.15, 0.3, 0.5])) for _ in range(init)]
        for res in pool.imap_unordered(_work, jobs, chunksize=4):
            arch.add(res)
            done += 1
        while done < budget:
            elites = arch.elites()
            jobs = []
            for _ in range(batch):
                parent = rng.choice(elites)
                jobs.append((mutate(parent, rng, rng.choice([1, 2, 4, 8])), rng.randrange(1 << 30),
                             rng.choice([0.0, 0.1, 0.3])))
            for res in pool.imap_unordered(_work, jobs, chunksize=2):
                arch.add(res)
                done += 1
            if done % (batch * 10) < batch:
                a, b = arch.best("A_T30"), arch.best("B_T60")
                log(f"{done} rollouts, {time.time() - t0:.0f}s: cells A {len(arch.cells['A_T30'])} B {len(arch.cells['B_T60'])}, "
                    f"best A {a.value('A_T30'):.1f} B {b.value('B_T60'):.1f}")
                if out:
                    save(arch, out)
    if out:
        save(arch, out)
    return arch


def save(arch, path):
    data = {}
    for obj, cells in arch.cells.items():
        data[obj] = [{"cell": list(c), "value": r.value(obj), "snaps": {str(t): s for t, s in r.snaps.items()},
                      "desc": r.desc, "summary": r.summary,
                      "genome": [[list(k), v] for k, v in r.genome.items()]}
                     for c, r in sorted(cells.items(), key=lambda x: -x[1].value(obj))]
    json.dump(data, open(path, "w"), default=str)
