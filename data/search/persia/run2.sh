#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# Same four variants: iterated rollout with whole-rest repair (research, civics, sites frozen), 12 minutes each.
cd "$R/optimizer"
D=$R/data/search/persia
C="--turn 30 --ext 0 --team --allow-military --require civics:6,CIVIC_POLITICAL_PHILOSOPHY:1,UNIT_SETTLER:4,DISTRICT_COMMERCIAL_HUB:1 --require-weight 5000"
F=$(cat $D/freeze.json)
s=1
for v in R A B C; do
  ( nice "$BIN" irollout ../data/games/persia_20261017.json $C --freeze "$F" --threads 3 --minutes 12 --window 40 --passes 20 --kick-to 30 --seed $s --init $D/$v.json --out $D/${v}2.json > $D/${v}2.log 2>&1 ) &
  s=$((s+1))
done
wait
echo done > $D/all2.done
