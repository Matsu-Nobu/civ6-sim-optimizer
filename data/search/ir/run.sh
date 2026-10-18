#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# Method comparison at equal compute (4 threads, $MIN minutes each), from the milestone-objective rollout bests A and C
# (milestone weight 10: the T85/T90 goals as near-hard constraints): iterated rollout (kick + rollout repair of the
# next --window turns) vs the plain rollout. A run that dies restarts from its last saved plan.
cd "$R/optimizer"
D=$R/data/search/ir
MIN=${MIN:-150}
C="--turn 60 --ext 30 --milestones TECH_COMBUSTION@85,CIVIC_TOTALITARIANISM@90 --milestone-weight 10"
one() {
  m=$1; x=$2; start=$(date +%s); try=0
  while true; do
    left=$(( MIN - ($(date +%s) - start) / 60 )); [ $left -le 1 ] && break
    init=$R/data/search/ms/ms_$x.json; [ -f $D/${m}_$x.json ] && init=$D/${m}_$x.json
    try=$((try + 1)); echo "$(date +%H:%M) $m $x attempt $try from $init ($left min left)"
    if [ $m = ir ]; then M=irollout; X="--window 8 --seed $try"; else M=rollout; X=""; fi
    nice -n 19 "$BIN" $M ../data/games/rome_20261007b.json $C $X --threads 4 --minutes $left \
      --init $init --out $D/${m}_$x.json > $D/${m}_${x}_$try.log 2>&1 && break
    echo "$(date +%H:%M) $m $x attempt $try died"; [ $try -ge 5 ] && break
  done
  echo "$(date +%H:%M) $m $x done: $(tail -1 $D/${m}_${x}_$try.log)"
}
for m in ir ro; do for x in A C; do one $m $x >> $D/progress.log 2>&1 & done; done
wait
