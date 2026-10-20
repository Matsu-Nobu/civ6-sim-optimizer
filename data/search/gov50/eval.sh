#!/bin/bash
# eval.sh <tall|wide>: the plan's yields at the end of T50 and what it built
cd "$(dirname "$0")/../../../optimizer"
b=$([ "$1" = tall ] && echo BUILDING_GOV_TALL || echo BUILDING_GOV_WIDE)
target/x86_64-unknown-linux-gnu/release/civ6opt run ../data/games/gran_colombia_20261019.json --turn 50 --ext 0 --allow-military --team --require $b:1 --require-weight 5000 --genome ../data/search/gov50/$1.json > ../data/search/gov50/$1.eval 2>&1
