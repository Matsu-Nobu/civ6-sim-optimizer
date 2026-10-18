#!/bin/bash
R=$(cd "$(dirname "$0")/../../.." && pwd)
BIN="$R/optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt"
# France 10/18, T35: the conditions of run3.sh with the cities founded as early as in the recorded game (second by T12,
# third by T20) and a fourth city by T27. Two 10-minute iterated rollouts from the recorded play with a third settler
# in place of the trader (record_settler3.json).
cd "$R/optimizer"
D=$R/data/search/france18
G=../data/games/france_20261018.json
C="--turn 35 --ext 0 --allow-military --team --require-weight 5000 --reward UNIT_ARCHER:150,UNIT_HORSEMAN:450,gold:0.5 --require DISTRICT_GOVERNMENT:1,DISTRICT_ENCAMPMENT:1,BUILDING_MONUMENT:1,cities:2@12,cities:3@20,cities:4@27"
nice "$BIN" irollout $G $C --threads 6 --minutes 10 --window 50 --passes 20 --kick-to 30 --seed 5 --init $D/record_settler3.json --out $D/fast4_1.json > $D/fast4_1.log 2>&1 &
nice "$BIN" irollout $G $C --threads 6 --minutes 10 --window 50 --passes 20 --kick-to 30 --seed 6 --init $D/record_settler3.json --out $D/fast4_2.json > $D/fast4_2.log 2>&1 &
wait
