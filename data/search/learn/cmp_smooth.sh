#!/bin/bash
# 10/23: the rollout search as it is (cliff) against the smooth requirement with the model filling untakeable plan
# choices, then 5 minutes with the cliff to finish; both from the best plan, 8 threads each, 20 minutes in all.
# Run from the repository root.
B=optimizer/target/release/civ6opt
G=data/games/rome_20261021.json
C="--turn 65 --ext 0 --allow-military --team --require TECH_BALLISTICS:1@65 --require-weight 20000 --reward UNIT_KNIGHT:100@65"
R="--threads 8 --window 65 --passes 20 --kick-to 60 --seed 7"
D=data/search/learn
$B irollout $G $C $R --minutes 20 --init $D/rows2.json --out $D/cmp_cliff.json > $D/cmp_cliff.log 2>&1 &
( $B irollout $G $C $R --require-smooth --fallback-model $D/m3_obj.json --minutes 15 --init $D/rows2.json --out $D/cmp_smooth.json > $D/cmp_smooth.log 2>&1
  $B irollout $G $C $R --fallback-model $D/m3_obj.json --minutes 5 --init $D/cmp_smooth.json --out $D/cmp_smooth_final.json > $D/cmp_smooth_final.log 2>&1 ) &
wait
