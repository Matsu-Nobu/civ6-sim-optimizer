-- Gameplay script for the probe's boost experiment (MODE "probe" with ECONRUN.BOOSTS = true): right after a save
-- loads, every major player gets the boost of each technology and civic it has neither researched nor had boosted,
-- from no progress.
-- A civic's progress has no getter in a gameplay script, so the probe's UI script reads the civics afterwards.
include("Civ6EconRunnerConfig")

local function emit(s)
  print("ECONBOOST " .. s)
end

local function run()
  if ECONRUN == nil or ECONRUN.MODE ~= "probe" or not ECONRUN.BOOSTS or ECONRUN.PROBE == nil then return end
  local turn = Game.GetCurrentGameTurn()
  for _, pid in ipairs(PlayerManager.GetAliveMajorIDs()) do
    local pl = Players[pid]
    local t, c = pl:GetTechs(), pl:GetCulture()
    for row in GameInfo.Technologies() do
      if not t:HasTech(row.Index) and not t:HasBoostBeenTriggered(row.Index) then
        t:SetResearchProgress(row.Index, 0)
        t:TriggerBoost(row.Index, 0)
        emit(string.format('{"turn":%d,"player":%d,"item":"%s","cost":%s,"amount":%s}', turn, pid, row.TechnologyType,
          tostring(t:GetResearchCost(row.Index)), tostring(t:GetResearchProgress(row.Index))))
      end
    end
    for row in GameInfo.Civics() do
      if not c:HasCivic(row.Index) and not c:HasBoostBeenTriggered(row.Index) then
        c:SetCulturalProgress(row.Index, 0)
        c:TriggerBoost(row.Index, 0)
        emit(string.format('{"turn":%d,"player":%d,"item":"%s","boosted":true}', turn, pid, row.CivicType))
      end
    end
  end
  emit('{"done":true}')
end

run()
