#!/bin/bash
# The accuracy of the simulator on the recorded games: the turn-by-turn check (civ6opt validate: turns without model
# errors) and the full replay against the record (replay_check.sh) at T10, T20, ..., T50 (food, production,
# science, culture: replay / record).
cd "$(dirname "$0")/.."
B=target/x86_64-unknown-linux-gnu/release/civ6opt
for g in ${@:-france_20261014 france_20261018 persia_20261017 gran_colombia_20261019 rome_20261007b}; do
  v=$($B validate ../data/games/$g.json --team 2>&1 | tail -1 | grep -o 'errors [0-9/]*')
  r=$(scripts/replay_check.sh 50 $g 2>/dev/null | grep -E '^T(30|40|50) ' | awk '{printf "%s %s %s | ", $1, $2, $3}')
  echo "$g: $v | $r"
done
