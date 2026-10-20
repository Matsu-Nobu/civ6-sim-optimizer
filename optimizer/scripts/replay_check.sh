#!/bin/bash
# Full replay of the recorded play against the record (tools/replay_check.py), for each game named, to the turn given
# (default 50; the game data's horizon caps it).
#   optimizer/scripts/replay_check.sh <turn> <game data name>...   e.g. 50 gran_colombia_20261019 persia_20261017
set -eu
cd "$(dirname "$0")/.."
turn=$1; shift
for g in "$@"; do
  echo "=== $g"
  target/x86_64-unknown-linux-gnu/release/civ6opt run ../data/games/$g.json --turn $((turn + 1)) --ext 0 --allow-military --team --genome ${GENOME:-record} \
    > /tmp/replay_$g.out 2> /tmp/replay_$g.err
  grep -E "^exogenous" /tmp/replay_$g.err || true
  python3 ../tools/replay_check.py ../data/games/$g.json /tmp/replay_$g.out
done
