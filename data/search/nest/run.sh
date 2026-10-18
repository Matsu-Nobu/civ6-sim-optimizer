#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/14, deadline T44: Castles, Mercenaries, a Great General required (5000 each), horsemen +200 each.
# Iterated rollout with the repair over the whole rest of the plan (--window 50) and kicks in T2-30 only:
# an early change is compared only after everything after it has been re-optimized (--passes 20: refit until no change, as in the 2026-10-17 trial: 6/36 kicks
# beat the converged plan, best 11 -> 16 horsemen). Two starts: the converged max plan, and the recorded play.
cd "$R/optimizer"
B="$BIN"
D=$R/data/search/nest
C="--turn 44 --ext 0 --allow-military --team --require TECH_CASTLES:1,CIVIC_MERCENARIES:1,GREAT_PERSON_CLASS_GENERAL:1 --require-weight 5000 --reward UNIT_HORSEMAN:200"
( nice $B irollout ../data/games/france_20261014.json $C --threads 8 --minutes 60 --window 50 --passes 20 --kick-to 30 --seed 1 --init $D/max44.json --out $D/from_max.json > $D/from_max.log 2>&1; echo done > $D/from_max.done ) &
( nice $B rollout ../data/games/france_20261014.json $C --threads 8 --minutes 5 --quiet --init record_wide --out $D/rec44.json > $D/rec44_ro.log 2>&1
  nice $B irollout ../data/games/france_20261014.json $C --threads 8 --minutes 55 --window 50 --passes 20 --kick-to 30 --seed 2 --init $D/rec44.json --out $D/from_rec.json > $D/from_rec.log 2>&1; echo done > $D/from_rec.done ) &
wait
