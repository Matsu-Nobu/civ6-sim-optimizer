#!/bin/bash
# Usage: save_export/build_games.sh [out dir, default data/games]
# Makes the game data files from the compressed records and maps in data/exports and, for each game, the rules
# database data/rules/<game>.sqlite: the game's Cache/DebugGameplay.sqlite, written by a game set up like the recorded
# one (the mods in data/exports/game_<date>_mods.txt, the same victory types). Also needs data/rules/text_en.sqlite
# (python_legacy/scripts/build_text_db.py).
set -e
cd "$(dirname "$0")/.."
out=${1:-data/games}
mkdir -p "$out"
for f in data/exports/*.zst; do [ -f "${f%.zst}" ] || zstd -q -d "$f"; done
# record date, game data name, player, last turn of the plans, builder candidates per city
one() {
  python3 python_legacy/scripts/export_problem.py "data/exports/game_$1" "data/rules/$2.sqlite" "$out/$2.json" \
    --player "$3" --horizon "$4" --builder-candidates "$5"
}
one 20261005 rome_20261005 0 60 8
one 20261007b rome_20261007b 0 85 8
one 20261013 france_20261013 8 60 8
one 20261014 france_20261014 5 68 8
one 20261017 persia_20261017 2 45 4
one 20261018 france_20261018 1 45 8
# 10/17: the slinger (T15) and the trader (T16) were bought with gold given by an ally the same turn, so the record's
# gold does not show the purchases; they are given as gifts.
python3 - "$out/persia_20261017.json" <<'PY'
import json, sys
p = sys.argv[1]
d = json.load(open(p))
d["scenario"]["unit_gifts"].update({"14": ["UNIT_SLINGER"], "15": ["UNIT_TRADER"]})
json.dump(d, open(p, "w"))
PY
