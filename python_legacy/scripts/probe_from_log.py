"""Rebuild one player's probe rows (ECONPROBE_PART lines) from a saved Lua.log.

Usage: python3 python_legacy/scripts/probe_from_log.py <Lua.log> <player> > out.jsonl
"""
import json
import sys


def main():
    log_path, player = sys.argv[1], int(sys.argv[2])
    parts, rows = {}, {}
    for line in open(log_path, errors="replace"):
        i = line.find("ECONPROBE_PART ")
        if i < 0:
            continue
        f = line[i + 15:].rstrip("\n").split(" ", 3)
        if len(f) < 4:
            continue
        pid, k, n, chunk = f[0], int(f[1]), int(f[2]), f[3]
        if k == 1:
            parts[pid] = []
        if pid not in parts:
            continue
        parts[pid].append(chunk)
        if k == n:
            try:
                r = json.loads("".join(parts.pop(pid)))
            except json.JSONDecodeError:
                continue
            if r.get("player") == player:
                rows.setdefault(r["turn"], r)  # the first dump of a turn (turn begin)
    for t in sorted(rows):
        print(json.dumps(rows[t]))


if __name__ == "__main__":
    main()
