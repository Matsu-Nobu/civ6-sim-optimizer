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


def only_human(path, player):
    """Rewrites the slot status in a save's header so that `player` is the only human (taken) slot and the other
    human slots are computer ones: loaded as a single-player game, the first human slot is the local player."""
    import struct
    import zlib
    b = bytearray(path.read_bytes())
    key = lambda name: struct.pack("<I", zlib.crc32(name.encode()) ^ 0xFFFFFFFF)
    def entries(name):
        # an entry: the key, type 2 (a 32-bit number), 8 bytes, the value
        out, i = [], b.find(key(name))
        while i >= 0:
            if struct.unpack("<I", b[i + 4:i + 8])[0] == 2:
                out.append((i, struct.unpack("<I", b[i + 16:i + 20])[0]))
            i = b.find(key(name), i + 1)
        return out
    ids = entries("PLAYER_ID")
    changed = 0
    for i, status in entries("SLOT_STATUS"):
        # the slot's PLAYER_ID entry comes shortly before its SLOT_STATUS entry
        owner = max((p for p in ids if i - 1200 < p[0] < i), default=None)
        # (SlotStatus: 1 computer, 3 taken by a human)
        if status == 3 and owner is not None and owner[1] != player:
            b[i + 16:i + 20] = struct.pack("<I", 1)
            changed += 1
    path.write_bytes(bytes(b))
    return changed


def probe_records(body):
    """The probe's records in the log; a long one comes as "ECONPART" pieces before its last "ECONPROBE" piece."""
    out, parts = [], ""
    for x in body.splitlines():
        i = x.find("ECONPART ")
        if i >= 0:
            parts += x[i + 9:]
            continue
        i = x.find("ECONPROBE ")
        if i < 0:
            continue
        text, parts = parts + x[i + 10:], ""
        try:
            out.append(json.loads(text, strict=False))
        except json.JSONDecodeError:
            print("unreadable probe record:", text[:200])
    return out


def main():
    if len(sys.argv) < 2:
        sys.exit(__doc__)
    if civ_pids():
        sys.exit("Civ6 is already running: not launched")
    if any(x.lower().startswith("firaxisbugreporter") for x in sh("tasklist.exe").stdout.splitlines()):
        sys.exit("the game's bug reporter is open (a closed game's): close it first")
    args = sys.argv[1:]
    # --boosts: the boost experiment (Civ6EconBoostTest.lua) on the first save given
    boosts = "--boosts" in args
    args = [a for a in args if a != "--boosts"]
    # --harvest '<json>': the harvest experiment (Civ6EconAutoplay.lua harvest_test), {"steps": [{"x":..,"y":..} or
    # {"city": NAME, "choose": UNIT}]}, as the probed player of the first save, made its only human
    harvest = None
    if "--harvest" in args:
        k = args.index("--harvest")
        harvest = json.loads(args[k + 1])
        args = args[:k] + args[k + 2:]
    probes = []
    for arg in args:
        name, turn = arg.split(":")
        d, pid, civ = GAMES[name]
        save = f"EconProbe_{name}_T{int(turn):03d}"
        shutil.copy(ROOT / "data" / "saves" / d / f"AutoSave_{int(turn):04d}.Civ6Save", SAVES / f"{save}.Civ6Save")
        if harvest:
            print("slots made computer:", only_human(SAVES / f"{save}.Civ6Save", pid))
        # (a save's game turn is one less than the record's turn of it)
        probes.append({"name": save, "turn": int(turn), "game_turn": int(turn) - 1, "player": pid, "civ": civ, "game": name})
    for f in MOD_SRC.iterdir():
        shutil.copy(f, MOD / f.name)
    text = (MOD_SRC / "Civ6EconRunnerConfig.lua").read_text()
    text = text.replace('MODE = "off"', 'MODE = "probe"')
    text = re.sub(r'SAVE_NAME = "[^"]*"', f'SAVE_NAME = "{probes[0]["name"]}"', text)
    text = text.rstrip() + f"\nECONRUN.PROBE = {lua(probes)}\n"
    if boosts:
        text += "ECONRUN.BOOSTS = true\n"
    if harvest:
        harvest["player"] = probes[0]["player"]
        text += f"ECONRUN.HARVEST = {lua(harvest)}\n"
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
            boost_done = (not boosts or "ECONBOOST {\"done\"" in body[start:]) and (not harvest or '"harvest_done"' in body[start:])
            if (any('"done"' in x for x in lines) and boost_done) or time.time() - last > 420:
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
    records = probe_records(body)
    if harvest:
        rows = [d for d in records if str(d.get("kind", "")).startswith("harvest")]
        (out / f"harvest_{probes[0]['game']}_T{probes[0]['turn']}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
        print(len(rows), "harvest records")
    if boosts:
        rows = [json.loads(x[x.find("ECONBOOST ") + 10:].replace("\\", "/")) for x in body.splitlines() if "ECONBOOST " in x]
        rows += [d for d in records if d.get("kind") == "civics"]
        (out / f"boosts_{probes[0]['game']}_T{probes[0]['turn']}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
        print(len(rows) - 1, "boosts")
    for p in probes:
        rows = [d for d in records
                if d.get("kind") in ("city", "player") and d.get("game") == p["game"] and d.get("turn") == p["game_turn"]]
        (out / f"{p['game']}_T{p['turn']}.jsonl").write_text("".join(json.dumps(r) + "\n" for r in rows))
        print(p["game"], p["turn"], sum(1 for r in rows if r["kind"] == "city"), "cities")
    print("\n".join(json.dumps(d) for d in records if d.get("kind") not in ("city", "player")))


if __name__ == "__main__":
    main()
