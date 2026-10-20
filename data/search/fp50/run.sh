#!/bin/bash
# Can a plan reach 50 food and 50 production per turn at the end of T30? No requirements; the score adds
# 60 x min(food, production) + 5 x food + 5 x production of T30 to the usual food + production summed over the turns.
# Persia 10/17 from the best earlier plan (R2), Gran Colombia 10/19 from the recorded play: rollout 3 min, then
# iterated rollout 12 min, 4 threads each.
R=$(cd "$(dirname "$0")/../../.." && pwd)
B="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
cd "$R/optimizer"
D=$R/data/search/fp50
C="--turn 30 --ext 0 --allow-military --team --reward fp_min:60,food:5,production:5"
one() {
  name=$1; game=$2; init=$3; seed=$4
  nice $B rollout ../data/games/$game.json $C --threads 4 --minutes 3 --quiet --init $init --out $D/${name}_ro.json > $D/${name}_ro.log 2>&1
  nice $B irollout ../data/games/$game.json $C --threads 4 --minutes 12 --window 40 --passes 20 --kick-to 28 --seed $seed --init $D/${name}_ro.json --out $D/$name.json > $D/$name.log 2>&1
  echo done > $D/$name.done
}
one persia persia_20261017 $R/data/search/persia/R2.json 1 &
one gc gran_colombia_20261019 record 2 &
wait
