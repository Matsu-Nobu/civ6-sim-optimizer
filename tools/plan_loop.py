"""Plays plans in the game in segments, each planned from the game's actual state (docs/plan.md): the game's state
drifts from a plan made at the start (border plots are chosen at random), so every few turns the simulator plans
again from where the game is, and the mod plays that plan.

  plan_loop.py <run id> [--segment 8] [--last 65] [-- civ6opt search options]

Each segment: a plan is searched (civ6opt online) from the segment's start, written as the mod's plan file from the
segment's first turn on, and played in the game until the resume save of the segment's last turn exists; then the game
is closed. The first segment starts from EconPlanStart_mp1_T002 (game data econrun_mp1); the next ones from the
resume save EconRun_<run id>_R<turn> the mod writes when it has given a turn's orders and before it ends the turn, so
the game goes on from there with nothing lost. Civ6 is launched only when it is not running, and only the process
started here is closed.
"""
import json
import shutil
import subprocess
import sys
import time
from pathlib import Path

import civ6opt

ROOT = civ6opt.ROOT
DATA = ROOT / "data"
CIV = Path("/mnt/c/Users/nobus/Documents/My Games/Sid Meier's Civilization VI")
SAVES = CIV / "Saves" / "Single"
MOD = CIV / "Mods" / "Civ6EconAutoplay"
LOG = Path("/mnt/c/Users/nobus/AppData/Local/Firaxis Games/Sid Meier's Civilization VI/Logs/Lua.log")
MOD_SRC = ROOT / "game_mods" / "Civ6EconAutoplay"
RULES = DATA / "rules" / "rome_20261021.sqlite"
SEARCH = ["--turn", "65", "--ext", "20", "--ext-model", "--fallback-model", str(DATA / "search" / "econ21f" / "m1.json")]


def sh(*args, **kw):
    return subprocess.run([str(a) for a in args], capture_output=True, text=True, errors="replace", **kw)


def civ_pids():
    out = sh("tasklist.exe").stdout
    return [line.split()[1] for line in out.splitlines() if line.lower().startswith("civilizationvi")]


def config(run, save_name, mode):
    text = (MOD_SRC / "Civ6EconRunnerConfig.lua").read_text()
    text = text.replace('MODE = "off"', f'MODE = "{mode}"')
    import re
    text = re.sub(r'SAVE_NAME = "[^"]*"', f'SAVE_NAME = "{save_name}"', text)
    text = re.sub(r'RUN_ID = "[^"]*"', f'RUN_ID = "{run}"', text)
    text = text.replace("CITY_STATES = 18,", "CITY_STATES = 18, RESUME_SAVES = true,")
    (MOD / "Civ6EconRunnerConfig.lua").write_text(text)


def mod_off():
    shutil.copy(MOD_SRC / "Civ6EconRunnerConfig.lua", MOD / "Civ6EconRunnerConfig.lua")


def play(run, save_name, until_turn):
    """Plays the installed plan from save_name until the resume save of until_turn (or the last turn's save)."""
    if civ_pids():
        sys.exit("Civ6 is already running: not launched")
    for f in MOD_SRC.iterdir():
        shutil.copy(f, MOD / f.name)
    config(run, save_name, "plan")
    sh("powershell.exe", "-NoProfile", "-Command", 'Start-Process "steam://rungameid/289070"')
    pid = None
    for _ in range(60):
        p = civ_pids()
        if p:
            pid = p[0]
            break
        time.sleep(3)
    if pid is None:
        mod_off()
        sys.exit("the game did not start")
    target = SAVES / f"EconRun_{run}_R{until_turn:03d}.Civ6Save"
    last_end = SAVES / f"EconRun_{run}_T066.Civ6Save"
    seen, changed, t0 = None, time.time(), time.time()
    while not target.exists() and not last_end.exists():
        newest = max(SAVES.glob(f"EconRun_{run}_*.Civ6Save"), key=lambda f: f.stat().st_mtime, default=None)
        if newest != seen:
            seen, changed = newest, time.time()
        if time.time() - changed > 240 and seen is not None or time.time() - t0 > 2400:
            print(f"  stalled after {seen}")
            break
        time.sleep(5)
    time.sleep(5)
    sh("taskkill.exe", "/PID", pid, "/F")
    shutil.copy(LOG, DATA / "saves" / f"lua_{run}_{until_turn}.log")
    mod_off()
    return target.exists()


def game_data_from(run, turn):
    """The game data of a segment starting at the resume save of `turn`."""
    name = f"loop_{run}_{turn}"
    d = DATA / "saves" / name
    d.mkdir(parents=True, exist_ok=True)
    src = SAVES / f"EconRun_{run}_R{turn:03d}.Civ6Save"
    shutil.copy(src, d / src.name)
    rows = sh(civ6opt.binary(), "export", d, 0, RULES, check=True).stdout
    prefix = DATA / "exports" / f"game_{name}"
    Path(f"{prefix}_p0.jsonl").write_text(rows)
    Path(f"{prefix}_map_T{turn}.json").write_text(sh(civ6opt.binary(), "map", d / src.name, RULES, check=True).stdout)
    shutil.copy(DATA / "exports" / "game_econrun_mp1_mods.txt", f"{prefix}_mods.txt")
    game = DATA / "games" / f"{name}.json"
    sh(civ6opt.binary(), "build", prefix, RULES, game, "--player", 0, "--start", turn, "--horizon", 65,
       "--builder-candidates", 8, "--text", DATA / "rules" / "text_en.sqlite", check=True)
    return game


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    args = sys.argv[1:]
    extra = args[args.index("--") + 1:] if "--" in args else []
    args = args[:args.index("--")] if "--" in args else args
    run = args[0]
    seg = int(args[args.index("--segment") + 1]) if "--segment" in args else 8
    last = int(args[args.index("--last") + 1]) if "--last" in args else 65
    out = DATA / "search" / f"loop_{run}"
    out.mkdir(parents=True, exist_ok=True)
    start, save_name, game = 2, "EconPlanStart_mp1_T002", DATA / "games" / "econrun_mp1.json"
    while start <= last:
        end = min(start + seg - 1, last)
        plan = out / f"plan_{start}.json"
        print(f"segment {start}..{end}: planning", flush=True)
        if not plan.exists():
            sh(civ6opt.binary(), "online", game, *SEARCH, *extra, "--top", 4, "--threads", 14, "--out", plan)
        first = start if start == 2 else start + 1
        sh(sys.executable, ROOT / "tools" / "plan_lua.py", game, plan, out / f"turns_{start}.jsonl", "--from-turn", first,
           "--turn", 65, "--ext", 0, check=True)
        print(f"  playing from {save_name} to the resume save of T{end}", flush=True)
        if not play(run, save_name, end):
            print("  the segment did not reach its end; stopping", flush=True)
            break
        start, save_name = end, f"EconRun_{run}_R{end:03d}"
        game = game_data_from(run, end)
    print("done", flush=True)


if __name__ == "__main__":
    main()
