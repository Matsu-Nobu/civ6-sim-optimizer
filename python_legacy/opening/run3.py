"""Rules of thumb vs the optimum on several tile pools."""
from heur import *
from dp import solve
base = (3, 3)
pools = {
 'mixed': [(3, 1), (2, 2), (1, 3), (2, 1), (2, 1), (1, 1)],
 'with 4F0P': [(4, 0), (3, 1), (2, 2), (1, 3), (2, 1), (1, 1)],
 'with 4F1P': [(4, 1), (2, 2), (1, 3), (2, 1), (2, 1), (1, 1)],
 'France Paris': [(3, 1), (2, 2), (2, 1), (2, 1), (1, 1)],
 'Rome-like': [(3, 0), (2, 2), (2, 2), (2, 1), (2, 1), (1, 1)],
 'prod heavy': [(1, 3), (1, 3), (2, 2), (2, 1), (3, 0), (1, 1)],
}
food_first = lambda b, ts, ps, *a: max(ps, key=lambda p: yields(b, ts, p)[::1])
prod_first = lambda b, ts, ps, *a: max(ps, key=lambda p: yields(b, ts, p)[::-1])
total_first = lambda b, ts, ps, *a: max(ps, key=lambda p: (sum(yields(b, ts, p)), yields(b, ts, p)[1]))
for name, pool in pools.items():
    for H in (30, 40):
        opt = solve(base, pool, horizon=H)[0]
        r = {n: run(base, pool, f, horizon=H) for n, f in [('food', food_first), ('prod', prod_first), ('total', total_first),
             ('timing', lambda *a: rule_growth_timing(*a, settler_trick=False)), ('timing+settler', rule_growth_timing)]}
        print(f'{name:12s} T{H}: optimum {opt:.0f} | ' + ' '.join(f'{k} {v:.0f} ({100*(v/opt-1):+.1f}%)' for k, v in r.items()))
