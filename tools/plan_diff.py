"""Compares the simulator's turns of a plan (tools/plan_lua.py) with the saves of the game that played it, and prints
each kind of difference once: the first turn it shows, how many turns show it, and that first case.

  plan_diff.py <turns .jsonl> <saves dir | game data .json> [--rules data/rules/<game>.sqlite] [--player 0]
               [--log Lua.log] [--all]

The simulator's turn t gives the worked plots and item of the city in turn t, which the save of turn t shows, and
the population, food store, item progress, techs and gold after the turn, which the save of turn t + 1 shows. The
yields are not compared: a save shows those before the turn's improvements and harvests, the simulator those after. A game data file is read in place of saves (its rows: the record replay against its own saves).
--log adds the mod's reports (ECONPLAN lines: orders the game refused or the mod does not play). --all prints every
case, not only the first of each kind.
"""
import argparse
import collections
import json
from pathlib import Path

import civ6opt

def rows_of(source, rules, player):
    if source.endswith(".json"):
        return {r["turn"]: r for r in json.load(open(source))["rows"]}
    out = civ6opt.run(["export", source, player, rules], capture_output=True, text=True, check=True).stdout
    return {r["turn"]: r for r in map(json.loads, out.splitlines())}


def compare(sim, rows):
    """[(kind, turn, text)] for each difference."""
    diffs = []
    for t, s in sorted(sim.items()):
        now, after = rows.get(t), rows.get(t + 1)
        if now is None or after is None:
            continue
        for c in s["cities"]:
            xy = tuple(c["xy"])
            i = next((k for k, x in enumerate(now["cities"]) if (x["center"]["x"], x["center"]["y"]) == xy), None)
            j = next((x for x in after["cities"] if (x["center"]["x"], x["center"]["y"]) == xy), None)
            if i is None or j is None:
                diffs.append(("city missing in the game", t, f"{c['name']} {xy}"))
                continue
            g = now["cities"][i]
            worked = {(w["x"], w["y"]) for w in g["worked"]}
            sim_worked = {tuple(w) for w in c["worked"]}
            if worked != sim_worked:
                diffs.append(("worked plots", t, f"{c['name']} only sim {sorted(sim_worked - worked)} only game {sorted(worked - sim_worked)}"))
            if c["item"] and g["production"]["current"] != c["item"]:
                diffs.append(("item", t, f"{c['name']} sim {c['item']} game {g['production']['current']}"))
            if c["pop"] != j["population"]:
                diffs.append(("population", t, f"{c['name']} sim {c['pop']} game {j['population']}"))
            elif abs(c["food_store"] - j["foodStore"]) > 0.5:
                diffs.append(("food store", t, f"{c['name']} sim {c['food_store']:g} game {j['foodStore']:g}"))
            game_prog = j["production"]["progress"].get(c["item"]) if c["item"] else None
            if c["item"] and game_prog is not None and abs(c["item_progress"] - game_prog) > 0.5:
                diffs.append(("item progress", t, f"{c['name']} {c['item']} sim {c['item_progress']:g} game {game_prog:g}"))
        p = after["player"]
        sim_techs, game_techs = set(s["tech_list"]), set(p["techs"])
        if sim_techs != game_techs:
            diffs.append(("techs", t, f"only sim {sorted(sim_techs - game_techs)} only game {sorted(game_techs - sim_techs)}"))
        if abs(s["gold"] - p["gold"]) > 1:
            diffs.append(("gold", t, f"sim {s['gold']:g} game {p['gold']:g}"))
        if s["government"] != p["government"]:
            diffs.append(("government", t, f"sim {s['government']} game {p['government']}"))
    return diffs


def mod_reports(log):
    """[(kind, turn, text)] of the mod's ECONPLAN lines other than its routine ones."""
    routine = {"tick", "city", "player", "unit_op", "unit_done", "buy", "route", "governor", "dedication", "plan_start",
               "resolving", "diplomacy_closed"}
    out = []
    for line in open(log, encoding="utf-8", errors="replace"):
        i = line.find("ECONPLAN ")
        if i < 0:
            continue
        try:
            d = json.loads(line[i + 9:])
        except json.JSONDecodeError:
            continue
        if d.get("kind") not in routine:
            out.append((f"mod: {d['kind']}", d.get("turn", 0), json.dumps({k: v for k, v in d.items() if k not in ("kind", "turn")})))
    return out


def main():
    ap = argparse.ArgumentParser(description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter)
    ap.add_argument("turns")
    ap.add_argument("source")
    ap.add_argument("--rules", default=str(civ6opt.ROOT / "data" / "rules" / "rome_20261021.sqlite"))
    ap.add_argument("--player", default="0")
    ap.add_argument("--log")
    ap.add_argument("--all", action="store_true")
    a = ap.parse_args()
    sim = {s["turn"]: s for s in map(json.loads, Path(a.turns).read_text().splitlines())}
    rows = rows_of(a.source, a.rules, a.player)
    diffs = compare(sim, rows)
    if a.log:
        diffs += mod_reports(a.log)
    kinds = collections.OrderedDict()
    for kind, t, text in sorted(diffs, key=lambda d: d[1]):
        kinds.setdefault(kind, []).append((t, text))
    compared = sum(1 for t in sim if t in rows and t + 1 in rows)
    print(f"{compared} simulator turns compared; {len(diffs)} differences of {len(kinds)} kinds")
    for kind, cases in kinds.items():
        turns = sorted({t for t, _ in cases})
        print(f"{kind}: first T{turns[0]}, {len(turns)} turns, {len(cases)} cases; T{cases[0][0]} {cases[0][1]}")
        if a.all:
            for t, text in cases[1:]:
                print(f"    T{t} {text}")


if __name__ == "__main__":
    main()
