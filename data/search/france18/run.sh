#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/18, T35: each land combat unit owned at T35 earns W points per strength point (the higher of melee and ranged),
# on top of the food + production total. W = 3, 10, 30 trade military strength against the economy.
# From the recorded play: a 2-minute rollout, then an 8-minute iterated rollout with the repair over the whole rest of the plan.
cd "$R/optimizer"
D=$R/data/search/france18
G=../data/games/france_20261018.json
one() {
  w=$1
  r="UNIT_HORSEMAN:$((36*w)),UNIT_HEAVY_CHARIOT:$((36*w)),UNIT_SPEARMAN:$((29*w)),UNIT_ARCHER:$((25*w)),UNIT_WARRIOR:$((24*w)),UNIT_SLINGER:$((15*w))"
  C="--turn 35 --ext 0 --allow-military --team --reward $r"
  nice "$BIN" rollout $G $C --threads 4 --minutes 2 --quiet --init record --out $D/w${w}_ro.json > $D/w${w}_ro.log 2>&1
  nice "$BIN" irollout $G $C --threads 4 --minutes 8 --window 50 --passes 20 --kick-to 30 --seed $w --init $D/w${w}_ro.json --out $D/w$w.json > $D/w$w.log 2>&1
}
one 3 &
one 10 &
one 30 &
wait
