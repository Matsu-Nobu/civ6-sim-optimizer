#!/bin/bash
# The user's player at the end of T50 (the save that starts T51; the last save if the game stopped before) in each
# game that got near it, summarized in stats.tsv. Save copies and raw exports: data/saves/picks/t50.
cd "$(dirname "$0")/../../.."
B=optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt
R=data/rules/gameplay_2026-10-19.sqlite
P=data/saves/picks/t50
mkdir -p $P/exports
for x in 20261014:5 20261016b:0 20261016c:0 20261017:2 20261019:0 20261019a:0; do
  g=${x%:*}; p=${x#*:}
  s=$(ls data/saves/game_$g | grep -E 'AutoSave_0051\.Civ6Save$' | head -1)
  [ -z "$s" ] && s=$(ls data/saves/game_$g | grep Civ6Save | tail -1)
  mkdir -p $P/$g; cp --update=none data/saves/game_$g/$s $P/$g/
  $B export $P/$g $p $R > $P/exports/${g}_p$p.jsonl 2>/dev/null
done
python3 tools/turn_stats.py $P/exports data/search/t50/stats.tsv
