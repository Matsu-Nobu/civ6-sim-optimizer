"""Compare two exported games' openings (T2..T30): per-turn F/P, cities, production, units, huts, gold, techs."""
import json, sys
def load(f):
    return {r['turn']: r for r in (json.loads(l) for l in open(f))}
def summary(G, label, last=30):
    print(f'===== {label}')
    cum = 0; prev_items = {}; prev_units = {}; prev_huts = {}; prev_techs = []; prev_civics = []
    founded = {}
    for t in range(1, last + 1):
        r = G.get(t)
        if not r: continue
        F = sum(c['modified']['YIELD_FOOD'] for c in (r['cityYields'] or [])); P = sum(c['modified']['YIELD_PRODUCTION'] for c in (r['cityYields'] or []))
        cum += F + P
        ev = []
        for c in r['cities']:
            if c['name'] not in founded:
                founded[c['name']] = t; ev.append(f"建設 {c['name']}")
            cur = (c.get('production') or {}).get('current')
            if prev_items.get(c['name']) != cur and cur:
                ev.append(f"{c['name'][:6]}:{cur.replace('UNIT_','').replace('BUILDING_','').replace('DISTRICT_','')}")
            prev_items[c['name']] = cur
        ut = r['player'].get('unitsTrained') or {}
        for u, n in ut.items():
            if n > prev_units.get(u, 0): ev.append(f"+{u.replace('UNIT_','')}")
        prev_units = dict(ut)
        hu = r['player'].get('goodyHutsReceived') or {}
        for h, n in hu.items():
            if n > prev_huts.get(h, 0): ev.append(f"村:{h.replace('GOODYHUT_','')}")
        prev_huts = dict(hu)
        techs = r['player']['techs']
        for x in techs:
            if x not in prev_techs: ev.append('技術:' + x.replace('TECH_',''))
        prev_techs = list(techs)
        civ = r['player']['civics']
        for x in civ:
            if x not in prev_civics: ev.append('制度:' + x.replace('CIVIC_',''))
        prev_civics = list(civ)
        pops = '/'.join(str(c['population']) for c in r['cities'])
        owned = sum(len(c.get('owned') or []) for c in r['cities'])
        print(f"T{t:2d} 人口{pops:8s} 食{F:5.1f} 生{P:5.1f} 累計{cum:6.0f} 金{r['player']['gold']:5.0f} タイル{owned:3d} | {' '.join(ev)}")
a, b = load(sys.argv[1]), load(sys.argv[2])
summary(a, sys.argv[3]); summary(b, sys.argv[4])
