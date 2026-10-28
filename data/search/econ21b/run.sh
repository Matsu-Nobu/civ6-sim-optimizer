#!/bin/bash
# 10/29: data/search/econ21 again with what harvests give counted in the turn's output (food and production of a
# chop count as the plots' yields do). From the best game so far (econ21/nested_best1.json): guided online games
# until one gains less than 1 point, then the nested rollout to no improvement. Run from the repository root.
B=optimizer/target/release/civ6opt
export G=data/games/rome_20261021.json
export C="--turn 65 --ext 0 --team --ally-supply --sum-floor 2680,1852,10"
export D=data/search/econ21b
M=data/search/learn/aa_m3.json
score() { $B run $G $C --genome "$1" 2>/dev/null | tail -1 | python3 -c "import json,sys; print(json.load(sys.stdin)['score'])"; }
prev=data/search/econ21/nested_best1.json; ps=$(score $prev)
echo "start $(date +%H:%M) $ps" >> $D/run.log
for i in $(seq 1 12); do
  [ -f $D/online$i.json ] || $B online $G $C --fallback-model $M --guide $prev --top 6 --threads 16 --out $D/online$i.json > /dev/null 2> $D/online$i.err
  s=$(score $D/online$i.json); echo "online$i $(date +%H:%M) $s" >> $D/run.log
  python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" $ps $s || break
  prev=$D/online$i.json; ps=$s
done
data/search/learn/nested.sh $M $prev
echo "nested done $(date +%H:%M)" >> $D/run.log
