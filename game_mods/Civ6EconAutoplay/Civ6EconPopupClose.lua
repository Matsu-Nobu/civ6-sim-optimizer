-- civ6-econ: closes the popup of this context soon after it shows while the plan runner plays (MODE "plan").
-- Era, natural wonder and natural disaster popups hold the game's event queue until the player closes them
-- (PopupManager.lua: UI.ReferenceCurrentEvent), which stops the run; only the popup's own Close releases the hold.
-- Each replacement script includes the game's script for its context, then this file.
include("Civ6EconRunnerConfig")

if ECONRUN.MODE == "plan" then
  local shown = 0
  ContextPtr:SetUpdate(function()
    -- a hidden context gets no update calls: the count restarts each time the popup shows
    shown = shown + 1
    if shown >= 30 then
      shown = 0
      print("ECONRUN closing popup " .. tostring(ContextPtr:GetID()))
      Close()
    end
  end)
end
