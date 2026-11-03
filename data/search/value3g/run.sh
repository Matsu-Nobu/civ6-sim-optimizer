#!/bin/bash
# Rows for the value model (tools/value_compare.py): valuerows around each plan of plans.txt on the 10/21 Rome game,
# one Windows job at a time. Run from the repository root.
i=0
for f in $(cat data/search/value3g/plans.txt); do
  i=$((i+1)); job=value3g_$i
  [ -f data/search/value3g/rows_$i.tsv ] && continue
  optimizer/scripts/win_run.sh start $job valuerows data/games/rome_20261021.json --turn 65 --plays 1000 --changes 30 --threads 8 --ally-supply --init $f --out data/search/value3g/rows_$i.tsv || exit 1
  until grep -q done /mnt/c/Users/${WINUSER:-nobus}/civ6opt/jobs/$job/status.txt 2>/dev/null; do sleep 20; done
  optimizer/scripts/win_run.sh fetch $job
done
