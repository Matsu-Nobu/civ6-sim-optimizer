#!/bin/bash
# For each multiplayer game: export the save that starts T31 (= the end of T30) and the last save, for every player
# id 0..23 (4 exports at a time), then summarize the major civilizations in stats.tsv. The save copies and the raw
# exports go to data/saves/picks/t30_stats (not in the repository: they hold the other players' names).
cd "$(dirname "$0")/../../.."
B=optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt
R=data/rules/gameplay_2026-10-19.sqlite
P=data/saves/picks/t30_stats
mkdir -p $P/exports
for g in "${@:-20261005 20261013 20261014 20261015 20261016 20261016b 20261016c 20261017 20261018 20261019 20261019a}"; do
  for g1 in $g; do
    d=$P/$g1; mkdir -p $d
    s31=$(ls data/saves/game_$g1 | grep -E 'AutoSave_0031\.Civ6Save$' | head -1)
    last=$(ls data/saves/game_$g1 | grep Civ6Save | tail -1)
    cp --update=none data/saves/game_$g1/$s31 data/saves/game_$g1/$last $d/
    seq 0 23 | xargs -P 4 -I{} sh -c "$B export $d {} $R > $P/exports/${g1}_p{}.jsonl 2>/dev/null"
  done
done
python3 tools/turn_stats.py $P/exports data/search/t30_stats/stats.tsv --majors
