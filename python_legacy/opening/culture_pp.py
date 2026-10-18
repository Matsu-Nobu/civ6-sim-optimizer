"""When must monuments be built for Political Philosophy by T30 (all inspirations), with or without Moksha?
Culture per turn = palace 1 (from T3) + 0.3 per citizen + 2 per monument; the capital's culture x1.15 once Moksha is
established, plus 1 per capital citizen once he also has Connoisseur. Civics in the order CoL, Foreign Trade, Early
Empire (title 1: Moksha appointed, established 4 turns later), Craftsmanship, State Workforce (title 2: Connoisseur),
Political Philosophy: cumulative culture 10, 23, 45, 58, 80, 121 (online, every inspiration). Population per turn from
a recorded game (no culture from tiles)."""
import json, sys

def pops(path):
    G = {r['turn']: r for r in (json.loads(l) for l in open(path))}
    return {t: (sum(c['population'] for c in G[t]['cities']), G[t]['cities'][0]['population'] if G[t]['cities'] else 0) for t in range(2, 31)}

def run(P, mon_cap, mon_2nd, moksha=True, second_city_from=12):
    cum, ee, sw, done = 0.0, None, None, None
    for t in range(2, 31):
        tot, cap = P[t]
        est = moksha and ee is not None and t >= ee + 4
        conn = est and sw is not None and t >= sw
        capc = (1 if t >= 3 else 0) + 0.3 * cap + (2 if mon_cap and t > mon_cap else 0) + (cap if conn else 0)
        if est: capc *= 1.15
        other = 0.3 * (tot - cap) + (2 if mon_2nd and t > mon_2nd and t >= second_city_from else 0)
        cum += capc + other
        if ee is None and cum >= 45: ee = t
        if sw is None and cum >= 80: sw = t
        if done is None and cum >= 121: done = t
    return done, cum, ee, sw

for path, label in [(sys.argv[1], sys.argv[2])] + ([(sys.argv[3], sys.argv[4])] if len(sys.argv) > 4 else []):
    P = pops(path)
    print(f'== population as in {label}')
    for moksha in (True, False):
        print(f'  Moksha: {moksha}')
        # one monument (capital): latest completion turn that still finishes PP by T30
        ok1 = [m for m in range(5, 31) if (run(P, m, None, moksha)[0] or 99) <= 30]
        print(f'    capital monument only: latest T{max(ok1)}' if ok1 else '    capital monument only: never enough')
        none = run(P, None, None, moksha)
        print(f'    no monument: PP T{none[0]}, culture at T30 {none[1]:.0f} (Early Empire T{none[2]}, State Workforce T{none[3]})')
        # two monuments: for each capital turn, latest 2nd-city turn
        rows = []
        for mc in range(12, 27, 2):
            ok = [m2 for m2 in range(12, 31) if (run(P, mc, m2, moksha)[0] or 99) <= 30]
            rows.append(f'cap T{mc}->2nd by T{max(ok)}' if ok else f'cap T{mc}->none')
        print('    two monuments: ' + ', '.join(rows))
        for mc, m2 in [(19, 24), (17, 20), (20, None)]:
            d = run(P, mc, m2, moksha)
            print(f'    example cap T{mc} 2nd {("T"+str(m2)) if m2 else "-"}: PP T{d[0]}, culture {d[1]:.0f}, EE T{d[2]}, SW T{d[3]}')
