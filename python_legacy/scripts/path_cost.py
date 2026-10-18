"""Science (culture) still needed at turn T for a tech (civic) and its prerequisites, from a player export:
online cost, game-era adjustment, 40% inspirations."""
import sqlite3, json, math, sys
c = sqlite3.connect(sys.argv[1])
G = {r['turn']: r for r in (json.loads(l) for l in open(sys.argv[2]))}
T = int(sys.argv[3]); gera = sys.argv[4]
eras = [r[0] for r in c.execute('select EraType from Eras order by ChronologyIndex')]
def closure(goal, civic):
    t, k, p = ('CivicPrereqs', 'Civic', 'PrereqCivic') if civic else ('TechnologyPrereqs', 'Technology', 'PrereqTech')
    out = set(); todo = [goal]
    while todo:
        x = todo.pop()
        if x in out: continue
        out.add(x); todo += [r[0] for r in c.execute(f'select {p} from {t} where {k}=?', (x,))]
    return out
def cost(x, civic):
    tab, col = ('Civics', 'CivicType') if civic else ('Technologies', 'TechnologyType')
    base, era = c.execute(f'select Cost, EraType from {tab} where {col}=?', (x,)).fetchone()
    oe, og = eras.index(era), eras.index(gera)
    pct = 20 if oe > og else (-20 if oe < og else 0)
    return math.floor(math.floor(base) * 0.5 * (1 + pct / 100) + 1e-9), era
for goal in sys.argv[5:]:
    civic = goal.startswith('CIVIC_')
    p = G[T]['player']
    have = set(p['civics'] if civic else p['techs'])
    need = sorted(closure(goal, civic) - have)
    prog = (p['civicProgress'] if civic else p['techProgress']) or {}
    boosted = set((p.get('civicsInspired') if civic else p.get('techsBoosted')) or [])
    tot = totb = rest = 0
    print(f'== {goal}: still needed at T{T} (game era {gera})')
    for x in need:
        cst, era = cost(x, civic)
        col = 'CivicType' if civic else 'TechnologyType'
        b = c.execute(f'select BoostClass from Boosts where {col}=?', (x,)).fetchone()
        boost = math.floor(0.4 * (cst - 2.5)) if b else 0
        tot += cst; totb += cst - boost
        got = x in boosted
        left = max(0, cst - prog.get(x, 0) - (0 if got else boost))
        rest += left
        print(f"   {x.split('_', 1)[1]:22s} {era[4:]:9s} cost {cst:4d}  inspiration -{boost:3d} ({b[0].replace('BOOST_TRIGGER_', '') if b else '-'}{', got' if got else ''})  progress {prog.get(x, 0):.0f}  left {left:.0f}")
    print(f'   total {tot}, with every inspiration {totb}; still needed at T{T} (inspirations not yet got assumed got) {rest:.0f}')
