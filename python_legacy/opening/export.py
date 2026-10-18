"""Data for the opening report's charts (JSON on stdout)."""
import json, itertools
from dp import solve, thr, new_city_yield
from heur import step, rule_growth_timing

BASE = (3, 3)
Q = (('scout', 15), ('settler', 40), ('settler', 55), ('settler', 70))

def trace(pool, picker, horizon=30, travel=3):
    """Per turn: pop, food store, capital F, P, empire F+P, worked tiles, settler done; picker(t, state, picks) -> pick."""
    tiles = sorted(pool, reverse=True)
    t, pop, store, prog, qi = 2, 1, 4, 0, 0
    founded, rows, cum = [], [], 0
    while t <= horizon:
        picks = list(itertools.combinations(range(len(tiles)), min(pop, len(tiles))))
        pick = picker(t, (pop, store, prog, qi), tiles, picks)
        F = BASE[0] + sum(tiles[i][0] for i in pick); P = BASE[1] + sum(tiles[i][1] for i in pick)
        _, pop2, store2, prog2, qi2, settled = step(BASE, tiles, pick, t, pop, store, prog, qi, Q, horizon, travel)
        if settled:
            founded.append(t + travel)
        empire = F + P + sum(new_city_yield(t - f) for f in founded if f <= t)
        cum += empire
        rows.append(dict(t=t, pop=pop, store=store, need=thr(pop), prog=prog, item=Q[qi][0] if qi < len(Q) else '-',
                         cost=Q[qi][1] if qi < len(Q) else 0, F=F, P=P, empire=empire, cum=cum, settled=settled,
                         tiles=[f'{tiles[i][0]}F{tiles[i][1]}P' for i in pick]))
        t += 1; pop, store, prog, qi = pop2, store2, prog2, qi2
    return rows

def optimal_picker(pool, horizon=30):
    plan = solve(BASE, pool, horizon=horizon)[1]
    tiles = sorted(pool, reverse=True)
    by_turn = {}
    for (t, pop, store, prog, item, pick, settled) in plan:
        by_turn[t] = list(pick)
    def picker(t, state, tiles_, picks):
        want = sorted(by_turn[t], reverse=True)
        for p in picks:
            if sorted((tiles_[i] for i in p), reverse=True) == want:
                return p
        raise ValueError
    return picker

def policy(key):
    return lambda t, s, tiles, picks: max(picks, key=lambda p: key((BASE[0] + sum(tiles[i][0] for i in p), BASE[1] + sum(tiles[i][1] for i in p))))

pools = {
    'mixed': [(3, 1), (2, 2), (1, 3), (2, 1), (2, 1), (1, 1)],
    'f4': [(4, 0), (3, 1), (2, 2), (1, 3), (2, 1), (1, 1)],
    'prod': [(1, 3), (1, 3), (2, 2), (2, 1), (3, 0), (1, 1)],
}
out = {'thresholds': [{'pop': p, 'need': thr(p)} for p in range(1, 8)], 'traces': {}}
for name, pool in pools.items():
    out['traces'][name] = {
        'pool': [f'{f}F{p}P' for f, p in sorted(pool, reverse=True)],
        'optimal': trace(pool, optimal_picker(pool)),
        'food': trace(pool, policy(lambda y: y)),
        'prod': trace(pool, policy(lambda y: y[::-1])),
        'rule': trace(pool, lambda t, s, tiles, picks: rule_growth_timing(BASE, tiles, picks, t, *s, Q)),
    }
print(json.dumps(out))
