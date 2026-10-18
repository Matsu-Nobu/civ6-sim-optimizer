"""Copy every new Civ6 autosave into data/saves/raw/ so none are lost to rotation, and keep the Econ Probe
mod's lines from Lua.log (which the game overwrites on each launch) in data/probe/econprobe.jsonl.

Usage: python3 python_legacy/scripts/watch_autosaves.py
Files are named <mode>_<YYYYmmdd-HHMMSS>_<original name> so turns from different games never collide.
"""
import shutil
import time
from datetime import datetime
import subprocess
from pathlib import Path

# the Windows user folder, as WSL sees it
WIN_HOME = Path(subprocess.run(["wslpath", subprocess.run(["cmd.exe", "/c", "echo %USERPROFILE%"], capture_output=True, text=True).stdout.strip()], capture_output=True, text=True).stdout.strip())
SAVES = WIN_HOME / "Documents/My Games/Sid Meier's Civilization VI/Saves"
DEST = Path(__file__).resolve().parent.parent / "data" / "saves" / "raw"
WATCH = {"multi": SAVES / "Multi" / "auto", "single": SAVES / "Single" / "auto"}
POLL_SECONDS = 2
LUA_LOG = WIN_HOME / "AppData/Local/Firaxis Games/Sid Meier's Civilization VI/Logs/Lua.log"
PROBE_OUT = Path(__file__).resolve().parent.parent / "data" / "probe" / "econprobe.jsonl"


def collect_probe(seen):
    """Append new ECONPROBE lines from Lua.log (dedup by content, since the log is rewritten per launch)."""
    try:
        text = LUA_LOG.read_text(errors="replace")
    except OSError:
        return
    new = []
    events = []
    mods = []
    parts = {}  # id -> list of chunks; the game cuts log lines at 2047 characters, so the mod writes parts
    for line in text.splitlines():
        j = line.find("ECONPROBE_EVENT ")
        if j >= 0:
            payload = line[j + len("ECONPROBE_EVENT "):].strip()
            if payload not in seen:
                seen.add(payload)
                events.append(payload)
            continue
        i = line.find("ECONPROBE_PART ")
        tag = "ECONPROBE_PART "
        if i < 0:
            i = line.find("ECONPROBE_MODS ")
            tag = "ECONPROBE_MODS "
        if i < 0:
            continue
        fields = line[i + len(tag):].split(" ", 3)
        if len(fields) < 4:
            continue
        pid, k, n, chunk = fields[0], int(fields[1]), int(fields[2]), fields[3]
        if k == 1:
            parts[pid] = []
        if pid not in parts:
            continue
        parts[pid].append(chunk)
        if k == n:
            payload = "".join(parts.pop(pid))
            if payload not in seen:
                seen.add(payload)
                (mods if pid.startswith("mods_") else new).append(payload)
    if mods:
        PROBE_OUT.parent.mkdir(parents=True, exist_ok=True)
        with PROBE_OUT.with_name("econprobe_mods.jsonl").open("a") as f:
            for x in mods:
                f.write(x + "\n")
        print(f"probe: {len(mods)} modifier dump(s)", flush=True)
    if events:
        PROBE_OUT.parent.mkdir(parents=True, exist_ok=True)
        with PROBE_OUT.with_name("econprobe_events.jsonl").open("a") as f:
            for x in events:
                f.write(x + "\n")
        print(f"probe: {len(events)} new event(s)", flush=True)
    if new:
        PROBE_OUT.parent.mkdir(parents=True, exist_ok=True)
        with PROBE_OUT.open("a") as f:
            for x in new:
                f.write(x + "\n")
        print(f"probe: {len(new)} new line(s)", flush=True)


def snapshot():
    out = {}
    folders = list(WATCH.items()) + [("autoplay", SAVES / "Single")]
    for mode, folder in folders:
        pattern = "Econ*_T*.Civ6Save" if mode == "autoplay" else "*.Civ6Save"
        for f in folder.glob(pattern):
            try:
                out[f] = (mode, f.stat().st_mtime, f.stat().st_size)
            except FileNotFoundError:
                pass
    return out


def main():
    DEST.mkdir(parents=True, exist_ok=True)
    seen = snapshot()
    probe_seen = set(PROBE_OUT.read_text().splitlines()) if PROBE_OUT.exists() else set()
    print(f"watching {', '.join(str(p) for p in WATCH.values())} -> {DEST}", flush=True)
    while True:
        time.sleep(POLL_SECONDS)
        collect_probe(probe_seen)
        now = snapshot()
        for f, (mode, mtime, size) in now.items():
            if seen.get(f, (None, None, None))[1:] == (mtime, size):
                continue
            time.sleep(1)  # let the game finish writing
            stamp = datetime.fromtimestamp(mtime).strftime("%Y%m%d-%H%M%S")
            target = DEST / f"{mode}_{stamp}_{f.name}"
            try:
                shutil.copyfile(f, target)
                print(f"copied {target.name}", flush=True)
            except OSError as e:
                print(f"failed {f.name}: {e}", flush=True)
                continue
            now[f] = (mode, f.stat().st_mtime, f.stat().st_size)
        seen = now


if __name__ == "__main__":
    main()
