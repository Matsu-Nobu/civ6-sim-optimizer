"""Toy model of a Civ6 BBG capital's opening (online speed), solved exactly by DP over the tiles worked each turn.
Rules as in the simulator: growth threshold floor((15+8n+n^1.5)/2) (n = pop-1), 2 food per citizen, food store kept
when a settler is trained (pop -1, then growth checked against the new pop), housing slows growth (left<=1: 50%,
left<=0: 25%), production overflow carries. Settler 40/55/70, scout 15. A settler founds a city `travel` turns later;
a new city's F+P stream is a fixed curve (from the France game). Objective: capital F+P + new cities' F+P to `horizon`."""
import itertools, functools, sys

def thr(pop):
    n = pop - 1
    return int((15 + 8 * n + n ** 1.5) * 0.5)

def growth_mult(pop, housing):
    left = housing - pop
    return 0.0 if left <= -4 else 0.25 if left <= 0 else 0.5 if left <= 1 else 1.0

# a new city's F+P by turns since founding (France: pop1 ~9, pop2 ~13 after ~4 turns, pop3 ~16 after ~12)
def new_city_yield(k):
    return 9 if k < 4 else 13 if k < 12 else 16 if k < 20 else 19

def solve(base, tiles, housing=6, queue=(('scout', 15), ('settler', 40), ('settler', 55), ('settler', 70)), start=2,
          horizon=30, travel=3, pop0=1, store0=4, policy=None):
    tiles = tuple(sorted(tiles, reverse=True))
    found_value = lambda t: sum(new_city_yield(k) for k in range(horizon - t + 1)) if t <= horizon else 0

    @functools.lru_cache(maxsize=None)
    def V(t, pop, store, prog, qi):
        if t > horizon:
            return 0.0, ()
        best = (-1e9, ())
        seen = set()
        picks = itertools.combinations(range(len(tiles)), min(pop, len(tiles)))
        if policy:
            order = sorted(range(len(tiles)), key=lambda i: policy(tiles[i]), reverse=True)
            picks = [tuple(sorted(order[:min(pop, len(tiles))]))]
        for pick in picks:
            F = base[0] + sum(tiles[i][0] for i in pick)
            P = base[1] + sum(tiles[i][1] for i in pick)
            if (F, P) in seen:
                continue
            seen.add((F, P))
            gain = F + P
            npop, nstore, nprog, nqi = pop, store, prog, qi
            extra = 0.0
            item = queue[qi] if qi < len(queue) else None
            settled = False
            if item and not (item[0] == 'settler' and pop < 2):
                nprog = prog + P
                if nprog >= item[1]:
                    nprog -= item[1]
                    nqi = qi + 1
                    if item[0] == 'settler':
                        npop -= 1
                        settled = True
                        extra = found_value(t + travel)
                        # the weakest worked tile is dropped for this turn's food
                        F -= min(tiles[i][0] for i in pick) if pick else 0
            surplus = F - 2 * npop
            add = surplus if surplus <= 0 else surplus * growth_mult(npop, housing)
            nstore = store + add
            if nstore >= thr(npop):
                nstore -= thr(npop)
                npop += 1
            nstore = round(nstore * 4) / 4
            v, plan = V(t + 1, npop, nstore, nprog if nqi < len(queue) else 0, nqi)
            # ties go to production (a tiny bonus), so a food tile is chosen only where it gains something
            tot = gain + extra + v + 1e-4 * P
            if tot > best[0]:
                best = (tot, ((t, pop, store, prog, queue[qi][0] if qi < len(queue) else '-', tuple(tiles[i] for i in pick), settled),) + plan)
        return best

    return V(start, pop0, store0, 0, 0)

def show(res, label):
    total, plan = res
    print(f'--- {label}: {total:.0f}')
    for t, pop, store, prog, item, pick, settled in plan:
        print(f'  T{t:2d} pop{pop} food{store:5.1f} prod{prog:3d} {item:7s} works {" ".join(f"{f}F{p}P" for f, p in pick)}{"  << settler" if settled else ""}')
