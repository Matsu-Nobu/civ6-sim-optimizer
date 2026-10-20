#!/bin/bash
# Government plaza buildings over time: every major civilization of the three games where all built one, at the end
# of every tenth turn (the saves that start T11, T21, ...), 2 exports at a time; summarized in stats.tsv.
# Save copies and raw exports: data/saves/picks/gov_compare.
cd "$(dirname "$0")/../../.."
B=optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt
R=data/rules/gameplay_2026-10-19.sqlite
P=data/saves/picks/gov_compare
mkdir -p $P/exports
for x in "20261014:0 1 2 3 4 5 7 8 9 10" "20261017:0 1 2 3 4 5 6 7 8 9 10 12" "20261019:0 8 9 10 11 12 13 14"; do
  g=${x%%:*}; pids=${x#*:}
  mkdir -p $P/$g
  for n in 0011 0021 0031 0041 0051 0061 0071 0081 0091 0100; do
    [ -f data/saves/game_$g/AutoSave_$n.Civ6Save ] && cp --update=none data/saves/game_$g/AutoSave_$n.Civ6Save $P/$g/
  done
  echo $pids | tr ' ' '\n' | xargs -P 2 -I{} sh -c "$B export $P/$g {} $R > $P/exports/${g}_p{}.jsonl 2>/dev/null"
done
python3 tools/turn_stats.py $P/exports data/search/gov_compare/stats.tsv --majors
