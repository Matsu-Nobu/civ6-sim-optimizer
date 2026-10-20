#!/bin/bash
# Gran Colombia 10/19: every major civilization at the ends of T10, T20, T30, T45, T60, T80 and T99 (the saves that
# start the next turn), summarized in civs.tsv. Save copies and raw exports: data/saves/picks/gc19_civs.
cd "$(dirname "$0")/../../.."
B=optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt
R=data/rules/gameplay_2026-10-19.sqlite
P=data/saves/picks/gc19_civs
mkdir -p $P/saves $P/exports
for n in 0011 0021 0031 0046 0061 0081 0100; do cp --update=none data/saves/game_20261019/AutoSave_$n.Civ6Save $P/saves/; done
for p in 0 8 9 10 11 12 13 14; do $B export $P/saves $p $R > $P/exports/20261019_p$p.jsonl 2>/dev/null & done
wait
python3 tools/turn_stats.py $P/exports data/search/gc19/civs.tsv
