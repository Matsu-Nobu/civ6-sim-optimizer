#!/bin/bash
# Method comparison (notes/plan.md, phase 3): each search method from each start with each seed, the same budget
# (MINUTES per run, THREADS threads). Runs one after another; a run whose result file exists is skipped, so the script
# can be stopped and started again. Results: data/search/cmp/<method>_<start>_s<seed>.{json,log}
set -u
cd "$(dirname "$0")/.."
BIN=${BIN:-target/x86_64-unknown-linux-gnu/release/civ6opt}
P=${P:-../data/games/rome_20261007b.json}
MINUTES=${MINUTES:-30}
THREADS=${THREADS:-10}
OUT=../../data/search/cmp
mkdir -p $OUT
for seed in 1 2 3; do
  for start in record_wide base; do
    for method in lns ils ma nrpa; do
      f=$OUT/${method}_${start}_s${seed}
      [ -f $f.json ] && continue
      common="--turn 60 --ext 10 --seed $seed --threads $THREADS"
      case $method in
        lns)  args="lns $P $common --init $start --window 5 --budget 70 --passes 4" ;;
        ils)  args="ils $P $common --init $start --budget 70 --minutes $MINUTES" ;;
        ma)   args="ma $P $common --init $start --budget 70 --minutes $MINUTES" ;;
        nrpa) args="search $P $common --init $start --runs $THREADS --budget 7200 --rounds 3" ;;
      esac
      echo "$(date +%H:%M) $method $start seed $seed"
      # a run that dies (rare heap aborts, cause not found) is run once more
      for attempt in 1 2; do
        nice -n 19 $BIN $args --out $f.json.tmp > $f.log 2>&1 && { mv $f.json.tmp $f.json; break; }
        echo "$(date +%H:%M) $method $start seed $seed died (attempt $attempt)"
      done
    done
  done
done
