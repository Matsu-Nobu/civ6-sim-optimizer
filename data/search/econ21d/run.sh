#!/bin/bash
# 10/29 evening: the inner-economy search with a 20-turn extension played by the learned model (--ext-model) and
# harvests counted as far as the item used them (overflow lost). Floors: the recorded play's science and culture sums
# to T65 under these rules (2674, 1842). Guided online games from the best game so far until one gains less than 1.
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
C="--turn 65 --ext 20 --ext-model --fallback-model data/search/learn/aa_m3.json --team --ally-supply --sum-floor 2674,1842,10"
D=data/search/econ21d
score() { $B run $G $C --genome "$1" 2>/dev/null | tail -1 | python3 -c "import json,sys; print(json.load(sys.stdin)['score'])"; }
prev=data/search/econ21c/online1.json; ps=$(score $prev)
echo "start $(date +%H:%M) $ps record $(score record)" >> $D/run.log
for i in $(seq 1 12); do
  [ -f $D/online$i.json ] || $B online $G $C --guide $prev --top 6 --threads 16 --out $D/online$i.json > /dev/null 2> $D/online$i.err
  s=$(score $D/online$i.json); echo "online$i $(date +%H:%M) $s" >> $D/run.log
  python3 -c "import sys; sys.exit(0 if float(sys.argv[2]) > float(sys.argv[1]) + 1 else 1)" $ps $s || break
  prev=$D/online$i.json; ps=$s
done
echo "done $(date +%H:%M) best $prev $ps" >> $D/run.log
