"""Search for the best plan under one objective: per-turn food + production of all cities, summed over the turns up
to a given turn.

Methods, compared at equal numbers of rollouts (python_legacy/scripts/opt_experiment.py):
  - NRPA (Nested Rollout Policy Adaptation, Rosin 2011; generalized with a prior bias, Cazenave 2020): a softmax
    policy over the options of each decision, learned towards the best plans found. The planner's base policy is
    the prior: its choice gets a bias, so rollouts start as sensible plans and the policy learns where to deviate.
    A move is coded by (decision key, option); the key (kind, entity, n) is the n-th decision of its kind for the
    entity, so a choice stays attached to the same decision when earlier changes shift turns.
  - ES: (1+lambda) evolution: re-decide a few choices of the best plan (the rest kept), keep the best.
Rollouts run in worker processes (each builds its own planner), lambda at a time.
"""
import copy
import math
import random

OBJ_WEIGHTS = {"YIELD_FOOD": 1.0, "YIELD_PRODUCTION": 1.0}


def objective(snap):
    return sum(w * snap.get(k, 0.0) for k, w in OBJ_WEIGHTS.items())


def play(planner, st0, chooser, turn):
    """Simulate to the end of `turn`. The objective (user, 2026-10-07): the per-turn food + production of all
    cities summed over every turn up to `turn`, so growth along the way counts and end-of-horizon tricks do not pay.
    st.final_output keeps that turn's own per-turn output."""
    st = copy.deepcopy(st0)
    total = 0.0
    while st.turn <= turn:
        planner.advance(st, chooser)
        total += objective(st.last_output)
    return total, st


# ------------------------------------------------------------------ policy rollouts
def _logits(planner, s, key, options, policy, bias, incumbent=None):
    """The prior choice gets `bias`: the incumbent plan's choice where it has one that is valid, else the base
    policy's."""
    base = incumbent.get(key) if incumbent else None
    if base is None or base not in options:
        base = planner.base(s, key, options)
    return [policy.get((key, o), 0.0) + (bias if o == base else 0.0) for o in options]


def policy_rollout(planner, st0, turn, policy, bias, rng, fixed=None, incumbent=None):
    """One plan drawn from the softmax policy. Returns (score, sequence [(key, choice, options, probs)])."""
    seq = []

    def chooser(s, key, options):
        if fixed is not None and key in fixed and fixed[key] in options:
            c = fixed[key]
            seq.append((key, c, list(options), None))
            return c
        lg = _logits(planner, s, key, options, policy, bias, incumbent)
        m = max(lg)
        ws = [math.exp(x - m) for x in lg]
        tot = sum(ws)
        probs = [w / tot for w in ws]
        x, acc = rng.random(), 0.0
        c = options[-1]
        for o, p in zip(options, probs):
            acc += p
            if x <= acc:
                c = o
                break
        seq.append((key, c, list(options), probs))
        return c

    score, st = play(planner, st0, chooser, turn)
    return score, seq


def adapt(policy, seq, alpha):
    """NRPA step: towards the choices of `seq`, away from the alternatives in proportion to their probability."""
    pol = dict(policy)
    for key, c, options, probs in seq:
        if probs is None:
            continue
        pol[(key, c)] = pol.get((key, c), 0.0) + alpha
        for o, p in zip(options, probs):
            pol[(key, o)] = pol.get((key, o), 0.0) - alpha * p
    return pol


def genome_of(seq):
    return {key: c for key, c, _, _ in seq}


# ------------------------------------------------------------------ parallel workers
_W = {}


def _winit(build):
    planner, st0 = build()
    _W.update(planner=planner, st0=st0)


def _wroll(args):
    turn, policy, bias, seed, incumbent, fixed = args
    return policy_rollout(_W["planner"], _W["st0"], turn, policy, bias, random.Random(seed), fixed=fixed,
                          incumbent=incumbent)


def _wgenome(args):
    """Follow a genome; other decisions from the base policy, or a random option with probability eps."""
    turn, genome, eps, seed = args
    planner, rng = _W["planner"], random.Random(seed)
    seq = []

    def chooser(s, key, options):
        want = genome.get(key)
        c = want if (want is not None and want in options) else (
            rng.choice(options) if rng.random() < eps else planner.base(s, key, options))
        seq.append((key, c, list(options), None))
        return c

    score, _ = play(planner, _W["st0"], chooser, turn)
    return score, seq


class Counter:
    def __init__(self, budget, log):
        self.n, self.budget, self.log, self.best, self.curve = 0, budget, log, -1e18, []

    def add(self, results):
        for score, seq in results:
            self.n += 1
            if score > self.best:
                self.best = score
                self.best_seq = seq
            self.curve.append((self.n, self.best))
        if self.log and self.n % 200 < len(results):
            self.log(f"  {self.n} rollouts, best {self.best:.1f}")

    def done(self):
        return self.n >= self.budget


def nrpa(pool, turn, budget, lam=4, level=2, iters=None, alpha=1.0, bias=3.0, seed=0, log=None, init=None, fixed=None):
    """Level-`level` NRPA; at level 1 each iteration draws `lam` rollouts in parallel and adapts to the best so far.
    iters per level defaults so that the budget is used (iters^level * lam ~ budget)."""
    rng = random.Random(seed)
    cnt = Counter(budget, log)
    if iters is None:
        iters = max(2, int(round((budget / lam) ** (1.0 / level))))

    def run(lv, policy):
        if lv == 0 or cnt.done():
            res = pool.map(_wroll, [(turn, policy, bias, rng.randrange(1 << 30), init, fixed) for _ in range(lam)])
            cnt.add(res)
            return max(res, key=lambda r: r[0])
        best = (-1e18, None)
        for _ in range(iters):
            if cnt.done():
                break
            r = run(lv - 1, dict(policy))
            if r[0] >= best[0]:
                best = r
            policy = adapt(policy, best[1], alpha)
        return best

    while not cnt.done():
        run(level, {})
    return cnt


def es(pool, turn, budget, lam=4, n_drop=3, eps=0.15, seed=0, log=None, init=None):
    """(1+lambda): mutate the best plan (drop n_drop of its choices, or its tail from a random point) and re-decide.
    init: a genome to start from (e.g. the recorded game's choices); default the base policy."""
    rng = random.Random(seed)
    cnt = Counter(budget, log)
    first = pool.map(_wgenome, [(turn, dict(init or {}), 0.0, 0)])[0]
    best_score, best_g = first[0], genome_of(first[1])
    cnt.add([first])
    while not cnt.done():
        kids = []
        for _ in range(lam):
            g = dict(best_g)
            keys = list(g)
            if keys and rng.random() < 0.3:
                for k in keys[rng.randrange(len(keys)):]:
                    g.pop(k)
            else:
                for k in rng.sample(keys, min(n_drop, len(keys))):
                    g.pop(k)
            kids.append((turn, g, eps, rng.randrange(1 << 30)))
        res = pool.map(_wgenome, kids)
        cnt.add(res)
        top = max(res, key=lambda r: r[0])
        if top[0] >= best_score:
            best_score, best_g = top[0], genome_of(top[1])
    return cnt


def es_rec(pool, turn, budget, lam=4, seed=0, log=None, init=None):
    """ES started from the recorded game's choices (init)."""
    return es(pool, turn, budget, lam=lam, seed=seed, log=log, init=init)


def nrpa_rec(pool, turn, budget, lam=4, seed=0, log=None, init=None):
    """NRPA whose prior is the incumbent plan (init, e.g. the recorded game's choices)."""
    return nrpa(pool, turn, budget, lam=lam, seed=seed, log=log, init=init)


def hybrid(pool, turn, budget, lam=4, seed=0, log=None, init=None, rounds=3):
    """Alternate NRPA (prior: the incumbent) and ES (from the incumbent), the incumbent being the best so far."""
    cnt = Counter(budget, log)
    inc = dict(init or {})
    per = budget // (2 * rounds)
    for r in range(rounds):
        for method in (nrpa, es):
            c = method(pool, turn, per, lam=lam, seed=seed * 100 + r, init=inc)
            cnt.n += c.n
            cnt.curve += [(cnt.n - c.n + n, max(b, cnt.best)) for n, b in c.curve]
            if c.best > cnt.best:
                cnt.best, cnt.best_seq = c.best, c.best_seq
                inc = genome_of(c.best_seq)
    return cnt
