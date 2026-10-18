#!/bin/bash
# Regression summary: replays (turns that match), and per player the city-turns whose non-tile yields match the game.
cd "$(dirname "$0")/.."
echo "replay rome $(python3 python_legacy/scripts/replay.py data/exports/game_20261005_p0.jsonl data/rules/gameplay_2026-10-05.sqlite 2>&1 | grep -c OK)/21"
echo "replay portugal $(python3 python_legacy/scripts/replay.py data/exports/game_auto2_p0.jsonl data/rules/gameplay_auto2.sqlite 2>&1 | grep -c OK) OK turns"
for p in 0 1 2 3; do
  o="--ai"; [ $p = 0 ] && o="--offset -1"
  python3 python_legacy/scripts/nontile_diff.py data/rules/gameplay_auto2.sqlite data/exports/game_auto2_p$p.jsonl data/probe/auto2_p$p.jsonl $o 2>&1 |
    awk -v p=$p '{n+=$1; if ($3=="{}") z+=$1} END {print "nontile p" p, z "/" n}'
done
python3 python_legacy/scripts/gpp_diff.py data/rules/gameplay_auto2.sqlite data/exports/game_auto2_p*.jsonl | head -1 | sed 's/^/great person points /'
