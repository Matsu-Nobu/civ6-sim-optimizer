#!/bin/bash
# Usage: save_export/setup_parser.sh
# Fetches the save parser (a git submodule), applies this project's changes to it and builds it with npm.
set -e
cd "$(dirname "$0")/.."
git submodule update --init third_party/civ6-savegame-parser
cd third_party/civ6-savegame-parser
if git apply --check ../civ6-savegame-parser.patch 2>/dev/null; then
  git apply ../civ6-savegame-parser.patch
fi
npm ci
npm run build
