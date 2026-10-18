#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# Rollout policy improvement from the three memetic bests (A, B, C), one after another, each until it stops (no single
# change improves) or 60 minutes; a run that dies starts again from its last saved plan.
cd "$R/optimizer"
D=$R/data/search/rollout
for x in A B C; do
  start=$(date +%s); try=0
  while true; do
    left=$(( 60 - ($(date +%s) - start) / 60 )); [ $left -le 1 ] && break
    init=$R/data/search/final/ma_$x.json; [ -f $D/ro_$x.json ] && init=$D/ro_$x.json
    try=$((try + 1)); echo "$(date +%H:%M) $x attempt $try from $init ($left min left)"
    nice -n 19 "$BIN" rollout ../data/games/rome_20261007b.json --turn 60 --ext 10 --threads 10 --minutes $left \
      --init $init --out $D/ro_$x.json > $D/ro_${x}_$try.log 2>&1 && break
    echo "$(date +%H:%M) $x attempt $try died"
  done
  echo "$(date +%H:%M) $x done: $(tail -1 $D/ro_${x}_$try.log)"
done
