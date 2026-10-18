#!/bin/bash
# Usage: save_export/mod_versions.sh <saveDir> > mods.txt
# Lists the mods (with their versions in the titles) the game was played with, from the first of its autosaves that lists them: the game's
# rules database must be made with the same mods to recompute the game data.
set -e
for save in $(ls "$1"/*.Civ6Save | sort | head -5); do
  out=$(strings -n 6 "$save" | grep -o -E '^\{"[^"]+ [0-9][0-9.]*":\[\]\}' | sed -E 's/^\{"(.*)":\[\]\}$/\1/'; strings -n 6 "$save" | grep -o -E '"en_US", "text":"[^"]+ [0-9][0-9.]*"' | sed -E 's/.*"text":"(.*)"/\1/')
  if [ -n "$out" ]; then printf '%s\n' "$out" | sort -u; exit 0; fi
done
