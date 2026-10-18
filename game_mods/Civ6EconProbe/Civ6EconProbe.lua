-- Civ6 Econ Probe: read-only dump of the game's own economy numbers to Lua.log, one JSON line per turn,
-- prefixed "ECONPROBE ". Only queries the game; never changes it.

local function esc(s)
  s = tostring(s):gsub('\\', '\\\\'):gsub('"', '\\"'):gsub('\n', '\\n'):gsub('\r', '\\r'):gsub('\t', '\\t')
  return (s:gsub('[%c]', ''))
end
local function enc(v)
  local t = type(v)
  if t == "nil" then return "null" end
  if t == "number" then
    if v ~= v or v == math.huge or v == -math.huge then return "null" end
    return tostring(v)
  end
  if t == "boolean" then return v and "true" or "false" end
  if t == "string" then return '"' .. esc(v) .. '"' end
  if t == "table" then
    if #v > 0 or next(v) == nil then
      local out = {}
      for i = 1, #v do out[#out + 1] = enc(v[i]) end
      return "[" .. table.concat(out, ",") .. "]"
    end
    local out = {}
    for k, x in pairs(v) do out[#out + 1] = '"' .. esc(k) .. '":' .. enc(x) end
    return "{" .. table.concat(out, ",") .. "}"
  end
  return "null"
end

local function try(f, ...)
  local ok, r = pcall(f, ...)
  if ok then return r end
  return nil
end

local function yields_of(get)
  local y = {}
  for row in GameInfo.Yields() do
    y[row.YieldType] = try(get, row.Index)
  end
  return y
end

local function dump_city(pCity)
  local c = {}
  c.name = pCity:GetName()
  c.x, c.y = pCity:GetX(), pCity:GetY()
  c.population = pCity:GetPopulation()
  c.yields = yields_of(function(i) return pCity:GetYield(i) end)
  local g = pCity:GetGrowth()
  if g then
    c.food = try(function() return g:GetFood() end)
    c.foodSurplus = try(function() return g:GetFoodSurplus() end)
    c.growthThreshold = try(function() return g:GetGrowthThreshold() end)
    c.turnsUntilGrowth = try(function() return g:GetTurnsUntilGrowth() end)
    c.housing = try(function() return g:GetHousing() end)
    c.amenities = try(function() return g:GetAmenities() end)
    c.amenitiesNeeded = try(function() return g:GetAmenitiesNeeded() end)
  end
  -- Breakdowns: where the city's numbers come from, as the game itself reports them.
  if g then
    c.housingParts = {}
    for _, k in ipairs({"Water", "Buildings", "Improvements", "Districts", "Civics", "GreatPeople", "GreatWorks", "StartingEra"}) do
      c.housingParts[k] = try(function() return g["GetHousingFrom" .. k](g) end)
    end
    c.amenityParts = {}
    for _, k in ipairs({"Luxuries", "Entertainment", "Districts", "Civics", "GreatPeople", "CityStates", "Religion",
                        "NationalParks", "NaturalWonders", "Improvements", "StartingEra", "Traits", "Governors"}) do
      c.amenityParts[k] = try(function() return g["GetAmenitiesFrom" .. k](g) end)
    end
    c.amenityParts.LostWarWeariness = try(function() return g:GetAmenitiesLostFromWarWeariness() end)
    c.amenityParts.LostBankruptcy = try(function() return g:GetAmenitiesLostFromBankruptcy() end)
    c.growthMods = {overall = try(function() return g:GetOverallGrowthModifier() end),
      happiness = try(function() return g:GetHappinessGrowthModifier() end),
      housing = try(function() return g:GetHousingGrowthModifier() end),
      other = try(function() return g:GetOtherGrowthModifier() end),
      nonFoodYield = try(function() return g:GetHappinessNonFoodYieldModifier() end),
      happinessLevel = try(function() return g:GetHappiness() end)}
  end
  c.yieldTips = {}
  for row in GameInfo.Yields() do
    c.yieldTips[row.YieldType] = try(function() return pCity:GetYieldToolTip(row.Index) end)
  end
  -- yields of each building and district
  local bl = pCity:GetBuildings()
  c.buildingYields = {}
  for row in GameInfo.Buildings() do
    if bl and try(function() return bl:HasBuilding(row.Index) end) then
      c.buildingYields[row.BuildingType] = yields_of(function(i) return pCity:GetBuildingYield(row.Index, i) end)
      if try(function() return bl:IsPillaged(row.Index) end) then
        c.pillagedBuildings = c.pillagedBuildings or {}
        c.pillagedBuildings[#c.pillagedBuildings + 1] = row.BuildingType
      end
    end
  end
  c.districtYields = {}
  local ds = pCity:GetDistricts()
  if ds then
    for _, d in ds:Members() do
      local info = GameInfo.Districts[d:GetType()]
      c.districtYields[#c.districtYields + 1] = {info and info.DistrictType, d:GetX(), d:GetY(),
        yields_of(function(i) return d:GetYield(i) end), yields_of(function(i) return d:GetAdjacencyYield(i) end)}
    end
  end
  -- trade routes leaving this city, with what each side gets (route / path / modifiers, as the trade screen adds them)
  local tm = Game.GetTradeManager()
  local routes = try(function() return pCity:GetTrade():GetOutgoingRoutes() end) or {}
  c.routes = {}
  for _, r in ipairs(routes) do
    local args = {r.OriginCityPlayer, r.OriginCityID, r.DestinationCityPlayer, r.DestinationCityID}
    local function part(f) return try(function() return tm[f](tm, args[1], args[2], args[3], args[4]) end) end
    local path = try(function() return tm:GetTradeRoutePath(args[1], args[2], args[3], args[4]) end)
    c.routes[#c.routes + 1] = {destPlayer = r.DestinationCityPlayer, destCity = r.DestinationCityID,
      pathLength = path and #path or nil,
      path = path,  -- plot indices the trader walks (route bonuses count the cities passed through)
      origin = {route = part("CalculateOriginYieldsFromPotentialRoute"), path = part("CalculateOriginYieldsFromPath"),
                modifiers = part("CalculateOriginYieldsFromModifiers")},
      destination = {route = part("CalculateDestinationYieldsFromPotentialRoute"), path = part("CalculateDestinationYieldsFromPath"),
                     modifiers = part("CalculateDestinationYieldsFromModifiers")}}
  end
  local cul = pCity:GetCulture()
  if cul then
    c.nextPlotCultureCost = try(function() return cul:GetNextPlotCultureCost() end)
    c.currentCulture = try(function() return cul:GetCurrentCulture() end)
    c.turnsUntilExpansion = try(function() return cul:GetTurnsUntilExpansion() end)
    local np = try(function() return cul:GetNextPlot() end)
    if np and np >= 0 then
      local p = Map.GetPlotByIndex(np)
      if p then c.nextPlot = {p:GetX(), p:GetY()} end
    end
  end
  -- Production costs and purchase prices for everything this city can make now.
  local q = pCity:GetBuildQueue()
  local gold = pCity:GetGold()
  local items = {}
  local function add(kind, row, costf)
    if q:CanProduce(row.Hash, true) then
      local it = {type = row[kind .. "Type"], cost = try(costf, row.Index), progress = try(function() return q:GetProgress(row.Hash) end)}
      it.gold = try(function() return gold:GetPurchaseCost(YieldTypes.GOLD, row.Hash) end)
      it.faith = try(function() return gold:GetPurchaseCost(YieldTypes.FAITH, row.Hash) end)
      items[#items + 1] = it
    end
  end
  for row in GameInfo.Units() do add("Unit", row, function(i) return q:GetUnitCost(i) end) end
  for row in GameInfo.Buildings() do add("Building", row, function(i) return q:GetBuildingCost(i) end) end
  for row in GameInfo.Districts() do add("District", row, function(i) return q:GetDistrictCost(i) end) end
  for row in GameInfo.Projects() do add("Project", row, function(i) return q:GetProjectCost(i) end) end
  c.items = items
  c.current = try(function() return q:CurrentlyBuilding() end)
  -- Tiles: yields of every tile the city owns, and gold price of nearby tiles it could buy.
  local owned, buyable = {}, {}
  local cx, cy = pCity:GetX(), pCity:GetY()
  for dx = -4, 4 do
    for dy = -4, 4 do
      local p = Map.GetPlotXYWithRangeCheck(cx, cy, dx, dy, 3)
      if p then
        local owner = p:GetOwner()
        if owner == pCity:GetOwner() and Cities.GetPlotPurchaseCity(p) == pCity then
          owned[#owned + 1] = {p:GetX(), p:GetY(), yields_of(function(i) return p:GetYield(i) end), p:GetWorkerCount()}
        elseif owner == -1 then
          local price = try(function() return gold:GetPlotPurchaseCost(p:GetIndex()) end)
          if price and price > 0 then buyable[#buyable + 1] = {p:GetX(), p:GetY(), price} end
        end
      end
    end
  end
  c.tiles = owned
  c.buyableTiles = buyable
  return c
end

local function dump_player(pid)
  local player = Players[pid]
  local out = {turn = Game.GetCurrentGameTurn(), player = pid}
  out.gold = try(function() return player:GetTreasury():GetGoldBalance() end)
  out.faith = try(function() return player:GetReligion():GetFaithBalance() end)
  local techs, civics = {}, {}
  local pt, pc = player:GetTechs(), player:GetCulture()
  for row in GameInfo.Technologies() do
    techs[#techs + 1] = {row.TechnologyType, try(function() return pt:GetResearchCost(row.Index) end),
      try(function() return pt:GetResearchProgress(row.Index) end), try(function() return pt:HasTech(row.Index) end),
      try(function() return pt:HasBoostBeenTriggered(row.Index) end)}
  end
  for row in GameInfo.Civics() do
    civics[#civics + 1] = {row.CivicType, try(function() return pc:GetCultureCost(row.Index) end),
      try(function() return pc:GetCulturalProgress(row.Index) end), try(function() return pc:HasCivic(row.Index) end),
      try(function() return pc:HasBoostBeenTriggered(row.Index) end)}
  end
  out.techs, out.civics = techs, civics
  -- the policies actually in the government's slots (the save's list can keep stale entries)
  local pol, slots = {}, {}
  local n = try(function() return pc:GetNumPolicySlots() end) or 0
  for i = 0, n - 1 do
    local idx = try(function() return pc:GetSlotPolicy(i) end)
    local row = idx and idx >= 0 and GameInfo.Policies[idx]
    pol[#pol + 1] = row and row.PolicyType or false
    local st = try(function() return pc:GetSlotType(i) end)
    local strow = st and GameInfo.GovernmentSlots and GameInfo.GovernmentSlots[st]
    slots[#slots + 1] = {strow and strow.GovernmentSlotType or st,
      row and try(function() return pc:IsPolicyObsolete(row.Index) end),
      row and try(function() return pc:IsPolicyBanned(row.Index) end)}
  end
  out.policies = pol
  out.policySlots = slots  -- per slot: slot type, policy obsolete?, policy banned (World Congress)?
  -- gold to unlock the policies outside a civic completion (0 while a free change is open), for its formula
  out.policyUnlockCost = try(function() return pc:GetCostToUnlockPolicies() end)
  out.policyChangesFree = try(function() return pc:PolicyChangeMade() end)
  local trade = player:GetTrade()
  out.tradeCapacity = try(function() return trade:GetOutgoingRouteCapacity() end)
  out.tradeRoutesUsed = try(function() return trade:GetNumOutgoingRoutes() end)
  local tr = player:GetTreasury()
  out.treasury = {goldYield = try(function() return tr:GetGoldYield() end),
    totalMaintenance = try(function() return tr:GetTotalMaintenance() end),
    buildingMaintenance = try(function() return tr:GetBuildingMaintenance() end),
    districtMaintenance = try(function() return tr:GetDistrictMaintenance() end),
    unitMaintenance = try(function() return tr:GetUnitMaintenance() end),
    maintenanceTip = try(function() return tr:GetTotalMaintenanceToolTip() end),
    goldTip = try(function() return tr:GetGoldYieldToolTip() end)}
  -- great person points per class: per turn and banked (the game's live values)
  local gpp = {}
  local gp = try(function() return player:GetGreatPeoplePoints() end)
  if gp then
    for row in GameInfo.GreatPersonClasses() do
      local per = try(function() return gp:GetPointsPerTurn(row.Index) end)
      local tot = try(function() return gp:GetPointsTotal(row.Index) end)
      if (per and per ~= 0) or (tot and tot ~= 0) then gpp[row.GreatPersonClassType] = {per, tot} end
    end
  end
  out.greatPersonPoints = gpp
  local cities = {}
  for _, pCity in player:GetCities():Members() do
    local ok, c = pcall(dump_city, pCity)
    cities[#cities + 1] = ok and c or {error = tostring(c)}
  end
  out.cities = cities
  -- Lua.log cuts lines at 2047 characters: write the JSON in numbered parts that the collector joins.
  out.civilization = PlayerConfigurations[pid] and PlayerConfigurations[pid]:GetCivilizationTypeName() or nil
  out.isHuman = player:IsHuman()
  local text = enc(out)
  local size = 1800
  local n = math.ceil(#text / size)
  dump_count = (dump_count or 0) + 1
  local id = tostring(out.turn) .. "_" .. tostring(pid) .. "_" .. tostring(dump_count)
  for i = 1, n do
    print("ECONPROBE_PART " .. id .. " " .. i .. " " .. n .. " " .. text:sub((i - 1) * size + 1, i * size))
  end
end

-- Every 10 turns: the game's own list of active modifiers (GameEffects), per owner player, with what each acts on.
-- Compared against our engine's ledger, it shows which modifiers we miss or apply wrongly.
local function dump_modifiers()
  local turn = Game.GetCurrentGameTurn()
  local ids = try(function() return GameEffects.GetModifiers() end)
  if ids == nil then print("ECONPROBE_ERROR GameEffects.GetModifiers unavailable") return end
  local rows = {}
  for _, id in ipairs(ids) do
    local active = try(function() return GameEffects.GetModifierActive(id) end)
    if active then
      local def = try(function() return GameEffects.GetModifierDefinition(id) end) or {}
      local owner = try(function() return GameEffects.GetModifierOwner(id) end)
      local pid = owner and try(function() return GameEffects.GetObjectsPlayerId(owner) end)
      if pid ~= nil and pid >= 0 and Players[pid] and Players[pid]:IsMajor() then
        local subj = try(function() return GameEffects.GetModifierSubjects(id) end) or {}
        local names = {}
        for k, o in ipairs(subj) do
          if k > 40 then break end
          names[#names + 1] = try(function() return GameEffects.GetObjectString(o) end) or tostring(o)
        end
        rows[#rows + 1] = {def.Id or def.ModifierId or tostring(id), pid,
          try(function() return GameEffects.GetObjectString(owner) end), #subj, names}
      end
    end
  end
  local text = enc({turn = turn, modifiers = rows})
  local size = 1800
  local n = math.ceil(#text / size)
  local mid = "mods_" .. tostring(turn)
  for i = 1, n do
    print("ECONPROBE_MODS " .. mid .. " " .. i .. " " .. n .. " " .. text:sub((i - 1) * size + 1, i * size))
  end
end

-- Every alive major civ (the AI's cities are a free source of cost/yield observations), once per turn.
local last_turn = -1
local function dump()
  local turn = Game.GetCurrentGameTurn()
  if turn == last_turn then return end
  last_turn = turn
  if turn % 10 == 0 or turn == 11 then
    local ok, err = pcall(dump_modifiers)
    if not ok then print("ECONPROBE_ERROR modifiers " .. tostring(err)) end
  end
  for pid = 0, 62 do
    local p = Players[pid]
    if p ~= nil and p:IsAlive() and p:IsMajor() then
      local ok, err = pcall(dump_player, pid)
      if not ok then print("ECONPROBE_ERROR " .. tostring(pid) .. " " .. tostring(err)) end
    end
  end
end

local function safe_dump()
  local ok, err = pcall(dump)
  if not ok then print("ECONPROBE_ERROR " .. tostring(err)) end
end

-- Harvests and feature removals: the owning city's state at the moment (the yield lands in the city at once, so
-- the change from the turn's first dump is the harvest amount).
local function on_removed(kind)
  return function(x, y)
    local ok, err = pcall(function()
      local plot = Map.GetPlot(x, y)
      if plot == nil then return end
      -- the city object an event hands out lacks the build queue: look the city up in its owner's list
      local c0 = Cities.GetPlotPurchaseCity(plot)
      if c0 == nil then return end
      local owner = c0:GetOwner()
      local pCity = Players[owner]:GetCities():FindID(c0:GetID()) or c0
      local q = try(function() return pCity:GetBuildQueue() end)
      local cur = q and try(function() return q:CurrentlyBuilding() end)
      local hash = q and try(function() return q:GetCurrentProductionTypeHash() end)
      local ev = {event = kind, turn = Game.GetCurrentGameTurn(), x = x, y = y, player = pCity:GetOwner(),
        city = pCity:GetName(), current = cur,
        progress = q and hash and try(function() return q:GetProgress(hash) end),
        food = try(function() return pCity:GetGrowth():GetFood() end),
        gold = try(function() return Players[pCity:GetOwner()]:GetTreasury():GetGoldBalance() end),
        faith = try(function() return Players[pCity:GetOwner()]:GetReligion():GetFaithBalance() end),
        feature = plot:GetFeatureType(), resource = plot:GetResourceType()}
      print("ECONPROBE_EVENT " .. enc(ev))
    end)
    if not ok then print("ECONPROBE_ERROR event " .. tostring(err)) end
  end
end
Events.FeatureRemovedFromMap.Add(on_removed("feature_removed"))
Events.ResourceRemovedFromMap.Add(on_removed("resource_removed"))

Events.LocalPlayerTurnBegin.Add(safe_dump)
Events.TurnBegin.Add(safe_dump)  -- also fires during autoplay, when there is no local turn
Events.LoadScreenClose.Add(safe_dump)
print("ECONPROBE loaded")
