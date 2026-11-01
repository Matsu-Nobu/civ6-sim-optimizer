"""Compares what each harvest of the recorded play gave in the game with what the simulator gives it.

  chop_check.py <game data .json> [civ6opt options]

The simulator's amount is its HARVEST trace (CIV6OPT_TRACE=<city>:x) while it replays the record. The game's is read
from the saves around the harvest's turn t: the city's item progress at the next save, less its progress at this
save, less the production the city made (the next save's modified production yield). Only turns where the city
kept the same item and did not finish it count; several harvests of a city in one turn are summed on both sides.
"""
import collections
import json
import os
import re
import sys

import civ6opt


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    game, opts = sys.argv[1], sys.argv[2:] or ["--turn", "65", "--ext", "0", "--team", "--ally-supply"]
    g = json.load(open(game))
    rows = {r["turn"]: r for r in g["rows"]}
    cities = sorted({c["name"] for r in g["rows"] for c in r["cities"]})
    sim = collections.defaultdict(float)
    for c in cities:
        out = civ6opt.run(["run", game, *opts, "--genome", "record"], capture_output=True, text=True,
                          env={**os.environ, "CIV6OPT_TRACE": f"{c}:x"}).stderr
        seen = set()
        for line in out.splitlines():
            m = re.match(r"HARVEST T(\d+) (\S+) \((\d+), (\d+)\) \S+ \[(.*)\]", line)
            if not m or line in seen:
                continue
            seen.add(line)
            prod = re.search(r", ([\d.]+)\)", m.group(5))
            # the trace's yields are (Sym, amount); production is the only harvest yield of a forest or a stone
            sim[(int(m.group(1)), m.group(2))] += float(prod.group(1)) if prod else 0.0
    print("turn city          sim  game  item")
    diffs = []
    for (t, c), s in sorted(sim.items()):
        a, b = rows.get(t), rows.get(t + 1)
        if not a or not b:
            continue
        ca = next((x for x in a["cities"] if x["name"] == c), None)
        cb = next((x for x in b["cities"] if x["name"] == c), None)
        if not ca or not cb:
            continue
        item = ca["production"]["current"]
        if not item or cb["production"]["current"] != item:
            print(f"T{t:<3} {c:12s} {s:5.0f}     -  {item} (finished or changed)")
            continue
        i = [x["name"] for x in b["cities"]].index(c)
        made = b["cityYields"][i]["modified"]["YIELD_PRODUCTION"]
        game_amount = cb["production"]["progress"].get(item, 0) - ca["production"]["progress"].get(item, 0) - made
        diffs.append(game_amount - s)
        print(f"T{t:<3} {c:12s} {s:5.0f} {game_amount:5.0f}  {item}")
    if diffs:
        print(f"{len(diffs)} turns compared; game - sim: mean {sum(diffs) / len(diffs):.1f}, "
              f"sum {sum(diffs):.0f}, within 1: {sum(abs(d) <= 1 for d in diffs)}")


if __name__ == "__main__":
    main()
