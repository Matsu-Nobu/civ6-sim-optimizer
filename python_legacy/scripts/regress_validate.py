"""Regression: civ6opt validate (each recorded turn from its save, one turn, compared with the next save) over every
recorded game and player. Builds what is missing first: the start map (python_legacy/scripts/export_map.sh, with mod resources) and
a problem file per player (python_legacy/scripts/export_problem.py --no-genome).

Usage: python3 python_legacy/scripts/regress_validate.py [game ...] [--rebuild] [--all-players]
Only the human slot (player 0) by default: AI players get difficulty bonuses (HIGH_DIFFICULTY_*_SCALING by the game's
handicap, which the exports lack), not modelled yet.
Prints per game and player: turns without model errors / turns, and the differences by kind.
"""
import glob
import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
BIN = ROOT / "optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# game -> rules database (the game's own DebugGameplay copy)
GAMES = {
    "game_20261005": "gameplay_2026-10-05",
    "game_20261006": "gameplay_2026-10-06",
    "game_20261007a": "gameplay_2026-10-08",
    "game_20261007b": "gameplay_2026-10-08",
    "game_auto2": "gameplay_auto2",
    "game_auto3": "gameplay_2026-10-06",
    "game_auto4": "gameplay_2026-10-06",
    "game_rome1": "gameplay_rome1",
    "game_rome2": "gameplay_rome2",
}


def run(cmd, out=None):
    with open(out, "w") if out else open(os.devnull, "w") as f:
        r = subprocess.run(cmd, stdout=f if out else subprocess.PIPE, stderr=subprocess.PIPE, text=True)
    return r


def first_row(path):
    """The first recorded turn where the player has a city (and that turn)."""
    for line in open(path):
        d = json.loads(line)
        if d["cities"]:
            return d["turn"]
    return None


def main():
    rebuild = "--rebuild" in sys.argv
    all_players = "--all-players" in sys.argv
    games = [a for a in sys.argv[1:] if not a.startswith("--")] or list(GAMES)
    outdir = ROOT / "data/games/regress"
    outdir.mkdir(parents=True, exist_ok=True)
    for g in games:
        db = ROOT / f"data/rules/{GAMES[g]}.sqlite"
        players = sorted(int(re.search(r"_p(\d+)\.jsonl$", p).group(1)) for p in glob.glob(str(ROOT / f"data/exports/{g}_p*.jsonl")))
        for pl in players if all_players else [p for p in players if p == 0]:
            rows = ROOT / f"data/exports/{g}_p{pl}.jsonl"
            start = first_row(rows)
            if start is None:
                continue
            last = max(json.loads(l)["turn"] for l in open(rows))
            mp = ROOT / f"data/exports/{g}_map_T{start}.json"
            if rebuild or not mp.exists() or "riverEdges" not in mp.read_text()[:2000]:
                saves = sorted(glob.glob(str(ROOT / f"data/saves/{g}/*")))
                save = next((s for s in saves if re.search(rf"(T0*{start}|AutoSave_0*{start})\.Civ6Save$", s)), None)
                if save is None:
                    print(f"{g} p{pl}: no save of T{start} for the map")
                    continue
                run(["bash", str(ROOT / "python_legacy/scripts/export_map.sh"), save, str(db)], out=str(mp))
            prob = outdir / f"{g}_p{pl}.json"
            if rebuild or not prob.exists():
                r = run(["nice", "-n", "19", "python3", str(ROOT / "python_legacy/scripts/export_problem.py"), str(ROOT / f"data/exports/{g}"), str(db), str(prob),
                         "--start", str(start), "--horizon", str(last), "--player", str(pl), "--no-genome"])
                if not prob.exists():
                    print(f"{g} p{pl}: problem export failed: {r.stderr.strip().splitlines()[-1:]}")
                    continue
            r = subprocess.run([str(BIN), "validate", str(prob), "--turn", str(last), "--from", str(start)], capture_output=True, text=True)
            tail = (r.stdout.strip().splitlines() or ["(no output)"])[-1]
            if r.returncode != 0:
                tail = "FAILED: " + (r.stderr.strip().splitlines() or ["?"])[-1]
            print(f"{g} p{pl} T{start}-{last}: {tail}", flush=True)


if __name__ == "__main__":
    main()
