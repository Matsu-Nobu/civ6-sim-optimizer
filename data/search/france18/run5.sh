#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/18, T35: the conditions of run3.sh without city deadlines, and two slingers trained by T25 (as in the recorded
# game) required. Two 20-minute iterated rollouts: from the best earlier plan free_rec.json and from the recorded play.
cd "$R/optimizer"
D=$R/data/search/france18
G=../data/games/france_20261018.json
C="--turn 35 --ext 0 --allow-military --team --require-weight 5000 --reward UNIT_ARCHER:150,UNIT_HORSEMAN:450,gold:0.5 --require DISTRICT_GOVERNMENT:1,DISTRICT_ENCAMPMENT:1,BUILDING_MONUMENT:1,UNIT_SLINGER:2@25"
nice "$BIN" irollout $G $C --threads 6 --minutes 20 --window 50 --passes 20 --kick-to 30 --seed 7 --init $D/free_rec.json --out $D/sl_a.json > $D/sl_a.log 2>&1 &
( nice "$BIN" rollout $G $C --threads 6 --minutes 2 --quiet --init record --out $D/sl_ro.json > $D/sl_ro.log 2>&1
  nice "$BIN" irollout $G $C --threads 6 --minutes 18 --window 50 --passes 20 --kick-to 30 --seed 8 --init $D/sl_ro.json --out $D/sl_rec.json > $D/sl_rec.log 2>&1 ) &
wait
