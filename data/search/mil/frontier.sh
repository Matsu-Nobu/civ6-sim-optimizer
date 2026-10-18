#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/14, deadline T43: Castles, Mercenaries, a Great General and K horsemen required (5000 each);
# maximize food + production to T43 plus 20 extension turns (the economy after the spike counts). K = 6, 9, 12, 15.
cd "$R/optimizer"
D=$R/data/search/mil
until [ -f $D/max.done ]; do sleep 20; done
one() {
  K=$1
  C="--turn 43 --ext 20 --allow-military --team --require TECH_CASTLES:1,CIVIC_MERCENARIES:1,GREAT_PERSON_CLASS_GENERAL:1,UNIT_HORSEMAN:$K --require-weight 5000"
  nice "$BIN" rollout ../data/games/france_20261014.json $C --threads 4 --minutes 10 --quiet --init $D/feasb43.json --out $D/k$K.json > $D/k${K}_ro.log 2>&1
  nice "$BIN" irollout ../data/games/france_20261014.json $C --threads 4 --minutes 30 --window 10 --init $D/k$K.json --out $D/k$K.json > $D/k${K}_ir.log 2>&1
  echo done > $D/k$K.done
}
one 6 & one 9 & one 12 & one 15 &
wait
