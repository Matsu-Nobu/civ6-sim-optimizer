"""Generic Civ6 modifier engine.

Every game effect in the rules database is a modifier: a ModifierType (= CollectionType x EffectType from
DynamicModifiers), arguments, and two requirement sets (owner and subject). Modifiers are attached to an
owner by a source table (TraitModifiers, PolicyModifiers, BuildingModifiers, ...). The collection turns the
owner into subjects (e.g. all the player's cities); each subject that meets the subject requirements gets the
effect. EFFECT_ATTACH_MODIFIER hands another modifier to each subject, which then becomes that modifier's owner.

`Engine(rules, player, cities)` resolves all of this for one player state and files the results by effect type.
Consumers (tile yields, city yields, housing, amenities, production %) read the effects they understand.
Requirements or collections we can't evaluate make the modifier not apply; they are recorded in
`engine.unknown_requirements` / `engine.unknown_collections` so coverage can be checked.

Subjects are small tuples:
  ("player",)                    the player
  ("cs", envoy_entry)            a city-state the player has envoys with (owner of its suzerain/envoy bonuses)
  ("city", name)
  ("district", city_name, x, y)
  ("plot", city_name, x, y)      a plot as the subject of plot-yield effects
"""
from collections import defaultdict

from .envs import governor_here

from .rules import Rules

PLAYER = ("player",)
WATER = ("TERRAIN_COAST", "TERRAIN_OCEAN")


class Engine:
    def __init__(self, rules: Rules, player, cities):
        from .yields import plots_by_xy_of
        self.r = rules
        self.player = player
        self.cities = list(cities)
        self.by_name = {c["name"]: c for c in self.cities}
        self.plots = plots_by_xy_of(self.cities)
        self.plot_city = {(p["x"], p["y"]): c["name"] for c in self.cities for p in c["owned"]}
        self.techs = set(player.get("techs", ()))
        self.civics = set(player.get("civics", ()))
        self.effects = defaultdict(list)  # effect type -> [(subject, args, modifierId, source)]
        self.applied = {}  # modifierId -> subjects it acts on (0: active, no subject passed)
        self.plot_effects = defaultdict(list)  # city name -> [(yield, amount, plot reqs, test_any, modifierId)]
        self.unknown_requirements = defaultdict(set)  # requirement type -> modifier ids
        self.unknown_collections = defaultdict(set)
        self.religions = {r["religion"]: r for r in player.get("religions") or ()}
        # India (Dharma, "EFFECT_ADJUST_GAINS_ALL_FOLLOWER_BELIEFS"): follower beliefs of every religion with a follower in the city, not just the majority's
        self.all_follower_beliefs = any(
            self.r.q("select 1 from TraitModifiers tm join Modifiers m on m.ModifierId=tm.ModifierId join DynamicModifiers d "
                     "on d.ModifierType=m.ModifierType where tm.TraitType=? and d.EffectType='EFFECT_ADJUST_GAINS_ALL_FOLLOWER_BELIEFS'", t)
            for t in rules.traits_of(player.get("civilization"), player.get("leader")))
        for source, owner, mid in self._sources():
            self._religion = source.split("@", 1)[1] if "@" in str(source) else None
            self._apply(mid, owner, source, depth=0)
        self._religion = None

    def _great_person_sources(self, gp):
        ind = gp["individual"]
        need = self.r.q("select ActionRequiresCompletedDistrictType from GreatPersonIndividuals where GreatPersonIndividualType=?", ind)
        need = need[0][0] if need else None
        city = self.by_name.get(gp.get("city")) if gp.get("city") else None
        if city is None:  # where unknown: the first city (capital first) with the district it needs
            for c in sorted(self.cities, key=lambda c: not c.get("capital")):
                if need is None or any(d["complete"] and self.r.district_is(d["type"], need) for d in c["districts"]):
                    city = c
                    break
        for mid, target in self.r.q("select ModifierId, AttachmentTargetType from GreatPersonIndividualActionModifiers "
                                    "where GreatPersonIndividualType=?", ind):
            if target.endswith("PLAYER") or target.endswith("UNIT_GREATPERSON"):
                yield ind, PLAYER, mid
            elif city is None:
                continue
            elif target.endswith("_CITY") or target.endswith("WONDER_IN_TILE"):
                yield ind, ("city", city["name"]), mid
            elif target.endswith("DISTRICT_IN_TILE"):
                d = next((d for d in city["districts"] if d["complete"] and (need is None or self.r.district_is(d["type"], need))), None)
                if d is not None:
                    yield ind, ("district", city["name"], d["x"], d["y"]), mid
            # GREAT_PERSON_ACTION_ATTACHMENT_TARGET_UNIT_DOMAIN_MILITARY_IN_TILE: military units (out of scope)

    # ------------------------------------------------------------------ sources
    def _ids(self, sql, *args):
        return [m for (m,) in self.r.q(sql, *args)]

    def _sources(self):
        r, pl = self.r, self.player
        for t in r.traits_of(pl.get("civilization"), pl.get("leader")):
            for m in self._ids("select ModifierId from TraitModifiers where TraitType=?", t):
                yield t, PLAYER, m
        for p in pl.get("policies", []):
            for m in self._ids("select ModifierId from PolicyModifiers where PolicyType=?", p):
                yield p, PLAYER, m
        if pl.get("government"):
            for m in self._ids("select ModifierId from GovernmentModifiers where GovernmentType=?", pl["government"]):
                yield pl["government"], PLAYER, m
        if pl.get("pantheon"):
            for m in self._ids("select ModifierId from BeliefModifiers where BeliefType=?", pl["pantheon"]):
                yield pl["pantheon"], PLAYER, m
        # religions: beliefs of every religion we founded or that is the majority in one of our cities; their own
        # requirements (founder / city follows religion) decide where they apply
        followed = {(c.get("religion") or {}).get("majority") for c in self.cities} - {None}
        if self.all_follower_beliefs:
            followed |= {k for c in self.cities for k, n in ((c.get("religion") or {}).get("followers") or {}).items() if n > 0} - {"NONE"}
        for rel, r in self.religions.items():
            if rel in followed or r["founder"] == pl.get("playerId"):
                for b in r["beliefs"]:
                    for m in self._ids("select ModifierId from BeliefModifiers where BeliefType=?", b):
                        yield f"{b}@{rel}", PLAYER, m
        for c in pl.get("dedications", []):
            for m in self._ids("select ModifierId from CommemorationModifiers where CommemorationType=?", c):
                yield c, PLAYER, m
        for t in self.techs:
            for m in self._ids("select ModifierId from TechnologyModifiers where TechnologyType=?", t):
                yield t, PLAYER, m
        for t in self.civics:
            for m in self._ids("select ModifierId from CivicModifiers where CivicType=?", t):
                yield t, PLAYER, m
        for m in self._ids("select ModifierId from GameModifiers"):
            yield "game", PLAYER, m
        for c in self.cities:
            for b in c["buildings"]:
                for m in self._ids("select ModifierId from BuildingModifiers where BuildingType=?", b):
                    yield b, ("city", c["name"]), m
            for d in c["districts"]:
                if d["complete"]:
                    for m in self._ids("select ModifierId from DistrictModifiers where DistrictType=?", d["type"]):
                        yield d["type"], ("district", c["name"], d["x"], d["y"]), m
            for p in c["owned"]:
                if p["improvement"] and not p.get("pillaged"):
                    for m in self._ids("select ModifierId from ImprovementModifiers where ImprovementType=?", p["improvement"]):
                        yield p["improvement"], ("plot", c["name"], p["x"], p["y"]), m
        # great people used (activated): GreatPersonIndividualActionModifiers, attached to the player, the city or
        # the district of the tile where the great person was used (a district of the type it needs)
        for gp in pl.get("greatPeopleUsed") or ():
            yield from self._great_person_sources(gp)
        for g in pl.get("governors", []):
            city = next((c for c in self.cities if governor_here(g, c, pl)), None)
            if city is None or not g["established"]:
                continue
            owner = ("city", city["name"])
            for m in self._ids("select ModifierId from GovernorModifiers where GovernorType=?", g["type"]):
                yield g["type"], owner, m
            for p in g["promotions"]:
                for m in self._ids("select ModifierId from GovernorPromotionModifiers where GovernorPromotionType=?", p):
                    yield p, owner, m
        for e in pl.get("envoys", []):
            # the city-state's own traits (unique suzerain bonus) and its type's (envoy bonuses)
            leaders = [e["leader"]] + [b for (b,) in self.r.q("select InheritFrom from Leaders where LeaderType=?", e["leader"]) if b]
            for leader in leaders:
                for m in self._ids("select tm.ModifierId from LeaderTraits lt join TraitModifiers tm on tm.TraitType=lt.TraitType "
                                   "where lt.LeaderType=?", leader):
                    yield f"{e['civilization']} envoys", ("cs", e), m

    # ------------------------------------------------------------------ resolution
    def _modifier(self, mid):
        row = self.r.q("select m.ModifierType, m.SubjectRequirementSetId, m.OwnerRequirementSetId, d.CollectionType, d.EffectType "
                       "from Modifiers m left join DynamicModifiers d on d.ModifierType=m.ModifierType where m.ModifierId=?", mid)
        if not row:
            return None
        args = dict(self.r.q("select Name, Value from ModifierArguments where ModifierId=?", mid))
        return row[0] + (args,)

    def _apply(self, mid, owner, source, depth):
        m = self._modifier(mid)
        if m is None or depth > 4:
            return
        # A permanent modifier stays once its conditions were met: a "game era is X" condition then means "X or later".
        perm = self.r.q("select Permanent from Modifiers where ModifierId=?", mid)
        prev_perm = getattr(self, "_permanent", False)
        self._permanent = prev_perm or bool(perm and perm[0][0])
        try:
            self._apply_inner(mid, owner, source, depth, m)
        finally:
            self._permanent = prev_perm

    def _apply_inner(self, mid, owner, source, depth, m):
        mtype, subj_rs, owner_rs, collection, effect, args = m
        if owner_rs and self.reqset(owner_rs, owner, mid) is not True:
            return
        if effect == "EFFECT_ADJUST_PLOT_YIELD" and not args.get("ModifierId"):
            self.applied.setdefault(mid, 0)
            self.applied[mid] += 1  # plot conditions are tested per tile later; counted as acting here
            if not str(source).startswith("IMPROVEMENT_"):  # an improvement's own plot yields: see tile_yields
                self._plot_yield(mid, owner, collection, subj_rs, args)
            return
        subjects = self.collect(collection, owner, mid)
        self._owner = owner
        self.applied.setdefault(mid, 0)  # active (owner conditions met): compared with the game's GameEffects list
        for s in subjects:
            if subj_rs and self.reqset(subj_rs, s, mid) is not True:
                continue
            self.applied[mid] += 1
            if effect == "EFFECT_ATTACH_MODIFIER":
                if args.get("ModifierId"):
                    self._apply(args["ModifierId"], s, source, depth + 1)
            else:
                self.effects[effect].append((s, args, mid, source))

    def _plot_yield(self, mid, owner, collection, subj_rs, args):
        """Plot-yield effects are kept per city with their plot conditions unevaluated, so tile_yields can test
        them on the actual (or a hypothetical, for the planner) plot; other conditions are settled here."""
        from .yields import PLOT_REQUIREMENTS
        if "YieldType" not in args or "Amount" not in args:
            return
        reqs, test_any = (self.r.requirement_set(subj_rs) if subj_rs else ([], False))
        at = []
        if collection == "COLLECTION_CITY_PLOT_YIELDS":
            cities = [self._city_of(owner)]
        elif collection in ("COLLECTION_PLAYER_PLOT_YIELDS", "COLLECTION_ALL_PLOT_YIELDS"):
            cities = [c["name"] for c in self.cities]
        elif collection == "COLLECTION_SINGLE_PLOT_YIELDS":
            xy = self._xy_of(owner)
            if xy is None or xy not in self.plot_city:
                return
            cities = [self.plot_city[xy]]
            at = [("REQUIREMENT_PLOT_AT_XY", {"X": xy[0], "Y": xy[1]}, False)]
            if test_any:  # settle an any-of set now, on the plot itself
                if self.reqset(subj_rs, ("plot", cities[0], xy[0], xy[1]), mid) is not True:
                    return
                reqs, test_any = [], False
        else:
            self.unknown_collections[collection].add(mid)
            return
        for city in cities:
            if city is None:
                continue
            plot_reqs, results = [], []
            for t, a, inv in reqs:
                if t in PLOT_REQUIREMENTS:
                    plot_reqs.append((t, a, inv))
                    continue
                v = self.requirement(t, a, ("city", city), mid)
                if v is None:
                    break
                results.append(v != inv)
            else:
                if test_any:
                    if any(results):
                        plot_reqs, t_any = [], False
                    elif plot_reqs:
                        t_any = True
                    else:
                        continue
                elif all(results):
                    t_any = False
                else:
                    continue
                self.plot_effects[city].append((args["YieldType"], float(args["Amount"]), plot_reqs + at, t_any, mid))

    def _city_of(self, subj):
        if subj[0] in ("city", "district", "plot"):
            return subj[1]
        if subj[0] == "unit":
            return subj[2]
        return None

    def _xy_of(self, subj):
        if subj[0] in ("district", "plot"):
            return (subj[2], subj[3])
        if subj[0] == "city":
            c = self.by_name[subj[1]]
            return (c["center"]["x"], c["center"]["y"])
        return None

    def collect(self, collection, owner, mid):
        cities = [("city", c["name"]) for c in self.cities]
        if owner[0] == "cs":
            # a city-state's own modifiers act on the city-state (its cities, units); only those that reach out to
            # all players touch us. Bonuses its cities give to arriving trade routes: see route_yields.
            if collection in ("COLLECTION_ALL_PLAYERS", "COLLECTION_MAJOR_PLAYERS"):
                return [PLAYER]
            if collection == "COLLECTION_OWNER":
                return [owner]
            return []
        if collection == "COLLECTION_OWNER":
            return [owner]
        if collection in ("COLLECTION_PLAYER_CITIES", "COLLECTION_ALL_CITIES", "COLLECTION_PLAYER_BUILT_CITIES"):
            return cities
        if collection == "COLLECTION_PLAYER_CAPITAL_CITY":
            return [("city", c["name"]) for c in self.cities if c["capital"]]
        if collection == "COLLECTION_OWNER_CITY":
            name = self._city_of(owner)
            return [("city", name)] if name else []
        if collection in ("COLLECTION_PLAYER_DISTRICTS", "COLLECTION_ALL_DISTRICTS"):
            return [("district", c["name"], d["x"], d["y"]) for c in self.cities for d in c["districts"] if d["complete"]]
        if collection == "COLLECTION_CITY_DISTRICTS":
            name = self._city_of(owner)
            c = self.by_name.get(name)
            return [("district", name, d["x"], d["y"]) for d in c["districts"] if d["complete"]] if c else []
        if collection in ("COLLECTION_ALL_PLAYERS", "COLLECTION_MAJOR_PLAYERS"):
            return [PLAYER]
        if collection == "COLLECTION_PLAYER_CAPTURED_CITIES":
            return []  # conquest is not modelled
        # Units: the subject is a unit *type* (trained anywhere, or in one city). Only the civilian units the economy
        # uses are represented; their effects (build charges, ...) are read when such a unit is trained.
        if collection in ("COLLECTION_PLAYER_UNITS", "COLLECTION_PLAYER_TRAINED_UNITS", "COLLECTION_ALL_UNITS"):
            return [("unit", u, None) for u in ECON_UNITS]
        if collection == "COLLECTION_CITY_TRAINED_UNITS":
            name = self._city_of(owner)
            return [("unit", u, name) for u in ECON_UNITS] if name else []
        self.unknown_collections[collection].add(mid)
        return []

    # ------------------------------------------------------------------ requirements
    def reqset(self, rs, subj, mid):
        """True / False, or None when some requirement can't be evaluated."""
        reqs, test_any = self.r.requirement_set(rs)
        results = []
        for t, a, inv in reqs:
            v = self.requirement(t, a, subj, mid)
            if v is None:
                return None
            results.append(v != inv)
        return any(results) if test_any else all(results)

    def city(self, subj):
        name = self._city_of(subj)
        return self.by_name.get(name) if name else None

    def district(self, subj):
        if subj[0] != "district":
            return None
        c = self.by_name[subj[1]]
        return next((d for d in c["districts"] if (d["x"], d["y"]) == (subj[2], subj[3])), None)

    def plot(self, subj):
        xy = self._xy_of(subj)
        if xy is None:
            return None
        p = self.plots.get(xy)
        if p is not None and subj[0] == "city":
            p = dict(p, districtType="DISTRICT_CITY_CENTER")
        return p

    def requirement(self, rtype, args, subj, mid):
        from .yields import CITY_REQUIREMENTS, PLOT_REQUIREMENTS
        test = REQUIREMENTS.get(rtype)
        if test is not None:
            v = test(self, subj, args)
            if v is not None:
                return bool(v)
        elif rtype in PLOT_REQUIREMENTS:
            p = self.plot(subj)
            if p is not None:
                c = self.city(subj)
                from .yields import appeal_modifiers
                ctx = {"techs": self.techs, "civics": self.civics, "plots": self.plots, "city": c,
                       "appeal_mods": appeal_modifiers(self, c["name"]) if c and hasattr(self, "effects") else None}
                return bool(PLOT_REQUIREMENTS[rtype](self.r, p, args, ctx))
        elif rtype in CITY_REQUIREMENTS:
            c = self.city(subj)
            if c is not None:
                return bool(CITY_REQUIREMENTS[rtype](self.r, c, args))
        self.unknown_requirements[rtype].add(mid)
        return None

    # ------------------------------------------------------------------ queries
    def city_effects(self, effect, city_name):
        """[(args, modifierId, source)] of an effect on a city (subject = that city)."""
        return [(a, m, s) for subj, a, m, s in self.effects.get(effect, ()) if subj == ("city", city_name)]

    def district_effects(self, effect, city_name):
        """[(district, args, modifierId, source)] of an effect on the city's districts."""
        out = []
        for subj, a, m, s in self.effects.get(effect, ()):
            if subj[0] == "district" and subj[1] == city_name:
                out.append((self.district(subj), a, m, s))
        return out

    def owned_plot_effects(self, effect, city_name):
        return [(self.plots.get((subj[2], subj[3])), a, m, s) for subj, a, m, s in self.effects.get(effect, ())
                if subj[0] == "plot" and subj[1] == city_name]

    def grants_of(self, table, column, key, owner, source=None):
        """Effects of one source's modifiers (e.g. a building just finished), resolved on their own: for one-time
        grants (units, population, techs, boosts, gold) applied when the source is gained.
        Returns [(effect, subject, args, modifierId)]."""
        saved = self.effects
        self.effects = defaultdict(list)
        try:
            for (m,) in self.r.q(f"select ModifierId from {table} where {column}=?", key):
                self._religion = None
                self._apply(m, owner, source or key, depth=0)
            return [(eff, subj, a, mid) for eff, rows in self.effects.items() for subj, a, mid, src in rows]
        finally:
            self.effects = saved

    def unit_effects(self, effect, unit, city_name=None):
        """[(args, modifierId, source)] of an effect on a unit type trained (in city_name, if given)."""
        return [(a, m, s) for subj, a, m, s in self.effects.get(effect, ())
                if subj[0] == "unit" and subj[1] == unit and (subj[2] is None or subj[2] == city_name)]

    def player_effects(self, effect):
        return [(a, m, s) for subj, a, m, s in self.effects.get(effect, ()) if subj == PLAYER]


# ---------------------------------------------------------------------- requirement tests
# test(engine, subject, args) -> bool, or None when it can't be evaluated for this subject.

def _governor_titles(e, city, established=True):
    """Titles of the governor in the city: one per promotion, the level-0 ability included (Toronto's Reyna with
    only Land Acquisition does not meet Civil Prestige's two titles)."""
    for g in e.player.get("governors", []):
        if governor_here(g, city, e.player) and (g["established"] or not established):
            return len(g["promotions"])
    return 0


def _has_fresh_water(e, s, a):
    from .growth import has_fresh_water
    c = e.city(s)
    if s[0] == "city" and c is not None:
        return has_fresh_water(c)
    p = e.plot(s)
    if p is None:
        return None
    from .yields import neighbour_xy
    return bool(p.get("river")) or any((e.plots.get(xy) or {}).get("lake") or (e.plots.get(xy) or {}).get("feature") == "FEATURE_OASIS"
                                       for xy in neighbour_xy(p["x"], p["y"]))


def _adjacent_to_coast(e, s, a):
    p = e.plot(s)
    if p is None:
        return None
    from .yields import neighbour_xy
    return any((e.plots.get(xy) or {}).get("terrain") == "TERRAIN_COAST" and not (e.plots.get(xy) or {}).get("lake")
               for xy in neighbour_xy(p["x"], p["y"]))


def _era_at_least(e, s, a):
    eras = [x for (x,) in e.r.q("select EraType from Eras order by ChronologyIndex")]
    era = e.player.get("era") or e.player.get("gameEra")
    if not era or a.get("EraType") not in eras:
        return None
    return eras.index(era) >= eras.index(a["EraType"])


def _district_type(e, s, a):
    d = e.district(s)
    if d is not None:
        return d["type"] == a["DistrictType"]
    p = e.plot(s)
    return None if p is None else p.get("districtType") == a["DistrictType"]


def _cs_tokens(e, s, a):
    """Envoys our player has given to the city-state that owns the modifier being resolved."""
    cs = s if s[0] == "cs" else getattr(e, "_owner", None)
    if cs and cs[0] == "cs":
        return cs[1]["count"] >= int(a["MinimumTokens"])
    return None


def _game_era_index(e, era):
    eras = [x for (x,) in e.r.q("select EraType from Eras order by ChronologyIndex")]
    cur = e.player.get("gameEra")
    if not cur or era not in eras:
        return None, None
    return eras.index(cur), eras.index(era)


def _game_era_is(e, s, a):
    cur, want = _game_era_index(e, a["EraType"])
    if cur is None:
        return None
    return cur >= want if getattr(e, "_permanent", False) else cur == want


def _game_era_at_least(e, s, a):
    cur, want = _game_era_index(e, a["EraType"])
    return None if cur is None else cur >= want


def _near_building(e, s, a):
    """A plot within [MinRange, MaxRange] of a finished building (a wonder's own plot, else its city center)."""
    from .yields import hex_distance
    p = e.plot(s)
    if p is None:
        return None
    b = a["BuildingType"]
    spots = [xy for xy, q in e.plots.items() if q.get("wonder") == b and q.get("wonderComplete")]
    spots += [(c["center"]["x"], c["center"]["y"]) for c in e.cities if b in c["buildings"] and not spots]
    lo, hi = int(a.get("MinRange") or 0), int(a.get("MaxRange") or 1)
    return any(lo <= hex_distance((p["x"], p["y"]), xy) <= hi for xy in spots)


def _amani_cs_ids(e):
    """(city id, owner) of the city-state cities where our ambassador (a governor that may serve city-states) is
    established."""
    me = e.player.get("playerId")
    cs_govs = {g for (g,) in e.r.q("select GovernorType from Governors where AssignCityState=1")}
    return {(g.get("cityId"), g.get("cityOwnerId")) for g in e.player.get("governors") or ()
            if g.get("type") in cs_govs and g.get("established") and g.get("cityOwnerId") not in (None, me)}


def _plot_property(e, s, a):
    """Plot properties BBG's script sets (REQUIREMENT_PLOT_PROPERTY_MATCHES): Amani established in a city-state, and
    a city with a trade route to that city-state (BBG Amani: +2 food +2 production in such cities). Other properties
    (capture / combat flags) are outside the model."""
    name = a.get("PropertyName")
    if name == "AMANI_ESTABLISHED_CS":
        return bool(_amani_cs_ids(e))
    if name == "TRADER_TO_AMANI_CS":
        c = e.city(s)
        if c is None:
            return None
        where = _amani_cs_ids(e)
        return any(isinstance(r, dict) and r["origin"].get("name") == c["name"]
                   and (r["destination"].get("id"), r["destination"].get("playerId")) in where
                   for r in e.player.get("tradeRoutes") or ())
    return None


def _majority(e, s):
    c = e.city(s)
    if c is None or c.get("religion") is None:
        return None, False
    return c["religion"]["majority"], True


def _follows_religion(e, s):
    """The city's majority religion is the religion of the belief being applied."""
    if e._religion is None:
        return None
    m, known = _majority(e, s)
    if known and m != e._religion and e.all_follower_beliefs:
        c = e.city(s)
        return ((c.get("religion") or {}).get("followers") or {}).get(e._religion, 0) > 0
    return None if not known else m == e._religion


def _follows_pantheon(e, s):
    """A pantheon works in every city of its owner, whatever religion the city follows (rome1: Scythia's Sacred Path
    in Pokrovka, converted to Gran Colombia's Catholicism; assuming it lapses there cost 31 city-turns)."""
    return True


def _founded_religion(e):
    """The player founded the religion of the belief being applied."""
    if e._religion is None:
        return any(r["founder"] == e.player.get("playerId") for r in e.religions.values())
    r = e.religions.get(e._religion)
    return bool(r) and r["founder"] == e.player.get("playerId")


def _cs_flag(e, s, key):
    cs = s if s[0] == "cs" else getattr(e, "_owner", None)
    if cs and cs[0] == "cs":
        return bool(cs[1].get(key))
    return None


def _requirementset(e, s, a):
    return e.reqset(a["RequirementSetId"], s, None)


ECON_UNITS = ("UNIT_BUILDER", "UNIT_SETTLER", "UNIT_TRADER", "UNIT_MILITARY_ENGINEER", "UNIT_SCOUT")


def _unit_field(e, s, col):
    if s[0] != "unit":
        return None
    row = e.r.q(f"select {col} from Units where UnitType=?", s[1])
    return row[0][0] if row else None


REQUIREMENTS = {
    "REQUIREMENT_PLAYER_HAS_TECHNOLOGY": lambda e, s, a: a["TechnologyType"] in e.techs,
    "REQUIREMENT_UNIT_TYPE_MATCHES": lambda e, s, a: None if s[0] != "unit" else s[1] == a["UnitType"],
    "REQUIREMENT_UNIT_PROMOTION_CLASS_MATCHES": lambda e, s, a: None if s[0] != "unit" else
        _unit_field(e, s, "PromotionClass") == a["UnitPromotionClass"],
    "REQUIREMENT_UNIT_DOMAIN_MATCHES": lambda e, s, a: None if s[0] != "unit" else _unit_field(e, s, "Domain") == a["UnitDomain"],
    "REQUIREMENT_UNIT_TAG_MATCHES": lambda e, s, a: None if s[0] != "unit" else a["Tag"] in e.r.tags_of(s[1]),
    "REQUIREMENT_PLAYER_HAS_CIVIC": lambda e, s, a: a["CivicType"] in e.civics,
    "REQUIREMENT_PLAYER_LEADER_TYPE_MATCHES": lambda e, s, a: (
        s[1]["leader"] if s[0] == "cs" else e.player.get("leader")) == a.get("LeaderType"),
    "REQUIREMENT_PLAYER_CIVILIZATION_TYPE_MATCHES": lambda e, s, a: (
        s[1]["civilization"] if s[0] == "cs" else e.player.get("civilization")) == a.get("CivilizationType"),
    "REQUIREMENT_PLAYER_IS_AT_PEACE": lambda e, s, a: True,  # wars are not modelled
    "REQUIREMENT_PLAYER_HAS_GIVEN_INFLUENCE_TOKENS": _cs_tokens,
    "REQUIREMENT_PLAYER_IS_SUZERAIN": lambda e, s, a: _cs_flag(e, s, "suzerain"),
    "REQUIREMENT_PLAYER_IS_SUZERAIN_BONUS_ENABLED": lambda e, s, a: True,
    "REQUIREMENT_PLAYER_ERA_AT_LEAST": _era_at_least,
    "REQUIREMENT_PLAYER_HAS_RESOURCE_VISIBILITY": lambda e, s, a: e.r.resource_revealed(a["ResourceType"], e.techs),
    "REQUIREMENT_REQUIREMENTSET_IS_MET": _requirementset,
    "REQUIREMENT_DISTRICT_TYPE_MATCHES": _district_type,
    "REQUIREMENT_CITY_HAS_GOVERNOR_WITH_X_TITLES": lambda e, s, a: None if e.city(s) is None else
        _governor_titles(e, e.city(s), a.get("Established") == "1") >= int(a["Amount"]),
    "REQUIREMENT_CITY_HAS_GOVERNOR": lambda e, s, a: None if e.city(s) is None else _governor_titles(e, e.city(s)) > 0,
    "REQUIREMENT_CITY_HAS_X_POPULATION": lambda e, s, a: None if e.city(s) is None else
        e.city(s)["population"] >= int(a["Amount"]),
    "REQUIREMENT_CITY_IS_CAPITAL": lambda e, s, a: None if e.city(s) is None else bool(e.city(s)["capital"]),
    "REQUIREMENT_CITY_FOLLOWS_PANTHEON": lambda e, s, a: _follows_pantheon(e, s),
    "REQUIREMENT_CITY_FOLLOWS_RELIGION": lambda e, s, a: _follows_religion(e, s),
    "REQUIREMENT_CITY_IS_ORIGINAL_OWNER": lambda e, s, a: True,  # conquest is not modelled
    "REQUIREMENT_PLOT_IS_FRESH_WATER": _has_fresh_water,
    "REQUIREMENT_PLOT_ADJACENT_TO_COAST": _adjacent_to_coast,
    "REQUIREMENT_PLOT_IS_COAST": lambda e, s, a: None if e.plot(s) is None else e.plot(s)["terrain"] == "TERRAIN_COAST",
    "REQUIREMENT_PLOT_NEAR_CAPITAL": lambda e, s, a: None,
    "REQUIREMENT_GAME_ERA_IS": _game_era_is,
    "REQUIREMENT_GAME_ERA_ATLEAST_EXPANSION": _game_era_at_least,
    # natural-wonder bonuses are attached by the game when the map has the wonder; their effects (adjacency to
    # the feature) can only fire next to it, so treating the wonder as present is harmless
    "REQUIREMENT_MAP_HAS_FEATURE": lambda e, s, a: True,
    "REQUIREMENT_PLAYER_FOUNDED_OUR_RELIGION": lambda e, s, a: _founded_religion(e),
    "REQUIREMENT_PLAYER_HAS_BUILDING": lambda e, s, a: any(a["BuildingType"] in c["buildings"] for c in e.cities),
    "REQUIREMENT_PLAYER_HAS_DISTRICT": lambda e, s, a: any(
        d["type"] == a["DistrictType"] and d["complete"] for c in e.cities for d in c["districts"]),
    "REQUIREMENT_PLAYER_HAS_IMPROVEMENT": lambda e, s, a: any(
        p["improvement"] == a["ImprovementType"] for c in e.cities for p in c["owned"]),
    "REQUIREMENT_PLOT_ADJACENT_BUILDING_TYPE_MATCHES": _near_building,
    "REQUIREMENT_CITY_HAS_FULL_LOYALTY": lambda e, s, a: True,  # loyalty is not modelled; our cities sit at full
    "REQUIREMENT_PLAYER_TYPE_MATCHES": lambda e, s, a: (
        s[1]["civilization"] if s[0] == "cs" else e.player.get("civilization")) == a.get("CivilizationType"),
    "REQUIREMENT_PLAYER_IS_MINOR": lambda e, s, a: s[0] == "cs",
    "REQUIREMENT_PLAYER_HAS_PANTHEON": lambda e, s, a: bool(e.player.get("pantheon")),
    "REQUIREMENT_PLAYER_HAS_GOLDEN_AGE": lambda e, s, a: e.player.get("age") in ("GOLDEN", "HEROIC"),
    "REQUIREMENT_PLAYER_HAS_DARK_AGE": lambda e, s, a: e.player.get("age") == "DARK",
    "REQUIREMENT_PLAYER_HAS_NORMAL_AGE": lambda e, s, a: e.player.get("age") == "NORMAL",
    "REQUIREMENT_PLAYER_ALWAYS_ALLOWED_COMMEMORATION_QUEST": lambda e, s, a: True,
    "REQUIREMENT_PLAYER_CAN_EVER_EARN_GREAT_PERSON_CLASS": lambda e, s, a: True,
    "REQUIREMENT_PLAYER_OWNS_OBJECT": lambda e, s, a: True,  # the object is the modifier's own source
    "REQUIREMENT_PLAYER_HAS_FEATURE": lambda e, s, a: any(p["feature"] == a["FeatureType"] for c in e.cities for p in c["owned"]),
    "REQUIREMENT_CITY_HAS_FEATURE": lambda e, s, a: None if e.city(s) is None else any(
        p["feature"] == a["FeatureType"] for p in e.city(s)["owned"]),
    "REQUIREMENT_PLAYER_FOUNDED_RELIGION_WITH_BELIEF": lambda e, s, a: any(
        r["founder"] == e.player.get("playerId") and a.get("BeliefType") in r["beliefs"] for r in e.religions.values()),
    "REQUIREMENT_PLOT_PROPERTY_MATCHES": _plot_property,
    "REQUIREMENT_PLAYER_IS_RELIGION_FOUNDER": lambda e, s, a: _founded_religion(e),
    "REQUIREMENT_PLAYER_HAS_AT_LEAST_NUMBER_CITIES": lambda e, s, a: len(e.cities) >= int(a["Amount"]),
}


# ---------------------------------------------------------------------- cache
_CACHE = {}


def engine_for(rules: Rules, player, cities):
    """Engine for a player state, cached on the parts of the state modifiers depend on."""
    cities = list(cities)
    key = (id(rules), player.get("gameEra"), player.get("civilization"), player.get("leader"), len(player.get("techs", ())),
           len(player.get("civics", ())), tuple(player.get("policies", ())), player.get("government"),
           player.get("pantheon"), player.get("age"), player.get("majorsMet"), tuple(player.get("dedications", ())), repr(player.get("governors")), repr(player.get("envoys")), repr(player.get("religions")), player.get("prophetRecruitedTurn"), player.get("prophetOnOffer"), repr(player.get("tradeRoutes")), repr(player.get("greatPeopleUsed")), repr(player.get("greatPeopleOnOffer")),
           tuple((c["name"], c["capital"], c["population"], repr(c.get("religion")), tuple(c["buildings"]),
                  tuple((d["type"], d["x"], d["y"], d["complete"]) for d in c["districts"]),
                  tuple((p["x"], p["y"], p["improvement"], p.get("pillaged"), p["feature"]) for p in c["owned"]))
                 for c in cities))
    e = _CACHE.get(key)
    if e is None:
        if len(_CACHE) > 256:
            _CACHE.clear()
        e = _CACHE[key] = Engine(rules, player, cities)
    return e
