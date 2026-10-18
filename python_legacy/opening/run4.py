"""Scouts (0-2) x settlers (2-4) before other items, each with the optimal tiles, at T30 and T40."""
from dp import solve
base = (3, 3)
pools = {
    'mixed': [(3, 1), (2, 2), (1, 3), (2, 1), (2, 1), (1, 1)],
    'with 4F0P': [(4, 0), (3, 1), (2, 2), (1, 3), (2, 1), (1, 1)],
    'France Paris': [(3, 1), (2, 2), (2, 1), (2, 1), (1, 1)],
    'prod heavy': [(1, 3), (1, 3), (2, 2), (2, 1), (3, 0), (1, 1)],
}
for name, pool in pools.items():
    for H in (30, 40):
        row = []
        for sc in (0, 1, 2):
            for ns in (2, 3, 4):
                q = tuple([('scout', 15)] * sc) + tuple(('settler', 40 + 15 * i) for i in range(ns))
                row.append((sc, ns, solve(base, pool, horizon=H, queue=q)[0]))
        best = max(r[2] for r in row)
        print(name, 'T%d' % H, ' '.join(f'scout{a}/settler{b}:{v:.0f}({v - best:+.0f})' for a, b, v in row))
