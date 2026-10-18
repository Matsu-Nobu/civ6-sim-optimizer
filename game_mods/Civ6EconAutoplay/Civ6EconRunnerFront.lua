-- civ6-econ runner, front end: load the configured save as soon as the main menu is up.
include("Civ6EconRunnerConfig")
print("ECONRUN front end loaded, mode " .. tostring(ECONRUN and ECONRUN.MODE))
local requested = false

local function load_save()
  if requested or ECONRUN == nil or ECONRUN.MODE ~= "experiment" then return end
  requested = true
  local ok, err = pcall(function()
    Network.LeaveGame()
    local g = {}
    g.Location = SaveLocations.LOCAL_STORAGE
    g.Type = SaveTypes.SINGLE_PLAYER
    g.IsAutosave = false
    g.IsQuicksave = false
    g.Directory = SaveDirectories.DEFAULT
    g.Name = ECONRUN.SAVE_NAME
    GameConfiguration.SetToDefaults()
    local r = Network.LoadGame(g, ServerType.SERVER_TYPE_NONE)
    print("ECONRUN load " .. ECONRUN.SAVE_NAME .. " -> " .. tostring(r))
  end)
  if not ok then print("ECONRUN load failed: " .. tostring(err)) end
end

-- the main menu raises its screen on startup; try on the first front-end ticks once systems are up
local ticks = 0
ContextPtr:SetUpdate(function()
  ticks = ticks + 1
  if ticks == 120 then load_save() end
end)
