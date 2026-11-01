-- Written by civ6-econ scripts. MODE: "off" (do nothing), "autoplay" (AI plays AUTOPLAY_TURNS),
-- "experiment" (load SAVE_NAME from the main menu, play TURNS turns, save each turn),
-- "plan" (load SAVE_NAME from the main menu, play Civ6EconPlan.lua as the local player, save each turn),
-- "newgame" (set up a new game with player 0 as LEADER, play TURNS turns, save each turn).
ECONRUN = { MODE = "plan", SAVE_NAME = "EconPlanStart_rome2_T002", TURNS = 100, AUTOPLAY_TURNS = 150, RUN_ID = "econrun1",
            LEADER = "LEADER_TRAJAN", SPEED = "GAMESPEED_ONLINE", MAP = "Pangaea.lua", MAP_SIZE = "MAPSIZE_STANDARD",
            CITY_STATES = 12 }
