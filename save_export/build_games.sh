#!/bin/bash
# Usage: save_export/build_games.sh [out dir, default data/games]
# Makes the game data files from the compressed records and maps in data/exports and, for each game, the rules
# database data/rules/<game>.sqlite: the game's Cache/DebugGameplay.sqlite, written by a game set up like the recorded
# one (the mods in data/exports/game_<date>_mods.txt, the same victory types). Also needs data/rules/text_en.sqlite
# (civ6opt text) and the built civ6opt (cd optimizer && cargo build --release).
set -e
cd "$(dirname "$0")/.."
out=${1:-data/games}
mkdir -p "$out"
for f in data/exports/*.zst; do [ -f "${f%.zst}" ] || zstd -q -d "$f"; done
# record date, game data name, player, last turn of the plans, builder candidates per city
BIN=optimizer/target/x86_64-unknown-linux-gnu/release/civ6opt
one() {
  "$BIN" build "data/exports/game_$1" "data/rules/$2.sqlite" "$out/$2.json" \
    --player "$3" --horizon "$4" --builder-candidates "$5" --text data/rules/text_en.sqlite "${@:6}"
}
one 20261005 rome_20261005 0 60 8
one 20261007b rome_20261007b 0 85 8
one 20261013 france_20261013 8 60 8
one 20261014 france_20261014 5 68 8
# 10/17: the slinger (T15) and the trader (T16) were bought with gold an ally gave the same turn, so the record's
# gold does not show the purchases; they are given as gifts.
one 20261017 persia_20261017 2 45 4 --unit-gift 14:UNIT_SLINGER,15:UNIT_TRADER
one 20261018 france_20261018 1 45 8
