#!/bin/bash
# 11/01: a plan to play in the game (game_mods/Civ6EconAutoplay) from the single-player start EconPlanStart_rome2_T002,
# to find the simulator's holes by comparing its turns with the game's saves. Made by an online search from the empty
# plan (no guide), with the option model of econ21f; no allies (a single-player game).
B=optimizer/target/release/civ6opt
G=data/games/econrun_rome2.json
D=data/search/econrun1
O="--turn 65 --ext 20 --ext-model --fallback-model data/search/econ21f/m1.json"
[ -f $D/online1.json ] || $B online $G $O --top 4 --threads 14 --out $D/online1.json > /dev/null 2> $D/online1.err
$B run $G $O --genome $D/online1.json 2>/dev/null | tail -1 > $D/online1.score
