#!/bin/bash
# 11/01 (after the natural wonder, border culture and border pick fixes): a plan to play in the game (game_mods/Civ6EconAutoplay) from the single-player start EconPlanStart_mp1_T002 (set up as the user's multiplayer games),
# to find the simulator's holes by comparing its turns with the game's saves. Made by an online search from the empty
# plan (no guide), with the option model of econ21f; no allies (a single-player game).
B=optimizer/target/release/civ6opt
G=data/games/econrun_mp1.json
D=data/search/econrun_mp1c
O="--turn 65 --ext 20 --ext-model --fallback-model data/search/econ21f/m1.json"
[ -f $D/online1.json ] || $B online $G $O --top 4 --threads 14 --out $D/online1.json > /dev/null 2> $D/online1.err
$B run $G $O --genome $D/online1.json 2>/dev/null | tail -1 > $D/online1.score
