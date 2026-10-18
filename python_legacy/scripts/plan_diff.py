"""Compare a replayed plan (ECONPLAN lines in Lua.log) with the simulator's trace; one line per kind of difference
(the first turn it appears and how often), so a run is read in a few lines.

Usage: python3 python_legacy/scripts/plan_diff.py <Lua.log> data/plans/<name>_trace.json [--all]
"""
import json
import sys
from collections import defaultdict


def main():
    log_path, trace_path = sys.argv[1], sys.argv[2]
    show_all = "--all" in sys.argv
    offset = int(sys.argv[sys.argv.index("--offset") + 1]) if "--offset" in sys.argv else 1  # game turn = sim turn + offset
    game = defaultdict(dict)  # turn -> city xy -> report
    player = {}
    events = defaultdict(list)
    for line in open(log_path, encoding="utf-8", errors="replace"):
        i = line.find("ECONPLAN ")
        if i < 0:
            continue
        try:
            ev = json.loads(line[i + 9:].strip())
        except json.JSONDecodeError:
            events["unparsed"].append(line.strip()[:120])
            continue
        if ev["kind"] == "city":
            game[ev["turn"]].setdefault(tuple(ev["city"]), ev)  # the first report: before the turn is ended
        elif ev["kind"] == "player":
            player.setdefault(ev["turn"], ev)
        else:
            events[ev["kind"]].append(ev)
    trace = json.load(open(trace_path))
    diffs = defaultdict(list)  # kind -> [(turn, detail)]
    for ev in trace:
        if ev[0] != "turn":
            continue
        t, s = ev[1] + offset, ev[2]
        if t not in player:
            continue
        g = player[t]
        for k in ("research", "civic"):
            if s[k] and g.get(k) != s[k]:
                diffs[k].append((t, f"sim {s[k]} game {g.get(k)}"))
        for c in s["cities"]:
            xy = tuple(c["xy"])
            gc = game[t].get(xy)
            if gc is None:
                diffs["city missing"].append((t, f"{c['name']} {xy}"))
                continue
            if c["item"] and gc.get("item") != c["item"]:
                diffs["production item"].append((t, f"{c['name']} sim {c['item']} game {gc.get('item')}"))
            if gc.get("pop") != c["pop"]:
                diffs["population"].append((t, f"{c['name']} sim {c['pop']} game {gc.get('pop')}"))
            sw = {(w[0], w[1]): w[2] for w in c["worked"] if (w[0], w[1]) != xy}
            gw = {(w[0], w[1]): w[2] for w in gc.get("worked") or [] if (w[0], w[1]) != xy}
            if sw != gw:
                diffs["worked plots"].append((t, f"{c['name']} sim-only {sorted(set(sw.items()) - set(gw.items()))} "
                                                 f"game-only {sorted(set(gw.items()) - set(sw.items()))}"))
            for y in ("YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"):
                a, b = c["yields"].get(y, 0), (gc.get("yields") or {}).get(y)
                if b is None or abs(a - b) > 1e-6:
                    diffs[y].append((t, f"{c['name']} sim {a:g} game {b}"))
    turns = sorted(player)
    print(f"turns replayed {turns[0]}..{turns[-1]}" if turns else "no turns replayed")
    for kind, lst in sorted(events.items()):
        if kind in ("plan_start", "unit_op", "buy"):
            continue
        print(f"[run] {kind} x{len(lst)}: first T{lst[0].get('turn')} {json.dumps({k: v for k, v in lst[0].items() if k not in ('kind', 'turn')})[:160]}")
    for kind, lst in diffs.items():
        print(f"[diff] {kind} x{len(lst)}: first T{lst[0][0]} {lst[0][1][:200]}")
        if show_all:
            for t, d in lst[1:]:
                print(f"        T{t} {d[:200]}")
    if not diffs:
        print("no differences")


if __name__ == "__main__":
    main()
