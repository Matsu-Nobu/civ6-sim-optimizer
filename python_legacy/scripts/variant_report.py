"""One plan's T2-30 story from `civ6opt run ... --costs` output: research completions, each city's items (start-end,
cost when started), builders and improvements, cumulative food+production.
Usage: python3 python_legacy/scripts/diag/variant_report.py <run output> [last turn]"""
import json
import sys

last = int(sys.argv[2]) if len(sys.argv) > 2 else 30
snaps, costs = {}, {}
for line in open(sys.argv[1]):
    if not line.startswith('{'):
        continue
    v = json.loads(line)
    if 'costs_turn' in v:
        costs[v['costs_turn']] = {c[0]: (c[1], c[2], c[3]) for c in v['costs']}
    elif 'turn' in v:
        snaps[v['turn']] = v
cum, prev_res, techs = 0, None, []
items = {}   # city -> [item, start, cost]
order = {}   # city -> list of (item, start, end, cost)
imps = {}
prev_imp = {}
fp = {}
for t in sorted(snaps):
    if t > last:
        break
    s = snaps[t]
    cum += s['output'][0] + s['output'][1]
    fp[t] = cum
    if prev_res and s['research'] != prev_res:
        techs.append((t, prev_res.replace('TECH_', '')))
    prev_res = s['research']
    for c in s['cities']:
        n = c['name']
        cur = costs.get(t, {}).get(n)
        it = cur[0] if cur else None
        old = items.get(n)
        if old and old[0] != it:
            order.setdefault(n, []).append((old[0], old[1], t, old[2]))
        if it and (not old or old[0] != it):
            items[n] = [it, t, cur[1]]
        elif not it:
            items.pop(n, None)
        now = sorted(c['improvements'])
        was = prev_imp.get(n, [])
        for x in set(now):
            if now.count(x) > was.count(x):
                imps.setdefault(n, []).append((t, x.replace('IMPROVEMENT_', '')))
        prev_imp[n] = now
for n, it in items.items():
    order.setdefault(n, []).append((it[0], it[1], None, it[2]))
short = lambda x: x.replace('UNIT_', '').replace('BUILDING_', 'B:').replace('DISTRICT_', 'D:')
print('research done:', ' '.join(f'{x}T{t}' for t, x in techs))
for n, lst in order.items():
    print(f'{n}:', ' -> '.join(f"{short(i)}[{a}-{b if b else '...'}{f' c{c:.0f}' if 'D:' in short(i) else ''}]" for i, a, b, c in lst))
    if n in imps:
        print('   improvements:', ' '.join(f'{x}T{t}' for t, x in imps[n]))
print('cum F+P', ' '.join(f'T{t}:{fp[t]:.0f}' for t in (10, 15, 20, 25, 30) if t in fp))
