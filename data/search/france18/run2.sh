#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/18, T35: a government plaza, a barracks and a monument are required by T35 (5000 each).
# Each archer owned at T35 earns A points and each horseman 3A, on top of the food + production total.
# A = 50, 150, 500 trade the army against the economy.
# From the recorded play: a 2-minute rollout, then an 18-minute iterated rollout with the repair over the whole rest of the plan.
cd "$R/optimizer"
D=$R/data/search/france18
G=../data/games/france_20261018.json
one() {
  a=$1
  C="--turn 35 --ext 0 --allow-military --team --require DISTRICT_GOVERNMENT:1,BUILDING_BARRACKS:1,BUILDING_MONUMENT:1 --require-weight 5000 --reward UNIT_ARCHER:$a,UNIT_HORSEMAN:$((3*a))"
  nice "$BIN" rollout $G $C --threads 4 --minutes 2 --quiet --init record --out $D/a${a}_ro.json > $D/a${a}_ro.log 2>&1
  nice "$BIN" irollout $G $C --threads 4 --minutes 18 --window 50 --passes 20 --kick-to 30 --seed $a --init $D/a${a}_ro.json --out $D/a$a.json > $D/a$a.log 2>&1
}
one 50 &
one 150 &
one 500 &
wait
