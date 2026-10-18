"""Optimal tiles vs fixed priorities (food first / production first), T30 and T40, pool "mixed"."""
from dp import *
base = (3, 3)
pool = [(3, 1), (2, 2), (1, 3), (2, 1), (2, 1), (1, 1)]
food_first = lambda y: (y[0], y[1])
prod_first = lambda y: (y[1], y[0])
total_first = lambda y: (y[0] + y[1], y[1])
for H in (30, 40):
    r = solve(base, pool, horizon=H)
    show(r, f'pool {pool} optimal, horizon T{H}')
    for name, pol in [('food first', food_first), ('production first', prod_first), ('F+P then production', total_first)]:
        print(f'   {name}: {solve(base, pool, horizon=H, policy=pol)[0]:.0f}')
