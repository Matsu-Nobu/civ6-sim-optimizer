#!/bin/bash
# Every player at the end of T50 (the save that starts T51; T49 for 10/16c, whose last save starts T50) in the games
# that got there, 2 exports at a time. Save copies and raw exports: data/saves/picks/t50_all.
cd "$(dirname "$0")/../../.."
B=optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt
R=data/rules/gameplay_2026-10-19.sqlite
P=data/saves/picks/t50_all
mkdir -p $P/exports
for g in 20261014 20261016b 20261016c 20261017 20261019 20261019a; do
  s=$(ls data/saves/game_$g | grep -E 'AutoSave_0051\.Civ6Save$' | head -1)
  [ -z "$s" ] && s=$(ls data/saves/game_$g | grep Civ6Save | tail -1)
  mkdir -p $P/$g; cp --update=none data/saves/game_$g/$s $P/$g/
  seq 0 23 | xargs -P 2 -I{} sh -c "$B export $P/$g {} $R > $P/exports/${g}_p{}.jsonl 2>/dev/null"
done
python3 tools/turn_stats.py data/saves/picks/t50_all/exports data/search/t50/all.tsv --majors
