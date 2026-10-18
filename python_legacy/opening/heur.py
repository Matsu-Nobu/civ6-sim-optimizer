"""Forward simulation of the same toy with a rule (no lookahead beyond the rule), to test rules of thumb."""
import itertools, math
SLACK = 0  # 0: only the picks with the most F+P; 1: also those 1 below
from dp import thr, growth_mult, new_city_yield

def step(base, tiles, pick, t, pop, store, prog, qi, queue, horizon, travel):
    F = base[0] + sum(tiles[i][0] for i in pick); P = base[1] + sum(tiles[i][1] for i in pick)
    gain = F + P; extra = 0; npop, nprog, nqi = pop, prog, qi; settled = False
    item = queue[qi] if qi < len(queue) else None
    if item and not (item[0] == 'settler' and pop < 2):
        nprog = prog + P
        if nprog >= item[1]:
            nprog -= item[1]; nqi = qi + 1
            if item[0] == 'settler':
                npop -= 1; settled = True
                extra = sum(new_city_yield(k) for k in range(horizon - (t + travel) + 1)) if t + travel <= horizon else 0
                F -= min(tiles[i][0] for i in pick) if pick else 0
    surplus = F - 2 * npop
    add = surplus if surplus <= 0 else surplus * growth_mult(npop, 6)
    nstore = store + add
    if nstore >= thr(npop):
        nstore -= thr(npop); npop += 1
    return gain + extra, npop, round(nstore * 4) / 4, (nprog if nqi < len(queue) else 0), nqi, settled

def run(base, tiles, rule, queue=(('scout', 15), ('settler', 40), ('settler', 55), ('settler', 70)), horizon=30, travel=3, verbose=False):
    tiles = sorted(tiles, reverse=True)
    t, pop, store, prog, qi, total = 2, 1, 4, 0, 0, 0
    while t <= horizon:
        picks = list(itertools.combinations(range(len(tiles)), min(pop, len(tiles))))
        pick = rule(base, tiles, picks, t, pop, store, prog, qi, queue)
        g, pop2, store2, prog2, qi2, settled = step(base, tiles, pick, t, pop, store, prog, qi, queue, horizon, travel)
        if verbose:
            print(f'  T{t:2d} pop{pop} food{store:5.1f} prod{prog:3d} works {" ".join(f"{tiles[i][0]}F{tiles[i][1]}P" for i in pick)}{"  << settler" if settled else ""}')
        total += g; t += 1; pop, store, prog, qi = pop2, store2, prog2, qi2
    return total

def yields(base, tiles, pick):
    return base[0] + sum(tiles[i][0] for i in pick), base[1] + sum(tiles[i][1] for i in pick)

def rule_growth_timing(base, tiles, picks, t, pop, store, prog, qi, queue, settler_trick=True):
    """1) the most F+P; 2) among those within 1 of it: grow as early as possible (turns to the next citizen at this
    yield), then the most production; 3) (settler_trick) the turn a settler completes, do not grow."""
    best_total = max(sum(yields(base, tiles, p)) for p in picks)
    item = queue[qi] if qi < len(queue) else None
    def key(p):
        F, P = yields(base, tiles, p)
        surplus = F - 2 * pop
        grow_in = math.ceil((thr(pop) - store) / surplus) if surplus > 0 else 99
        completes = item is not None and item[0] == 'settler' and pop >= 2 and prog + P >= item[1]
        if settler_trick and completes:
            grow_in = -grow_in  # prefer not growing: the stored food regrows the citizen at the lower threshold
        return (F + P >= best_total - SLACK, -grow_in, F + P, P, F)
    return max(picks, key=key)
