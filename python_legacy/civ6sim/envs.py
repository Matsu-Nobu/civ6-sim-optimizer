"""The player environment the simulator reads, built from one exported row: the player record plus the
game-wide parts the modifiers need (game era, founded religions, trade routes)."""


def player_of(row, game_era=None):
    """The AI uses a great person the turn it recruits one, so recruited = used (where: see Engine)."""
    p = row["player"]
    return dict(p, gameEra=game_era or row.get("gameEra"), religions=row.get("religions"),
                tradeRoutes=row.get("tradeRoutes") or [],
                greatPeopleUsed=p.get("greatPeopleUsed") or [{"individual": x["individual"]} for x in p.get("greatPeopleRecruited") or ()])


def governor_here(g, city, player):
    """Is governor record g serving in this city of ours? City ids repeat across players (our capital and a
    city-state's city can both be 65536), so the city's owner must be us (cityOwnerId; absent in plans = ours)."""
    owner = g.get("cityOwnerId")
    return g.get("cityId") == city.get("id") and (owner is None or owner == player.get("playerId")
                                                   or player.get("playerId") is None)


def routes_from_probe(probe_row, export_row):
    """The game's own outgoing trade routes (probe) in the route format, with the destination from the exported
    world / own cities; plus routes of other players arriving in our cities (from the save). The save's own route
    records miss and invent routes, so validation uses these."""
    pid = export_row["player"]["playerId"]
    world = {(c["playerId"], c["id"]): c for c in export_row.get("worldCities") or ()}
    mine = {c["id"]: c for c in export_row["cities"]}
    w = export_row.get("mapWidth") or 0
    out = []
    for pc in probe_row["cities"]:
        name = pc["name"].replace("LOC_CITY_NAME_", "")
        origin = next((c for c in export_row["cities"] if c["name"] == name), None)
        if origin is None:
            continue
        for rt in pc.get("routes") or ():
            key = (rt["destPlayer"], rt["destCity"])
            if rt["destPlayer"] == pid:
                d = mine.get(rt["destCity"])
                if d is None:
                    continue
                dest = {"name": d["name"], "id": d["id"], "playerId": pid,
                        "civilization": export_row["player"]["civilization"], "x": d["center"]["x"], "y": d["center"]["y"],
                        "districts": [x["type"] for x in d["districts"] if x["complete"]],
                        "plots": [(p["terrain"], p["resource"], p["improvement"]) for p in d["owned"]]}
            else:
                dest = world.get(key)
                if dest is None:
                    continue
            route = {"origin": {"name": name, "id": origin["id"], "playerId": pid}, "destination": dest}
            if rt.get("path") and w:
                on = {(i % w, i // w) for i in rt["path"]}
                route["foreignCitiesOnPath"] = sum(1 for c in world.values() if (c["x"], c["y"]) in on)
            out.append(route)
    out += [r for r in export_row.get("tradeRoutes") or () if r["origin"]["playerId"] != pid]
    return out
