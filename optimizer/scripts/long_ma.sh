#!/bin/bash
# A long memetic search that survives crashes: runs civ6opt ma until it stops on its own (stalled, or MINUTES used up);
# a run that dies is started again from the best plan saved so far (the file is rewritten every generation).
# Usage: optimizer/scripts/long_ma.sh <out.json> [init]   env: MINUTES (total, 360), STALL (60), THREADS (10), SEED (1), P
set -u
cd "$(dirname "$0")/.."
BIN=${BIN:-target/x86_64-unknown-linux-gnu/release/civ6opt}
P=${P:-../data/games/rome_20261007b.json}
OUT=$1; INIT=${2:-record_wide}
MINUTES=${MINUTES:-360}; STALL=${STALL:-60}; THREADS=${THREADS:-10}; SEED=${SEED:-1}
start=$(date +%s); try=0
while true; do
  used=$(( ($(date +%s) - start) / 60 )); left=$(( MINUTES - used ))
  [ $left -le 1 ] && { echo "time used up"; break; }
  init=$INIT; [ -f "$OUT" ] && init=$OUT
  try=$((try + 1))
  echo "$(date +%H:%M) attempt $try from $init, $left minutes left"
  nice -n 19 $BIN ma $P --turn 60 --ext 10 --seed $((SEED + try - 1)) --threads $THREADS --pop ${POP:-10} --init $init --budget 70 \
    --minutes $left --stall $STALL --out "$OUT" > "${OUT%.json}_$try.log" 2>&1
  rc=$?
  tail -1 "${OUT%.json}_$try.log"
  [ $rc -eq 0 ] && break
  echo "attempt $try died (exit $rc); restarting from the saved best"
done
