#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/18, T35. Required (5000 each): a government plaza, an encampment and a monument by T35. Each archer earns 150,
# each horseman 450 and each gold left 0.5, on top of the food + production total.
# "fixed": also the second city by T12 and the third by T20 (as founded in the recorded game); "free": no such deadline.
# For each, two iterated rollouts of 18 minutes: one from the recorded play (after a 2-minute rollout), one from a150.json.
cd "$R/optimizer"
D=$R/data/search/france18
G=../data/games/france_20261018.json
BASE="--turn 35 --ext 0 --allow-military --team --require-weight 5000 --reward UNIT_ARCHER:150,UNIT_HORSEMAN:450,gold:0.5"
REQ="DISTRICT_GOVERNMENT:1,DISTRICT_ENCAMPMENT:1,BUILDING_MONUMENT:1"
run() {
  name=$1; req=$2; seed=$3
  C="$BASE --require $req"
  ( nice "$BIN" rollout $G $C --threads 3 --minutes 2 --quiet --init record --out $D/${name}_ro.json > $D/${name}_ro.log 2>&1
    nice "$BIN" irollout $G $C --threads 3 --minutes 18 --window 50 --passes 20 --kick-to 30 --seed $seed --init $D/${name}_ro.json --out $D/${name}_rec.json > $D/${name}_rec.log 2>&1 ) &
  nice "$BIN" irollout $G $C --threads 3 --minutes 20 --window 50 --passes 20 --kick-to 30 --seed $((seed+1)) --init $D/a150.json --out $D/${name}_a.json > $D/${name}_a.log 2>&1 &
}
run fixed "$REQ,cities:2@12,cities:3@20" 1
run free "$REQ" 3
wait
