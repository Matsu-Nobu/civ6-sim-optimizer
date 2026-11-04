#!/bin/bash
# Rows for the value models with each state's cities (tools/value_net.py): valuerows around each plan of plans.txt
# on the 10/21 Rome game, one Windows job at a time (jobs value5_<n>). The city rows stay in the job's folder on
# Windows (cities.tsv), where the models learn; the state rows are fetched here too. Run from the repository root.
i=0
for f in $(cat data/search/value5/plans.txt); do
  i=$((i+1)); job=value5_$i
  [ -f data/search/value5/rows_$i.tsv ] && continue
  optimizer/scripts/win_run.sh start $job valuerows data/games/rome_20261021.json --turn 65 --plays 1000 --threads 8 --ally-supply --init $f --out data/search/value5/rows_$i.tsv --city-out cities.tsv || exit 1
  until grep -q done /mnt/c/Users/${WINUSER:-nobus}/civ6opt/jobs/$job/status.txt 2>/dev/null; do sleep 20; done
  optimizer/scripts/win_run.sh fetch $job
done
echo ALLDONE
