"""Loads recorded saves in the game with the mod's MODE "probe" and collects what the game computes for a player's
cities right after loading (worked plots' yields, city yields, food surplus, amenities, item production): the game's
own answer for the state a save holds, to check the simulator's turn-start yields against.

  probe_run.py <game data name>:<turn> ...   e.g. probe_run.py persia_20261017:44 rome_20261021:32

The saves are copied to the single-player folder as EconProbe_<name>_T<turn>; the lines go to
data/probe/<name>_T<turn>.jsonl. Civ6 is launched only when it is not running, and only the process started here is
closed; the mod's MODE is set back to "off" afterwards.
"""
import json
import re
import shutil
import subprocess
import sys
import time
from pathlib import Path

import civ6opt

ROOT = civ6opt.ROOT
CIV = Path("/mnt/c/Users/nobus/Documents/My Games/Sid Meier's Civilization VI")
SAVES = CIV / "Saves" / "Single"
MOD = CIV / "Mods" / "Civ6EconAutoplay"
LOG = Path("/mnt/c/Users/nobus/AppData/Local/Firaxis Games/Sid Meier's Civilization VI/Logs/Lua.log")
MOD_SRC = ROOT / "game_mods" / "Civ6EconAutoplay"
# game data name -> (save directory, player id, civilization)
GAMES = {
    "rome_20261021": ("game_20261021", 2, "CIVILIZATION_ROME"),
    "france_20261024": ("game_20261024", 12, "CIVILIZATION_FRANCE"),
    "persia_20261017": ("game_20261017", 2, "CIVILIZATION_PERSIA"),
    "gran_colombia_20261019": ("game_20261019", 0, "CIVILIZATION_GRAN_COLOMBIA"),
    "france_20261014": ("game_20261014", 5, "CIVILIZATION_FRANCE"),
}


def sh(*args):
    return subprocess.run([str(a) for a in args], capture_output=True, text=True, errors="replace")


def civ_pids():
    return [line.split()[1] for line in sh("tasklist.exe").stdout.splitlines() if line.lower().startswith("civilizationvi")]


def lua(v):
    if isinstance(v, dict):
        return "{" + ", ".join(f"{k} = {lua(x)}" for k, x in v.items()) + "}"
    if isinstance(v, list):
        return "{" + ", ".join(lua(x) for x in v) + "}"
    return json.dumps(v)


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if civ_pids():
        sys.exit("Civ6 is already running: not launched")
    if any(x.lower().startswith("firaxisbugreporter") for x in sh("tasklist.exe").stdout.splitlines()):
        sys.exit("the game's bug reporter is open (a closed game's): close it first")
    probes = []
    for arg in sys.argv[1:]:
        name, turn = arg.split(":")
        d, pid, civ = GAMES[name]
        save = f"EconProbe_{name}_T{int(turn):03d}"
        shutil.copy(ROOT / "data" / "saves" / d / f"AutoSave_{int(turn):04d}.Civ6Save", SAVES / f"{save}.Civ6Save")
        # (a save's game turn is one less than the record's turn of it)
        probes.append({"name": save, "turn": int(turn), "game_turn": int(turn) - 1, "player": pid, "civ": civ, "game": name})
    for f in MOD_SRC.iterdir():
        shutil.copy(f, MOD / f.name)
    text = (MOD_SRC / "Civ6EconRunnerConfig.lua").read_text()
    text = text.replace('MODE = "off"', 'MODE = "probe"')
    text = re.sub(r'SAVE_NAME = "[^"]*"', f'SAVE_NAME = "{probes[0]["name"]}"', text)
    text = text.rstrip() + f"\nECONRUN.PROBE = {lua(probes)}\n"
    (MOD / "Civ6EconRunnerConfig.lua").write_text(text)
    start = LOG.stat().st_size if LOG.exists() else 0
    sh("powershell.exe", "-NoProfile", "-Command", 'Start-Process "steam://rungameid/289070"')
    pid = None
    for _ in range(60):
        if civ_pids():
            pid = civ_pids()[0]
            break
        time.sleep(3)
    try:
        if pid is None:
            sys.exit("the game did not start")
        t0, last, seen = time.time(), time.time(), 0
        while time.time() - t0 < 1800:
            time.sleep(5)
            body = LOG.read_bytes().decode("utf-8", "replace") if LOG.exists() else ""
            if len(body) < start:
                start = 0
            lines = [x for x in body[start:].splitlines() if "ECONPROBE " in x]
            if len(lines) != seen:
                seen, last = len(lines), time.time()
            if any('"done"' in x for x in lines) or time.time() - last > 420:
                break
    finally:
        if pid is not None:
            sh("taskkill.exe", "/PID", pid, "/F")
            # the game's bug reporter starts when the game is closed this way and keeps the next launch from starting
            for _ in range(10):
                time.sleep(3)
                reporters = [x.split()[1] for x in sh("tasklist.exe").stdout.splitlines() if x.lower().startswith("firaxisbugreporter")]
                for r in reporters:
                    sh("taskkill.exe", "/PID", r, "/F")
                if reporters:
                    break
        shutil.copy(MOD_SRC / "Civ6EconRunnerConfig.lua", MOD / "Civ6EconRunnerConfig.lua")
    out = ROOT / "data" / "probe"
    out.mkdir(parents=True, exist_ok=True)
    body = LOG.read_bytes().decode("utf-8", "replace")
    (out / "last_lua.log").write_text(body)
    for p in probes:
        rows = []
        for x in body.splitlines():
            i = x.find("ECONPROBE ")
            if i < 0:
                continue
            try:
                d = json.loads(x[i + 10:])
            except json.JSONDecodeError:
                continue
            if d.get("kind") in ("city", "player") and d.get("game") == p["game"] and d.get("turn") == p["game_turn"]:
                rows.append(d)
        (out / f"{p['game']}_T{p['turn']}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
        print(p["game"], p["turn"], sum(1 for r in rows if r["kind"] == "city"), "cities")
    print("\n".join(x[x.find("ECONPROBE "):] for x in body.splitlines() if "ECONPROBE " in x and '"city"' not in x)[-2000:])


if __name__ == "__main__":
    main()
