#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# Can Castles, Mercenaries and a Great General all be had by turn T? (no reward: requirements first, F+P second)
cd "$R/optimizer"
D=$R/data/search/mil
one() {
  T=$1
  C="--turn $T --ext 0 --allow-military --team --require TECH_CASTLES:1,CIVIC_MERCENARIES:1,GREAT_PERSON_CLASS_GENERAL:1 --require-weight 5000"
  nice "$BIN" rollout ../data/games/france_20261014.json $C --threads 5 --minutes 10 --quiet --init $D/feas$T.json --out $D/feasb$T.json > $D/feasb${T}_ro.log 2>&1
  nice "$BIN" irollout ../data/games/france_20261014.json $C --threads 5 --minutes 25 --window 12 --kick-to 40 --init $D/feasb$T.json --out $D/feasb$T.json > $D/feasb${T}_ir.log 2>&1
  echo done > $D/feasb$T.done
}
one 43 & one 44 & one 45 &
wait
