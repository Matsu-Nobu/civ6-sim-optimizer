#!/bin/bash
# Usage: python_legacy/scripts/explain_city.sh <game> <player> <city> <pattern-substring>   (e.g. rome1 1 XANADU "'S': -2.0}")
# Shows the first turn matching the pattern: our non-tile parts and the game's yield tooltips.
cd "$(dirname "$0")/.."
G=$1; P=$2; C=$3; PAT=$4
R=data/rules/gameplay_$G.sqlite; [ -f $R ] || R=data/rules/gameplay_auto2.sqlite
E=data/exports/game_${G}_p$P.jsonl; PR=data/probe/${G}_p$P.jsonl
AI=--ai; [ "$P" = 0 ] && AI=""
T=$(python3 python_legacy/scripts/nontile_diff.py $R $E $PR $AI --city $C 2>/dev/null | grep -F "$PAT" | head -1 | awk '{print $1}')
[ -z "$T" ] && { echo "no turn matches"; exit 1; }
python3 python_legacy/scripts/nontile_diff.py $R $E $PR $AI --city $C --turn $T 2>/dev/null
python3 - "$PR" "$T" "$C" <<'PY'
import json, sys
pr, t, c = sys.argv[1], int(sys.argv[2]), sys.argv[3]
for l in open(pr):
    d = json.loads(l)
    if d["turn"] == t:
        city = next(x for x in d["cities"] if c in x["name"])
        for k, v in (city.get("yieldTips") or {}).items():
            print("  game", k[6:], ":", v.replace("[NEWLINE]", " | "))
        print("  game districts:", [(x[0][9:], {k[6:9]: y for k, y in x[3].items() if y}) for x in city.get("districtYields") or []])
PY
