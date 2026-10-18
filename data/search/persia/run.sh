#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# Persia 10/17 (player 2), T30: research orders R (recorded) / A (Mining 2nd) / B (Animal Husbandry 2nd) /
# C (both 2nd-3rd) fixed with civics and city sites; everything else re-optimized by rollout, same budget each.
# Requirements as in the recorded play by T30: 6 civics incl. Political Philosophy, 4 settlers, a commercial hub.
cd "$R/optimizer"
D=$R/data/search/persia
C="--turn 30 --ext 0 --team --allow-military --require civics:6,CIVIC_POLITICAL_PHILOSOPHY:1,UNIT_SETTLER:4,DISTRICT_COMMERCIAL_HUB:1 --require-weight 5000"
F=$(cat $D/freeze.json)
for v in R A B C; do
  ( nice "$BIN" rollout ../data/games/persia_20261017.json $C --freeze "$F" --threads 4 --minutes 6 --quiet --init $D/${v}0.json --out $D/$v.json > $D/$v.log 2>&1 ) &
done
wait
echo done > $D/all.done
