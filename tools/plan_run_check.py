"""Checks a plan run of the game mod (docs/plan.md): copies the run's saves, makes the run's game data and validates it
turn by turn, and compares the run with the plan's simulator turns.

  plan_run_check.py <run id> <start game data name> <plan turns .jsonl>
  e.g. plan_run_check.py mp1h econrun_mp1 data/search/econrun_mp1b/online1.turns.jsonl

The saves EconRun_<run id>_T*.Civ6Save go to data/saves/econrun_<run id>. A single-player save of turn t shows what a
multiplayer autosave of turn t - 1 shows (the one the validation expects), so the run's rows are numbered one turn
earlier, the first save (the start save again: the turn it was loaded processes nothing) left out, and the start
save's row put first. The game data is data/games/econrun_<run id>.json.
"""
import json
import shutil
import sys
from pathlib import Path

import civ6opt

SAVES = Path("/mnt/c/Users/nobus/Documents/My Games/Sid Meier's Civilization VI/Saves/Single")
DATA = civ6opt.ROOT / "data"


def main():
    if len(sys.argv) < 4:
        sys.exit(__doc__)
    run, start, plan_turns = sys.argv[1], sys.argv[2], Path(sys.argv[3])
    name = f"econrun_{run}"
    saves = DATA / "saves" / name
    saves.mkdir(parents=True, exist_ok=True)
    for f in SAVES.glob(f"EconRun_{run}_T*.Civ6Save"):
        shutil.copy(f, saves / f.name)
    rules = DATA / "rules" / f"{name}.sqlite"
    if not rules.exists():
        rules.symlink_to("rome_20261021.sqlite")
    out = civ6opt.run(["export", saves, 0, rules], capture_output=True, text=True, check=True).stdout
    rows = [json.loads(x) for x in out.splitlines()]
    start_rows = DATA / "exports" / f"game_{start}_p0.jsonl"
    lines = [start_rows.read_text().splitlines()[0] + "\n"]
    for r in rows:
        if r["turn"] == rows[0]["turn"]:
            continue
        r["turn"] -= 1
        lines.append(json.dumps(r) + "\n")
    prefix = DATA / "exports" / f"game_{name}"
    Path(f"{prefix}_p0.jsonl").write_text("".join(lines))
    shutil.copy(DATA / "exports" / f"game_{start}_map_T2.json", f"{prefix}_map_T2.json")
    shutil.copy(DATA / "exports" / f"game_{start}_mods.txt", f"{prefix}_mods.txt")
    game = DATA / "games" / f"{name}.json"
    civ6opt.run(["build", prefix, rules, game, "--player", 0, "--horizon", 64, "--builder-candidates", 8,
                 "--text", DATA / "rules" / "text_en.sqlite"], check=True, capture_output=True)
    val = civ6opt.run(["validate", game, "--turn", 64], capture_output=True, text=True).stdout.splitlines()
    print(val[-1][:400] if val else "validation printed nothing")
    for line in val[:-1]:
        if not line.endswith(" ok"):
            print("  " + line[:220])
    sim = {s["turn"]: s for s in map(json.loads, plan_turns.read_text().splitlines())}
    by_turn = {r["turn"]: r for r in rows}
    print("save | game: cities techs improvements policies | plan (turn - 2): cities techs improvements policies")
    for t in [10, 14, 18, 22, 26, 30, 34, 42, 50, 58, 66]:
        r, s = by_turn.get(t), sim.get(t - 2)
        if not r or not s:
            continue
        p = r["player"]
        imp = sum(1 for c in r["cities"] for q in c["owned"] if q.get("improvement"))
        print(f"T{t:<3} | {len(r['cities']):2d} {len(p['techs']):2d} {imp:3d} {sorted(p['policies'])} | "
              f"{len(s['cities']):2d} {s['techs']:2d} {sum(len(c['improvements']) for c in s['cities']):3d} {sorted(s['policies'])}")


if __name__ == "__main__":
    main()
