#!/bin/bash
# Rows for the value net under the Rome search's settings (team, ally supply, the science and culture floors, a
# 20-turn extension): valuerows around each plan of plans.txt; from the first recorded turn on, the plays follow the
# learned policy of econ21f/m1.json, which also plays the extension. One Windows job at a time (value7_<n>); the city
# rows stay in the job's folder. Run from the repository root.
F="--turn 65 --ext 20 --team --ally-supply --sum-floor 2674,1842,10 --ext-model --fallback-model data/search/econ21f/m1.json"
i=0
for f in $(cat data/search/value7/plans.txt); do
  i=$((i+1)); job=value7_$i
  [ -f data/search/value7/rows_$i.tsv ] && continue
  optimizer/scripts/win_run.sh start $job valuerows data/games/rome_20261021.json $F --plays 1000 --threads 8 --base-after --init $f --out data/search/value7/rows_$i.tsv --city-out cities.tsv || exit 1
  until grep -q done /mnt/c/Users/${WINUSER:-nobus}/civ6opt/jobs/$job/status.txt 2>/dev/null; do sleep 20; done
  optimizer/scripts/win_run.sh fetch $job
done
echo ALLDONE
