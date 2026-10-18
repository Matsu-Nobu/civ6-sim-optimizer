"""Per player: scouts built (city production) before the first settler, goody-hut units, first settler, cities and
cumulative F+P at T20/T30, from per-player exports (export_game.sh) of the first 30 turns."""
import json, sys, glob, os, re
rows = []
for f in sorted(glob.glob(os.path.join(sys.argv[1], 'g*_p*.jsonl'))):
    game, pid = re.search(r'g(\d+)_p(\d+)', f).groups()
    G = {r['turn']: r for r in (json.loads(l) for l in open(f) if l.strip())}
    if not G: continue
    civ = next((r['player'].get('civilization') for r in G.values() if r['player'].get('civilization')), '?')
    cum = 0; out = {}; prev_cur = {}; prev_prog = {}; prev_sc = 0; built = 0; built_before_settler = None; first_settler = None
    prev_set = 0
    for t in sorted(G):
        r = G[t]
        ys = r['cityYields'] or []
        cum += sum(c['modified']['YIELD_FOOD'] + c['modified']['YIELD_PRODUCTION'] for c in ys)
        ut = r['player'].get('unitsTrained') or {}
        sc = ut.get('UNIT_SCOUT', 0)
        if sc > prev_sc:
            # built if some city was making a scout last turn and is not now (or restarted)
            def done(c):
                pr = c.get('production') or {}
                if prev_cur.get(c['name']) != 'UNIT_SCOUT':
                    return False
                return pr.get('current') != 'UNIT_SCOUT' or (pr.get('progress') or {}).get('UNIT_SCOUT', 0) < prev_prog.get(c['name'], 0)
            n_made = sum(1 for c in r['cities'] if done(c))
            built += min(sc - prev_sc, n_made)
        prev_sc = sc
        st = ut.get('UNIT_SETTLER', 0)
        if st > prev_set and first_settler is None:
            first_settler = t; built_before_settler = built
        prev_set = st
        prev_cur = {c['name']: (c.get('production') or {}).get('current') for c in r['cities']}
        prev_prog = {c['name']: ((c.get('production') or {}).get('progress') or {}).get('UNIT_SCOUT', 0) for c in r['cities']}
        if t in (20, 30):
            out[t] = (cum, len(r['cities']))
    huts = G[max(G)]['player'].get('goodyHutsReceived') or {}
    rows.append(dict(game=game, pid=int(pid), civ=civ.replace('CIVILIZATION_', ''), scouts_early=built_before_settler if built_before_settler is not None else built,
                     scouts30=built, first_settler=first_settler, huts=sum(huts.values()), hut_mil=huts.get('GOODYHUT_MILITARY', 0),
                     cum20=out.get(20, (0, 0))[0], cum30=out.get(30, (0, 0))[0], cities20=out.get(20, (0, 0))[1], cities30=out.get(30, (0, 0))[1]))
rows.sort(key=lambda r: (r['game'], -r['cum30']))
for r in rows:
    print(f"10/{r['game']} p{r['pid']:2d} {r['civ']:16s} 偵察(開拓前){r['scouts_early']} 偵察T30まで{r['scouts30']} 初開拓T{r['first_settler']} 集落{r['huts']}(軍{r['hut_mil']}) 都市T20:{r['cities20']} T30:{r['cities30']} 累計T20 {r['cum20']:.0f} T30 {r['cum30']:.0f}")
json.dump(rows, open(os.path.join(sys.argv[1], 'summary.json'), 'w'))
