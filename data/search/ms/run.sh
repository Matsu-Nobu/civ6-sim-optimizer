#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# Milestone objective (T85 combustion, T90 totalitarianism; ext 30): rollout from the rollout bests A/B/C in parallel,
# each until it stops or 90 minutes; a run that dies restarts from its last saved plan.
cd "$R/optimizer"
D=$R/data/search/ms
C="--turn 60 --ext 30 --milestones TECH_COMBUSTION@85,CIVIC_TOTALITARIANISM@90"
one() {
  x=$1; start=$(date +%s); try=0
  while true; do
    left=$(( 90 - ($(date +%s) - start) / 60 )); [ $left -le 1 ] && break
    init=$R/data/search/rollout/ro_$x.json; [ -f $D/ms_$x.json ] && init=$D/ms_$x.json
    try=$((try + 1)); echo "$(date +%H:%M) $x attempt $try from $init ($left min left)"
    nice -n 19 "$BIN" rollout ../data/games/rome_20261007b.json $C --threads 4 --minutes $left \
      --init $init --out $D/ms_$x.json > $D/ms_${x}_$try.log 2>&1 && break
    echo "$(date +%H:%M) $x attempt $try died"
  done
  echo "$(date +%H:%M) $x done: $(tail -1 $D/ms_${x}_$try.log)"
}
for x in A B C; do one $x >> $D/progress.log 2>&1 & done
wait
