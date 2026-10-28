#!/bin/bash
# 10/29 night: the 10/21 Rome game searched for the inner economy with the movement rules fixed (river crossing
# costs max(3, points left); a step that costs more than the points left only with full points) and the game data
# remade with the roads and settlers of every save. Score: food + production summed to T65; science and culture
# summed to T65 must reach the recorded play's (2680, 1852), 10 per point short; no military units; allies give
# missing gold and strategic resources. Steps (each skipped when its result is there, so the script can be run again):
# the recorded play written out as a full plan (guide0), a guided online game from it, then the nested rollout to no
# improvement. Run from the repository root.
set -e
B=optimizer/target/release/civ6opt
export G=data/games/rome_20261021.json
export C="--turn 65 --ext 0 --team --ally-supply --sum-floor 2680,1852,10"
export D=data/search/econ21
M=data/search/learn/aa_m3.json
score() { $B run $G $C --genome "$1" 2>/dev/null | tail -1 | python3 -c "import json,sys; print(json.load(sys.stdin)['score'])"; }
[ -f $D/guide0.json ] || $B complete $G $C --init record --out $D/guide0.json 2> $D/guide0.err
echo "guide0 $(date +%H:%M) $(score $D/guide0.json)" >> $D/run.log
[ -f $D/online1.json ] || $B online $G $C --fallback-model $M --guide $D/guide0.json --top 6 --threads 16 --out $D/online1.json > $D/online1.out 2> $D/online1.err
echo "online1 $(date +%H:%M) $(score $D/online1.json)" >> $D/run.log
data/search/learn/nested.sh $M $D/online1.json
echo "nested done $(date +%H:%M)" >> $D/run.log
