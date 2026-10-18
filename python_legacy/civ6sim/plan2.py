"""Full-action planner: every economic decision is a choice the search can change.

Decision points each turn (in this order), each identified by a stable key:
  ("research", T)            next tech when none is being researched
  ("civic", T)               next civic
  ("gov", T)                 government, when a new one is unlocked
  ("policy", T, slot)        each policy slot, after a civic completes or the government changes
  ("pantheon", T)            once faith covers the cost
  ("settle", T, i)           destination of a settler that has just been trained
  ("trade", T, i)            destination of a trader that has just been trained
  ("builder", T, i)          each builder's action this turn: improve / harvest / wait
  ("focus", T, city)         citizen placement policy of a city, when its population changes
  ("prod", T, city)          what a city builds next, when it is idle
  ("buy", T)                 one gold purchase per turn (current item in a city, a tile, or nothing)

Not decided (outside the economy model): exploration and combat boosts (meet civ, natural wonder,
camps, kills) and envoys happen on the turns they happened in the recorded game.
"""
import copy
from pathlib import Path
import math
from dataclasses import dataclass, field

from .amenities import amenity_scores, happiness
from .production import gold_price, item_cost, item_kind
from .sim import CityAcc, PlayerAcc, city_turn_yields, research_cost, step
from .world import DEFAULT_WEIGHTS, World
from .yields import district_adjacency, excluded_adjacencies, modifier_adjacency_rows, trade_route_yields

FOCUS = {
    "balanced": DEFAULT_WEIGHTS,
    "food": dict(DEFAULT_WEIGHTS, YIELD_FOOD=2.0),
    "production": dict(DEFAULT_WEIGHTS, YIELD_PRODUCTION=2.0),
}
# Boost triggers that come from exploration / combat: taken from the recorded game's timing.
EXOGENOUS_TRIGGERS = {"BOOST_TRIGGER_MEET_CIV", "BOOST_TRIGGER_MEET_X_CITY_STATES", "BOOST_TRIGGER_FIND_NATURAL_WONDER",
                      "BOOST_TRIGGER_KILL_WITH", "BOOST_TRIGGER_CLEAR_CAMP", "BOOST_TRIGGER_NUM_BARBS_KILLED",
                      "BOOST_TRIGGER_RECEIVE_DOW"}
PURCHASE_ROUNDING = 5
# Gold purchase price: production.gold_price (4 x cost, floor to 5), confirmed against the game's own prices.
def rules_district_is(rules, district, base):
    return district == base or rules.district_is(district, base)


NEW_CITY_NAMES = [f"CITY_{i}" for i in range(2, 20)]


@dataclass
class Settings:
    horizon: int = 31
    max_cities: int = 20
    settle_candidates: int = 5
    settle_radius: int = 8
    builder_candidates: int = 4
    score_weights: dict = field(default_factory=lambda: {"YIELD_PRODUCTION": 1.0, "YIELD_SCIENCE": 1.0,
                                                          "YIELD_CULTURE": 1.0, "YIELD_GOLD": 0.5, "YIELD_FAITH": 0.2})
    terminal_turns: int = 10
    sites: list = None  # force settling sites in order [(name, (x, y))] (for evaluating the recorded game)
    # Fixed choices the search may not change: (kind, entity, n) -> choice. "CAPITAL" stands for the capital's name.
    # Default: the capital's first build is a Scout (exploration is outside the model, and the opening scout is
    # always built in practice).
    fixed: dict = field(default_factory=lambda: {("prod", "CAPITAL", 0): "UNIT_SCOUT"})
    # The game picks the plot a border takes partly at random (its next plot changes from turn to turn with nothing
    # else changing); for checking against a game run: (city centre, n-th plot taken) -> the plot the game took.
    borders: dict = None
    # Options always offered (for searching from the recorded game: its city sites and builder actions stay
    # choosable), and the recorded names of those sites (so choices naming a city keep matching).
    extra_sites: list = None
    site_names: dict = None
    extra_builder: list = None
    # Replaying the recorded game: its cities' tiles, city centre (x, y) -> [(turn, (x, y))] in the order the recorded
    # city got them (borders and purchases); a city at a recorded centre gets each on that turn (none bought, none
    # by culture). The game picks border tiles partly at random, and which tiles were bought is not in the save.
    recorded_tiles: dict = None


@dataclass
class State:
    turn: int
    acc: PlayerAcc
    cities: list
    player: dict  # policies, government, pantheon, governors, envoys, civilization, leader
    builders: list = field(default_factory=list)  # [{"charges": n, "at": (x, y), "ready": turn}]
    settlers: list = field(default_factory=list)  # [{"from": city, "target": xy, "arrive": turn}]
    traders: list = field(default_factory=list)  # [{"origin": city, "dest": city}]
    units: dict = field(default_factory=dict)  # unit type -> count alive (non-civilian)
    research: str = None
    civic: str = None
    policy_dirty: bool = True
    known_govs: set = field(default_factory=set)
    border: dict = field(default_factory=dict)
    focus: dict = field(default_factory=dict)
    free_building_given: set = field(default_factory=set)
    gov_plaza: bool = False
    score: float = 0.0
    last_output: dict = field(default_factory=dict)
    log: list = field(default_factory=list)  # (key, turn, choice)
    counters: dict = field(default_factory=dict)
    bought: list = field(default_factory=list)  # (city, item) bought this turn
    tiles_bought: int = 0
    gp: dict = field(default_factory=dict)  # great person class -> points banked
    gp_ours: list = field(default_factory=list)  # individuals recruited by the plan
    gp_pending: list = field(default_factory=list)  # recruited, waiting for a place to act (district, wonder)
    trace: list = None  # when a list: what happened each turn, for replaying the plan in the game (python_legacy/scripts/plan_export.py)


def _tr(st, *ev):
    if st.trace is not None:
        st.trace.append(ev)


def produced_between(a, b, unit):
    """How many of `unit` cities finished between saves a and b (progress reset or switched away after it)."""
    n = 0
    after = {c["name"]: c for c in b["cities"]}
    for c in a["cities"]:
        c2 = after.get(c["name"])
        if c2 and c["production"]["current"] == unit and (
                c2["production"]["current"] != unit
                or c2["production"]["progress"].get(unit, 0) < c["production"]["progress"].get(unit, 0)):
            n += 1
    return n


class Scenario:
    """Exogenous parts of the recorded game, by turn."""

    def __init__(self, rows, rules):
        self.by_turn = {d["turn"]: d for d in rows}
        self.rows, self.rules = rows, rules
        self.exo_boosts = {}
        for a, b in zip(rows, rows[1:]):
            old = set(a["player"]["techsBoosted"] + a["player"]["civicsInspired"])
            for t in b["player"]["techsBoosted"] + b["player"]["civicsInspired"]:
                if t not in old and rules.boost_class(t) in EXOGENOUS_TRIGGERS:
                    self.exo_boosts.setdefault(a["turn"], []).append(t)
        self.faith_gift = self.faith_gifts(rows)
        # Gold from tribal villages: the gold jump on a turn the hut count went up, beyond the turn's income.
        self.gold_gift = {}
        for a, b in zip(rows, rows[1:]):
            hut = lambda p: sum((p.get("goodyHutsReceived") or {}).values())
            got = (b["player"].get("goodyHutsReceived") or {}).get("GOODYHUT_GOLD", 0) - \
                (a["player"].get("goodyHutsReceived") or {}).get("GOODYHUT_GOLD", 0)
            if hut(b["player"]) > hut(a["player"]) and got > 0:
                self.gold_gift[a["turn"]] = max(0.0, b["player"]["gold"] - a["player"]["gold"]
                                                - (a["player"].get("goldPerTurn") or 0))
        # Recon units trained by each turn in the recorded game: exploration rewards are only granted to a plan
        # that has scouted at least as much by then.
        self.scouts_by_turn = {d["turn"]: d["player"]["unitsTrained"].get("UNIT_SCOUT", 0) for d in rows}
        # Units from tribal villages ("survivors" / "military" huts): new units not produced by any city on a
        # turn the hut count went up.
        self.unit_gifts = {}
        for a, b in zip(rows, rows[1:]):
            huts_a = sum((a["player"].get("goodyHutsReceived") or {}).values())
            huts_b = sum((b["player"].get("goodyHutsReceived") or {}).values())
            if huts_b <= huts_a:
                continue
            for u, n in b["player"]["unitsTrained"].items():
                extra = n - a["player"]["unitsTrained"].get(u, 0)
                if extra > produced_between(a, b, u):
                    self.unit_gifts.setdefault(a["turn"], []).append(u)

    # Game (world) era switches observed in game_auto2 (online speed): every ~30 turns. Used past the recorded rows.
    DEFAULT_ERA_TURNS = [(1, "ERA_ANCIENT"), (31, "ERA_CLASSICAL"), (61, "ERA_MEDIEVAL"), (89, "ERA_RENAISSANCE"),
                         (119, "ERA_INDUSTRIAL")]

    def game_era(self, turn):
        """The game era is set by all players together: from the recorded game where known, else the default timing."""
        d = self.by_turn.get(turn)
        if d and d.get("gameEra"):
            return d["gameEra"]
        era = "ERA_ANCIENT"
        for t, e in self.DEFAULT_ERA_TURNS:
            if turn >= t:
                era = e
        return era

    def faith_gifts(self, rows):
        """Faith that arrived from outside the cities (tribal villages, ...): stock jumps beyond city faith."""
        out = {}
        for a, b in zip(rows, rows[1:]):
            fa, fb = a["player"].get("faithStock") or 0, b["player"].get("faithStock") or 0
            city = sum(y["modified"]["YIELD_FAITH"] for y in a["cityYields"]) if a.get("cityYields") else 0
            spent = 0
            if b["player"].get("pantheon") and not a["player"].get("pantheon"):
                spent = self.rules.param("RELIGION_PANTHEON_MIN_FAITH") * 0.5
            gift = fb - fa - city + spent
            if gift > 0.5:
                out[a["turn"]] = gift
        return out

    def envoys(self, turn):
        d = self.by_turn.get(turn) or self.rows[-1]
        return d["player"].get("envoys", [])

    def world_cities(self, turn):
        """Other players' cities (route endpoint format) in the recorded game at `turn`."""
        d = self.by_turn.get(turn) or self.rows[-1]
        return d.get("worldCities") or []

    def wonders_taken(self, turn):
        """World wonders other players have finished by `turn` (from the recorded game)."""
        d = self.by_turn.get(turn) or self.rows[-1]
        me = d["player"].get("playerId")
        return {w["building"] for w in d.get("worldWonders") or () if w["playerId"] != me}

    def gp_timeline(self):
        """Per great person class: [(individual, first turn on offer, last turn on offer)] in the order the recorded
        game offered them (the timeline is shared by all players)."""
        if getattr(self, "_gp_timeline", None) is None:
            seen = {}
            for d in self.rows:
                for cls, ind in (d["player"].get("greatPeopleOnOffer") or {}).items():
                    if ind:
                        f, _ = seen.get((cls, ind), (d["turn"], d["turn"]))
                        seen[(cls, ind)] = (f, d["turn"])
            own = {g["individual"] for d in self.rows for g in d["player"].get("greatPeopleRecruited") or ()}
            out = {}
            for (cls, ind), (f, l) in sorted(seen.items(), key=lambda kv: kv[1][0]):
                # still on offer at the last recorded turn, or taken by the recorded player itself: not taken by others
                last_row = self.rows[-1]["turn"]
                taken = None if (l >= last_row or ind in own) else l + 1
                out.setdefault(cls, []).append((ind, f, taken))
            self._gp_timeline = out
        return self._gp_timeline

    def gp_offer(self, cls, turn, ours):
        """The individual on offer in a class at `turn`: the first of the recorded order that others have not taken
        by then and the plan has not recruited. (The game era for the cost comes from game_era.)"""
        for ind, first, taken in self.gp_timeline().get(cls, ()):
            if ind in ours or (taken is not None and taken <= turn):
                continue
            return ind
        return None

    def prophet_on_offer(self, turn):
        """Whether a great prophet is still on offer (the other players claim them; the plan never recruits one)."""
        d = self.by_turn.get(turn) or self.rows[-1]
        return d["player"].get("prophetOnOffer", True)


class FullPlanner:
    def __init__(self, rules, world: World, scenario: Scenario, settings: Settings, speed="GAMESPEED_ONLINE"):
        self.r, self.w, self.sc, self.s, self.speed = rules, world, scenario, settings, speed

    # ------------------------------------------------------------------ start state
    def initial_state(self, turn):
        d = self.sc.by_turn[turn]
        p = d["player"]
        acc = PlayerAcc(gold=p["gold"], tech_progress=dict(p["techProgress"]), civic_progress=dict(p["civicProgress"]),
                        techs=set(p["techs"]), civics=set(p["civics"]), units_trained=dict(p["unitsTrained"]),
                        faith=p.get("faithStock") or 0.0)
        cities = copy.deepcopy(d["cities"])
        for c in cities:
            c["production"]["current"] = None
            acc.cities[c["name"]] = CityAcc(population=c["population"], food_store=c["foodStore"] or 0.0,
                                            progress=dict(c["production"]["progress"]))
        player = {k: copy.deepcopy(p.get(k)) for k in ("civilization", "leader", "policies", "government", "pantheon",
                                                        "governors", "envoys", "playerId")}
        player["policies"] = list(p["policies"])
        player["government"] = p["government"] or "GOVERNMENT_CHIEFDOM"
        units = {}
        for u in d["units"]:
            if u["type"] not in ("UNIT_BUILDER", "UNIT_SETTLER", "UNIT_TRADER"):
                units[u["type"]] = units.get(u["type"], 0) + 1
        st = State(turn=turn, acc=acc, cities=cities, player=player,
                   builders=[{"charges": u.get("charges") or 3, "at": (u["x"], u["y"]), "ready": turn}
                             for u in d["units"] if u["type"] == "UNIT_BUILDER"],
                   units=units,
                   border={c["name"]: [0.0, len(c["owned"]) - 7] for c in cities},
                   focus={c["name"]: "balanced" for c in cities},
                   gov_plaza=any(x["type"] == "DISTRICT_GOVERNMENT" for c in cities for x in c["districts"]))
        st.known_govs = set(self.available_governments(st))
        return st

    # ------------------------------------------------------------------ governors
    def governor_titles(self, st):
        """Titles earned: the Delta of every EFFECT_ADJUST_PLAYER_GOVERNOR_POINTS acting on the player."""
        from .modifiers import engine_for
        eng = engine_for(self.r, self.player_env(st), st.cities)
        return int(sum(float(a.get("Delta") or 0) for a, m, s_ in eng.player_effects("EFFECT_ADJUST_PLAYER_GOVERNOR_POINTS")))

    def establish_turns(self, gov):
        """Turns to establish, from the base promotion's text ("Establishes in N turns"; 5 when not stated)."""
        import re
        import sqlite3
        cache = self.__dict__.setdefault("_establish", {})
        if gov not in cache:
            base = self.r.q("select p.GovernorPromotionType, p.Description from GovernorPromotionSets s join GovernorPromotions p "
                            "on p.GovernorPromotionType=s.GovernorPromotion where s.GovernorType=? and p.Level=0", gov)
            n = 5
            # the game's English text (python_legacy/scripts/build_text_db.py), at the repository root's data/rules
            text = Path(__file__).resolve().parents[2] / "data/rules/text_en.sqlite"
            if not text.exists():
                raise FileNotFoundError(f"{text}: make it with python_legacy/scripts/build_text_db.py")
            db = sqlite3.connect(f"file:{text}?mode=ro", uri=True)
            for _, desc in base:
                row = db.execute("select Text from Text where Tag=?", (desc,)).fetchone()
                m = re.search(r"[Ee]stablishes in (\d+) turns", row[0] if row else "")
                if m:
                    n = int(m.group(1))
            cache[gov] = n
        return cache[gov]

    def governor_options(self, st):
        r = self.r
        if st.player.get("governors") is None:
            st.player["governors"] = []
        govs = st.player["governors"]
        have = {g["type"]: g for g in govs}
        opts = [("none",)]
        civ, leader = st.player["civilization"], st.player["leader"]
        used = {g.get("cityId") for g in govs}
        for gov, cs, trait in r.q("select GovernorType, AssignCityState, TraitType from Governors"):
            if trait and not r.civ_has_trait(civ, leader, trait):
                continue
            if gov in have:
                g = have[gov]
                for (pr,) in r.q("select GovernorPromotion from GovernorPromotionSets where GovernorType=?", gov):
                    if pr in g["promotions"]:
                        continue
                    pre = [x for (x,) in r.q("select PrereqGovernorPromotion from GovernorPromotionPrereqs where GovernorPromotionType=?", pr)]
                    if pre and not any(x in g["promotions"] for x in pre):
                        continue
                    opts.append(("promote", gov, pr))
            elif not cs:  # envoy governors (Amani) go to city-states, outside the model
                for c in st.cities:
                    if c["id"] not in used:
                        opts.append(("appoint", gov, c["name"]))
        return opts

    def governor_step(self, st, chooser):
        if st.player.get("governors") is None:
            st.player["governors"] = []
        govs = st.player["governors"]
        for g in govs:  # settle in after the establishing turns
            if not g.get("established") and st.turn >= g.get("ready", 0):
                g["established"] = True
        spent = sum(len(g["promotions"]) for g in govs)
        if self.governor_titles(st) <= spent:
            return
        pick = self.decide(st, "governor", "", self.governor_options(st), chooser)
        if not pick or pick[0] == "none":
            return
        _tr(st, "governor", st.turn, *pick)
        if pick[0] == "appoint":
            _, gov, cname = pick
            c = next(x for x in st.cities if x["name"] == cname)
            base = self.r.q("select p.GovernorPromotionType from GovernorPromotionSets s join GovernorPromotions p "
                            "on p.GovernorPromotionType=s.GovernorPromotion where s.GovernorType=? and p.Level=0", gov)
            govs.append({"type": gov, "promotions": [b for (b,) in base], "cityId": c["id"], "established": False,
                         "ready": st.turn + self.establish_turns(gov)})
        else:
            _, gov, pr = pick
            next(g for g in govs if g["type"] == gov)["promotions"].append(pr)

    # ------------------------------------------------------------------ options
    def available_techs(self, st):
        out = []
        for (t,) in self.r.q("select TechnologyType from Technologies"):
            if t in st.acc.techs:
                continue
            pre = [x for (x,) in self.r.q("select PrereqTech from TechnologyPrereqs where Technology=?", t)]
            era = self.r.q("select EraType from Technologies where TechnologyType=?", t)[0][0]
            if not pre and era != "ERA_ANCIENT":
                continue  # e.g. future-era techs listed without prerequisites
            if all(x in st.acc.techs for x in pre):
                out.append(t)
        return out

    def available_civics(self, st):
        out = []
        for (t,) in self.r.q("select CivicType from Civics"):
            if t in st.acc.civics:
                continue
            pre = [x for (x,) in self.r.q("select PrereqCivic from CivicPrereqs where Civic=?", t)]
            era = self.r.q("select EraType from Civics where CivicType=?", t)[0][0]
            if not pre and era != "ERA_ANCIENT":
                continue
            if all(x in st.acc.civics for x in pre):
                out.append(t)
        return out

    def available_governments(self, st):
        return [g for g, pc in self.r.q("select GovernmentType, PrereqCivic from Governments")
                if (pc is None or pc in st.acc.civics) and g != "GOVERNMENT_ANARCHY"]

    def slots(self, gov, st=None):
        """Policy slots of a government, with wonder / trait / dedication changes when the state is given."""
        if st is None:
            out = []
            for slot, n in self.r.q("select GovernmentSlotType, NumSlots from Government_SlotCounts where GovernmentType=?", gov):
                out += [slot] * n
            return out
        from .modifiers import engine_for
        from .production import policy_slots
        return policy_slots(self.r, engine_for(self.r, self.player_env(st), st.cities), gov)

    def policies_for_slot(self, st, slot):
        out = []
        eras = [e for (e,) in self.r.q("select EraType from Eras order by ChronologyIndex")]
        now = eras.index(self.sc.game_era(st.turn)) if self.sc.game_era(st.turn) in eras else 0
        for pol, pc, pt, ptype in self.r.q("select PolicyType, PrereqCivic, PrereqTech, GovernmentSlotType from Policies "
                                           "where PolicyType not like 'POLICY_GOV_%'"):
            if (pc and pc not in st.acc.civics) or (pt and pt not in st.acc.techs):
                continue
            # Policies_XP1: game-era range and dark / golden age. The age is not modelled yet: a normal age, so the
            # dark-age policies (Isolationism, ...) are never available
            xp1 = self.r.q("select MinimumGameEra, MaximumGameEra, RequiresDarkAge, RequiresGoldenAge from Policies_XP1 "
                           "where PolicyType=?", pol)
            if xp1:
                lo, hi, dark, golden = xp1[0]
                if dark or golden or (lo in eras and now < eras.index(lo)) or (hi in eras and now > eras.index(hi)):
                    continue
            if slot == "SLOT_WILDCARD" or ptype == slot:
                out.append(pol)
        return out

    def buildable(self, st, city):
        r = self.r
        techs, civics = st.acc.techs, st.acc.civics
        ca = st.acc.cities[city["name"]]
        done_d = {x["type"] for x in city["districts"] if x["complete"]}
        planned_d = {x["type"] for x in city["districts"]}
        specialty = sum(1 for x in city["districts"] if r.district_requires_population(x["type"]))
        allowed = 1 + (ca.population - 1) // int(r.param("DISTRICT_POPULATION_REQUIRED_PER"))
        out = []
        civ = st.player["civilization"]
        for u, pt, pc, trait, dom, can, cls, rel in r.q(
                "select UnitType, PrereqTech, PrereqCivic, TraitType, Domain, CanTrain, PromotionClass, "
                "ReligiousStrength > 0 or SpreadCharges > 0 or FoundReligion or EnabledByReligion from Units"):
            if not can or dom != "DOMAIN_LAND" or (pt and pt not in techs) or (pc and pc not in civics):
                continue
            if rel:
                continue  # religious units: founding a religion is outside the plan (great prophets)
            if trait and not r.civ_has_trait(civ, st.player["leader"], trait):
                continue
            if u in ("UNIT_SETTLER",) and ((ca.population < 2 and not self.settler_free_pop(st, city))
                                          or len(st.cities) + len(st.settlers) >= self.s.max_cities):
                continue
            if u == "UNIT_TRADER" and len(st.traders) >= self.trade_capacity(st):
                continue
            if u == "UNIT_SPY" and st.acc.units_trained.get(u, 0) >= self.spy_capacity(st):
                continue
            if r.unit_replaced(u, civ, st.player["leader"]):
                continue
            if "GREAT" in u or u.startswith("UNIT_ARCHAEOLOGIST") or cls is None and u not in ("UNIT_SETTLER", "UNIT_BUILDER", "UNIT_TRADER"):
                continue
            out.append(u)
        for b, pt, pc, pd, trait, wonder in r.q("select BuildingType, PrereqTech, PrereqCivic, PrereqDistrict, TraitType, IsWonder from Buildings "
                                                "where InternalOnly=0"):
            if b in city["buildings"] or (pt and pt not in techs) or (pc and pc not in civics):
                continue
            if pd and pd not in done_d and not r.district_replaces_done(pd, done_d):
                continue
            if trait and not r.civ_has_trait(civ, st.player["leader"], trait):
                continue
            if r.building_replaced(b, civ, st.player["leader"]) or not r.building_prereqs_met(b, city["buildings"]):
                continue
            if wonder:
                if b in self.sc.wonders_taken(st.turn) or any(b in cc["buildings"] for cc in st.cities) \
                        or any(cc["production"]["current"] == b for cc in st.cities if cc is not city) \
                        or not self.site_possible(st, city, b, wonder=True):
                    continue
            if b.startswith("BUILDING_GOV_") or b == "BUILDING_PALACE":
                continue
            out.append(b)
        from .modifiers import engine_for
        eng = engine_for(r, self.player_env(st), st.cities)
        # extra district capacity (Germany's free imperial city district, ...)
        allowed += sum(int(float(a["Amount"])) for a, m, s_ in eng.city_effects("EFFECT_ADJUST_CITY_EXTRA_DISTRICTS", city["name"]))
        # a district's tech prerequisite replaced (Qin: canal with Masonry)
        prereq = {a["DistrictType"]: a["TechType"] for a, m, s_ in eng.player_effects("EFFECT_ADJUST_DISTRICT_PREREQ")}
        for dist, pt, pc, trait, pop, one, maxp in r.q(
                "select DistrictType, PrereqTech, PrereqCivic, TraitType, RequiresPopulation, OnePerCity, MaxPerPlayer "
                "from Districts where InternalOnly=0 and CityCenter=0"):
            if dist in ("DISTRICT_SPACEPORT", "DISTRICT_CANAL"):
                continue  # no yields (canal: connects water for units)
            pt = prereq.get(dist, pt)
            if (pt and pt not in techs) or (pc and pc not in civics) or (pop and specialty >= allowed):
                continue
            if (one or pop) and dist in planned_d:
                continue
            if maxp and maxp > 0 and sum(1 for cc in st.cities for x in cc["districts"] if x["type"] == dist) >= maxp:
                continue
            if trait and not r.civ_has_trait(civ, st.player["leader"], trait):
                continue
            if r.district_replaced(dist, civ, st.player["leader"]):
                continue
            if dist == "DISTRICT_GOVERNMENT" and any(x["type"] == "DISTRICT_GOVERNMENT" for cc in st.cities for x in cc["districts"]):
                continue  # one per civilization, counting one under construction
            if not self.site_possible(st, city, dist):
                continue
            out.append(dist)
        # projects (repeatable): district projects, Bread and Circuses, Lijia / Court Festival when a trait unlocks them
        unlocked = {a["ProjectType"] for a, m, s_ in eng.player_effects("EFFECT_ADD_PLAYER_PROJECT_AVAILABILITY")}
        for proj, pt, pc, pd, maxi, space, wmd, from_eff in r.q(
                "select ProjectType, PrereqTech, PrereqCivic, PrereqDistrict, MaxPlayerInstances, SpaceRace, WMD, "
                "UnlocksFromEffect from Projects"):
            if space or wmd or maxi or (pt and pt not in techs) or (pc and pc not in civics):
                continue
            if from_eff and proj not in unlocked:
                continue
            if proj in ("PROJECT_REPAIR_OUTER_DEFENSES", "PROJECT_SEND_AID", "PROJECT_TRAIN_ATHLETES"):
                continue  # military / World Congress
            if pd and pd not in done_d and not r.district_replaces_done(pd, done_d):
                continue
            out.append(proj)
        return out

    def settler_free_pop(self, st, city):
        from .modifiers import engine_for
        return any(a.get("Enabled") in ("0", "false") for a, m, s_ in engine_for(self.r, self.player_env(st), st.cities)
                   .city_effects("EFFECT_ADJUST_CITY_SETTLER_CONSUME_POP", city["name"]))

    def route_endpoint(self, st, city):
        """One of our cities in the route endpoint format."""
        return {"name": city["name"], "id": city["id"], "playerId": st.player.get("playerId"),
                "civilization": st.player["civilization"], "leader": st.player["leader"],
                "x": city["center"]["x"], "y": city["center"]["y"],
                "districts": [x["type"] for x in city["districts"] if x["complete"]],
                "techCount": len(st.acc.techs), "civicCount": len(st.acc.civics),
                "plots": [(p["terrain"], p["resource"], p["improvement"]) for p in city["owned"]]}

    def route_options(self, st, origin_name):
        """Destinations a trader from `origin_name` can reach (TRADE_ROUTE_BASE_RANGE by distance; land paths and
        sea range are not checked): our other cities and the world's cities, best estimated yield first."""
        from .modifiers import engine_for
        from .yields import route_yields
        r = self.r
        origin = next(c for c in st.cities if c["name"] == origin_name)
        o = (origin["center"]["x"], origin["center"]["y"])
        rng = int(float(r.param("TRADE_ROUTE_BASE_RANGE")))
        eng = engine_for(r, self.player_env(st), st.cities)
        dests = [self.route_endpoint(st, c) for c in st.cities if c["name"] != origin_name]
        dests += self.sc.world_cities(st.turn)
        out = []
        for d in dests:
            dist = self.w.map.distance(o, (d["x"], d["y"]))
            if dist > rng:
                continue
            route = {"origin": self.route_endpoint(st, origin), "destination": d}
            out.append((sum(route_yields(r, eng, route, origin).values()), d["name"], d))
        out.sort(key=lambda t: -t[0])
        return out

    def start_route(self, st, origin_name, chooser):
        """A trader picks a destination; the route runs for TRADE_ROUTE_TURN_DURATION_BASE turns or one round trip
        if longer (2 x distance + 4 turns; fitted on one Maori route: 12 tiles, 28 turns)."""
        # traders beyond the route capacity (two cities finishing one each) wait without a route until one is free
        if sum(1 for t in st.traders if t.get("dest")) >= self.trade_capacity(st):
            return {"origin": origin_name, "dest": None, "info": None, "until": None}
        opts = self.route_options(st, origin_name)
        names = [n for _, n, _ in opts]
        dest = self.decide(st, "trade", "", names, chooser) if names else None
        t = {"origin": origin_name, "dest": dest, "info": None, "until": None}
        if dest:
            d = next(x for _, n, x in opts if n == dest)
            origin = next(c for c in st.cities if c["name"] == origin_name)
            dist = self.w.map.distance((origin["center"]["x"], origin["center"]["y"]), (d["x"], d["y"]))
            t["info"] = d if d["playerId"] != st.player.get("playerId") else None
            t["until"] = st.turn + max(int(float(self.r.param("TRADE_ROUTE_TURN_DURATION_BASE"))), 2 * dist + 4)
        _tr(st, "trade", st.turn, origin_name, dest, t["until"])
        return t

    def routes_now(self, st):
        """Active routes as route dicts (foreign endpoints refreshed from the recorded game)."""
        out = []
        world = {c["name"]: c for c in self.sc.world_cities(st.turn)}
        mine = {c["name"]: c for c in st.cities}
        for t in st.traders:
            if not t.get("dest") or t["origin"] not in mine:
                continue
            if t.get("info") is None:
                if t["dest"] not in mine:
                    continue
                dest = self.route_endpoint(st, mine[t["dest"]])
            else:
                dest = world.get(t["dest"], t["info"])
            out.append({"origin": self.route_endpoint(st, mine[t["origin"]]), "destination": dest})
        return out

    def spy_capacity(self, st):
        """Spies allowed: EFFECT_GRANT_SPY of the civics (Diplomatic Service, ...); none before."""
        from .modifiers import engine_for
        eng = engine_for(self.r, self.player_env(st), st.cities)
        return sum(int(float(a.get("Amount", 1))) for a, m, s_ in eng.player_effects("EFFECT_GRANT_SPY"))

    def trade_capacity(self, st):
        """Trade routes allowed: from the modifiers (Foreign Trade, BBG market / lighthouse, wonders, traits)."""
        from .modifiers import engine_for
        from .production import trade_route_capacity
        return trade_route_capacity(engine_for(self.r, self.player_env(st), st.cities))

    # ------------------------------------------------------------------ districts / sites
    def neighbours_with_districts(self, st, xy):
        at = {(x["x"], x["y"]): x["type"] for c in st.cities for x in c["districts"]}
        return [dict(self.w.map.plots[n], districtType=at.get(n)) for n in self.w.map.neighbours(*xy)]

    def near_fn(self, st):
        at = {(x["x"], x["y"]): x["type"] for c in st.cities for x in c["districts"]}
        imp = {(p["x"], p["y"]): p.get("improvement") for c in st.cities for p in c["owned"]}
        return lambda xy: [dict(self.w.map.plots[n], districtType=at.get(n), improvement=imp.get(n, self.w.map.plots[n]["improvement"]))
                           for n in self.w.map.neighbours(*xy)]

    def district_site(self, st, city, dist):
        from .modifiers import engine_for
        from .placement import district_ok
        center = (city["center"]["x"], city["center"]["y"])
        taken = {(x["x"], x["y"]) for c in st.cities for x in c["districts"]} | {center}
        eng = engine_for(self.r, self.player_env(st), st.cities)
        near = self.near_fn(st)
        best = None
        for p in city["owned"]:
            xy = (p["x"], p["y"])
            if self.w.map.distance(xy, center) > 3 or not district_ok(self.r, eng, dist, p, near, city, taken, self.w.map.is_lake):
                continue
            d = {"type": dist, "x": xy[0], "y": xy[1], "complete": True, "plot": p, "neighbours": self.neighbours_with_districts(st, xy)}
            adj = sum(district_adjacency(self.r, d, st.acc.techs, st.acc.civics, None,
                                         modifier_adjacency_rows(eng, city["name"], dist),
                                         excluded_adjacencies(self.r, st.player)).values())
            if best is None or adj > best[0]:
                best = (adj, xy, d)
        return best

    def site_possible(self, st, city, item, wonder=False):
        """Whether some plot of the city can take the district / wonder (the first valid plot settles it; the best
        plot is worked out only when the item is chosen)."""
        from .modifiers import engine_for
        from .placement import district_ok, wonder_ok
        center = (city["center"]["x"], city["center"]["y"])
        taken = {(x["x"], x["y"]) for c in st.cities for x in c["districts"]} | {center}
        eng = engine_for(self.r, self.player_env(st), st.cities)
        near = self.near_fn(st)
        ok = wonder_ok if wonder else district_ok
        return any(self.w.map.distance((p["x"], p["y"]), center) <= 3 and
                   ok(self.r, eng, item, p, near, city, taken, self.w.map.is_lake) for p in city["owned"])

    def wonder_site(self, st, city, wonder):
        """Where to put a wonder: a valid plot of the city, giving up the least tile value."""
        from .modifiers import engine_for
        from .placement import wonder_ok
        center = (city["center"]["x"], city["center"]["y"])
        taken = {(x["x"], x["y"]) for c in st.cities for x in c["districts"]} | {center}
        eng = engine_for(self.r, self.player_env(st), st.cities)
        near = self.near_fn(st)
        best = None
        for p in city["owned"]:
            xy = (p["x"], p["y"])
            if self.w.map.distance(xy, center) > 3 or not wonder_ok(self.r, eng, wonder, p, near, city, taken, self.w.map.is_lake):
                continue
            v = self.w_value(p, st.acc.techs, st.acc.civics, FOCUS["balanced"])
            if best is None or v < best[0]:
                best = (v, xy, p)
        return best

    def settle_sites(self, st, origin):
        o = (origin["center"]["x"], origin["center"]["y"])
        taken = {s["target"] for s in st.settlers if s.get("target")}
        sc = []
        for xy in self.w.map.within(o, self.s.settle_radius):
            if xy in taken:
                continue
            v = self.w.site_score(xy, st.cities, st.player | {"techs": st.acc.techs, "civics": st.acc.civics})
            if v > 0:
                sc.append((v - 0.7 * self.w.map.distance(o, xy), xy))
        sc.sort(reverse=True)
        # keep candidates apart from each other so options differ
        out = []
        for v, xy in sc:
            if all(self.w.map.distance(xy, y) >= 3 for y in out):
                out.append(xy)
            if len(out) >= self.s.settle_candidates:
                break
        for xy in self.s.extra_sites or ():
            xy = tuple(xy)
            if xy not in out and xy not in taken and self.w.site_score(
                    xy, st.cities, st.player | {"techs": st.acc.techs, "civics": st.acc.civics}) > 0:
                out.append(xy)
        return out

    # ------------------------------------------------------------------ builders
    def builder_options(self, st, player, at):
        """The builder's candidate actions, best first by their return over the rest of the horizon (Settings.horizon,
        the last turn counted): an improvement (or planted feature) gives its gain in weighted full yields (the
        city's focus) every turn from the builder's arrival (2 tiles a turn from `at`), counted on a worked tile, or
        beyond the weakest worked tile on one not worked; a harvest or a rush gives its lump once (x 0.7, less with
        under 10 turns left). The best builder_candidates, the recorded game's actions (extra_builder), then wait."""
        from .modifiers import engine_for
        from .placement import improvement_options
        from .yields import appeal_modifiers, city_plot_modifiers, plots_by_xy_of, tile_yields
        techs, civics = st.acc.techs, st.acc.civics
        eng = engine_for(self.r, player, st.cities)
        near = self.near_fn(st)
        pxy = plots_by_xy_of(st.cities)
        T, end = st.turn, self.s.horizon
        left = lambda xy: max(0, end - (T + (self.w.map.distance(at, xy) + 1) // 2))
        lump = lambda v, n: 0.7 * v * min(1.0, n / 10)
        opts = []
        for c in st.cities:
            worked = {(w["x"], w["y"]) for w in c["worked"]}
            center = (c["center"]["x"], c["center"]["y"])
            dtiles = {(x["x"], x["y"]) for x in c["districts"]}
            weights = FOCUS[st.focus.get(c["name"], "balanced")]
            mods = city_plot_modifiers(self.r, player, c, st.cities)
            amods = appeal_modifiers(eng, c["name"])
            wv = lambda p: sum(weights.get(k, 0) * v for k, v in tile_yields(
                self.r, p, techs, civics, plot_modifiers=mods, plots_by_xy=pxy, city=c, appeal_mods=amods).items())
            wk = [wv(p) for p in c["owned"] if (p["x"], p["y"]) in worked and (p["x"], p["y"]) != center]
            vmin = min(wk) if wk else 0.0
            gain = lambda xy, before, after: (after - before) if xy in worked else max(0.0, after - vmin)
            for p in c["owned"]:
                xy = (p["x"], p["y"])
                if xy == center or xy in dtiles or (not self.w.map.is_land(xy) and not p["resource"]):
                    continue
                n = left(xy)
                if not p["improvement"]:
                    base = wv(p)
                    for imp in improvement_options(self.r, eng, p, near, techs, civics, c, self.w.map.is_lake):
                        opts.append((gain(xy, base, wv(dict(p, improvement=imp))) * n, ("improve", c["name"], xy, imp)))
                # Planting woods (Conservation; Vietnam and BBG Teddy Roosevelt with Medieval Faires)
                if not p["feature"] and not p["improvement"] and not p["resource"]:
                    for f in self.plantable(eng, p, civics):
                        opts.append((gain(xy, wv(p), wv(dict(p, feature=f))) * n, ("plant", c["name"], xy, f)))
                # Harvests: features (chop) and bonus resources, for a burst in the owning city.
                for kind, yields in self.harvest_yields(p, techs, st, c):
                    opts.append((lump(sum(yields.values()), n), ("harvest", c["name"], xy, kind)))
        # builder charges spent on the city's district / wonder / project (Aztecs 30% of a district, Qin 15% of an
        # ancient or classical wonder)
        for c in st.cities:
            item = c["production"]["current"]
            pct = self.rush_percent(eng, item)
            if pct:
                center = (c["center"]["x"], c["center"]["y"])
                cost = item_cost(self.r, dict(player, unitsTrained=st.acc.units_trained), item, self.speed) or 0
                opts.append((lump(cost * pct / 100, left(center)), ("rush", c["name"], center, item)))
        opts.sort(key=lambda t: -t[0])
        keep = opts if self.s.builder_candidates is None else opts[:self.s.builder_candidates]
        out = [o for _, o in keep]
        extra = set(self.s.extra_builder or ())
        out += [o for _, o in opts if o in extra and o not in out]
        return out + [("wait",)]

    def idle_mark(self, st):
        """What a waiting builder waits for: a change of techs, civics, cities, tiles (and their improvements, features
        and resources), population or items."""
        owned = [p for c in st.cities for p in c["owned"]]
        return (len(st.acc.techs), len(st.acc.civics), len(st.cities), len(owned),
                sum(1 for p in owned if p["improvement"]), sum(1 for p in owned if p["feature"]),
                sum(1 for p in owned if p["resource"]),
                sum(st.acc.cities[c["name"]].population for c in st.cities),
                tuple(c["production"]["current"] for c in st.cities))

    def w_value(self, p, techs, civics, weights):
        from .yields import tile_yields
        return sum(weights.get(k, 0) * v for k, v in tile_yields(self.r, p, techs, civics).items())

    def plantable(self, eng, p, civics):
        """Features a builder can plant on plot p: Features.AddCivic, or an earlier civic from
        EFFECT_ADJUST_FEATURE_PREREQ; the terrain must suit the feature."""
        out = []
        early = {a["FeatureType"]: a["CivicType"] for a, m, s_ in eng.player_effects("EFFECT_ADJUST_FEATURE_PREREQ")}
        for f, civic in self.r.q("select FeatureType, AddCivic from Features where AddCivic is not null"):
            if civic in civics or early.get(f) in civics:
                if self.r.q("select 1 from Feature_ValidTerrains where FeatureType=? and TerrainType=?", f, p["terrain"]):
                    out.append(f)
        return out

    def rush_percent(self, eng, item):
        if not item:
            return 0.0
        kind = item_kind(item)
        if kind == "DISTRICT":
            eff = "EFFECT_ADJUST_PLAYER_UNIT_DISTRICT_PERCENT"
        elif kind == "BUILDING" and self.r.is_wonder(item):
            eff = "EFFECT_ADJUST_PLAYER_UNIT_WONDER_PERCENT"
        elif kind == "PROJECT":
            eff = "EFFECT_ADJUST_PLAYER_UNIT_PROJECT_PERCENT"
        else:
            return 0.0
        return sum(float(a["Amount"]) for a, m, s_ in eng.player_effects(eff))

    def harvest_yields(self, p, techs, st=None, city=None):
        """[(kind, yields)] a builder could harvest on plot p: the feature (chop) or a bonus resource (not for the
        Maori), with the game-progress formula (production.harvest_yield)."""
        from .modifiers import engine_for
        from .production import harvest_yield
        out = []
        eng = engine_for(self.r, self.player_env(st), st.cities) if st is not None else None
        civics = st.acc.civics if st is not None else ()
        if p["feature"]:
            tech = self.r.q("select RemoveTech from Features where FeatureType=?", p["feature"])
            rows = self.r.q("select YieldType, Yield from Feature_Removes where FeatureType=?", p["feature"])
            if rows and tech and (tech[0][0] is None or tech[0][0] in techs) and not p["improvement"]:
                out.append(("feature", harvest_yield(self.r, eng, "feature", p["feature"], techs, civics, city, self.speed)))
        no_res = eng is not None and eng.player_effects("EFFECT_ADJUST_PLAYER_PREVENT_HARVEST_RESOURCE")
        if p["resource"] and not no_res:
            rows = self.r.q("select YieldType, Amount, PrereqTech from Resource_Harvests where ResourceType=?", p["resource"])
            if rows and all(pt is None or pt in techs for _, _, pt in rows) and not p["improvement"]:
                out.append(("resource", harvest_yield(self.r, eng, "resource", p["resource"], techs, civics, city, self.speed)))
        return out

    # ------------------------------------------------------------------ base policies
    def base(self, st, key, options):
        kind = key[0]
        if not options:
            return None
        if kind in ("research", "civic"):
            prog = st.acc.tech_progress if kind == "research" else st.acc.civic_progress
            era = self.sc.game_era(st.turn)
            return min(options, key=lambda t: research_cost(self.r, t, self.speed, era) - prog.get(t, 0))
        if kind == "gov":
            return max(options, key=lambda g: (len(self.slots(g, st)), g == st.player["government"]))
        if kind == "policy":
            return options[0]
        if kind == "pantheon":
            return options[0]
        if kind in ("settle", "trade", "builder", "focus"):
            return options[0]
        if kind == "buy":
            return ("none",)
        if kind == "governor":
            # economy governors first, the capital first; otherwise promote
            pref = ["GOVERNOR_THE_RESOURCE_MANAGER", "GOVERNOR_THE_BUILDER", "GOVERNOR_THE_EDUCATOR",
                    "GOVERNOR_THE_MERCHANT", "GOVERNOR_THE_CARDINAL"]
            capital = st.cities[0]["name"] if st.cities else None
            rank = lambda o: (pref.index(o[1]) if o[1] in pref else len(pref), 0 if o[0] == "promote" else 1,
                              0 if len(o) > 2 and o[2] == capital else 1)
            real = [o for o in options if o[0] != "none"]
            return min(real, key=rank) if real else options[0]
        if kind == "prod":
            order = ["UNIT_SETTLER", "UNIT_BUILDER", "BUILDING_MONUMENT", "DISTRICT_GOVERNMENT", "DISTRICT_COMMERCIAL_HUB",
                     "DISTRICT_CAMPUS", "BUILDING_GRANARY", "BUILDING_LIBRARY", "BUILDING_MARKET", "UNIT_TRADER",
                     "BUILDING_WATER_MILL"]
            if not st.builders and "UNIT_BUILDER" in options:
                return "UNIT_BUILDER"
            for it in order:
                if it in options:
                    return it
            return options[0]
        return options[0]

    # ------------------------------------------------------------------ turn
    def decide(self, st, kind, entity, options, chooser):
        """Ask for a decision. Keys are (kind, entity, n): the n-th decision of this kind for this entity,
        so a plan stays attached to the same decision when earlier changes shift turns."""
        n = st.counters.get((kind, entity), 0)
        st.counters[(kind, entity)] = n + 1
        key = (kind, entity, n)
        capital = st.cities[0]["name"] if st.cities else None
        forced = self.s.fixed.get(key) or (self.s.fixed.get((kind, "CAPITAL", n)) if entity == capital else None)
        if forced is not None and forced in options:
            options = [forced]  # a fixed choice: the search sees a single option
        choice = chooser(st, key, options) if options else None
        st.log.append((key, st.turn, choice))
        return choice

    def road_type(self, turn):
        """The road of the era (roads take the era's road whatever the terrain): the best one unlocked by now."""
        eras = [e for (e,) in self.r.q("select EraType from Eras order by ChronologyIndex")]
        now = eras.index(self.sc.game_era(turn)) if self.sc.game_era(turn) in eras else 0
        best = "ROUTE_ANCIENT_ROAD"
        for rt, era in self.r.q("select RouteType, PrereqEra from Routes where RouteType<>'ROUTE_RAILROAD' order by PlacementValue"):
            if era is None or (era in eras and eras.index(era) <= now):
                best = rt
        return best

    def travel(self, st, a, b, unit):
        """Turns until `unit` (full points now) can act on b, by the movement rules (world.travel_turns)."""
        from .world import travel_turns
        moves = self.r.q("select BaseMoves from Units where UnitType=?", unit)[0][0]
        cities = {(c["center"]["x"], c["center"]["y"]) for c in st.cities}
        cities |= {(c["x"], c["y"]) for c in self.sc.world_cities(st.turn) or () if "x" in c}
        return travel_turns(self.r, self.w.map, tuple(a), tuple(b), moves, road_type=self.road_type(st.turn), cities=cities)

    GP_CLASSES = ("GREAT_PERSON_CLASS_ENGINEER", "GREAT_PERSON_CLASS_MERCHANT", "GREAT_PERSON_CLASS_SCIENTIST",
                  "GREAT_PERSON_CLASS_WRITER")  # the economy's; prophets (religion), generals and admirals are left out

    def gp_cost(self, st, cls, ind):
        """Recruitment cost: the individual's era base cost x game speed; the first of a new era in its class while
        the world era is behind costs 30% more, cut to a multiple of 5 (CivFanatics "the mechanism of great people";
        checked on the probes' point drops: 30/60/120/210, 155 = 120 x 1.3, 75 = 60 x 1.3)."""
        r = self.r
        era = r.q("select EraType from GreatPersonIndividuals where GreatPersonIndividualType=?", ind)[0][0]
        base = r.q("select GreatPersonBaseCost from Eras where EraType=?", era)[0][0] * r.game_speed_cost_multiplier(self.speed)
        order = {e: i for i, (e,) in enumerate(r.q("select EraType from Eras order by ChronologyIndex"))}
        same_era_before = [i for i, f, taken in self.sc.gp_timeline().get(cls, ())
                           if i != ind and (i in st.gp_ours or (taken is not None and taken <= st.turn))
                           and r.q("select EraType from GreatPersonIndividuals where GreatPersonIndividualType=?", i)[0][0] == era]
        if not same_era_before and order.get(self.sc.game_era(st.turn), 0) < order[era]:
            return math.floor(base * 1.3 / 5) * 5
        return base

    def gp_place(self, st, ind):
        """The city where a recruited great person can act now (the district it needs, a wonder being built for a
        wonder-production one), capital first; None to wait."""
        r = self.r
        need = r.q("select ActionRequiresCompletedDistrictType from GreatPersonIndividuals where GreatPersonIndividualType=?", ind)[0][0]
        targets = {t for (t,) in r.q("select AttachmentTargetType from GreatPersonIndividualActionModifiers where GreatPersonIndividualType=?", ind)}
        for c in st.cities:
            if need and not any(rules_district_is(r, d["type"], need) and d["complete"] for d in c["districts"]):
                continue
            if "TARGET_DISTRICT_WONDER_IN_TILE" in " ".join(targets):
                cur = c["production"]["current"]
                if not (cur and r.is_wonder(cur)):
                    continue
            return c["name"]
        return None

    def great_people(self, st, chooser):
        """Points banked each turn; a great person on offer is recruited when the points reach its cost (or the plan
        waits), then acts where it can: one-time grants now, lasting effects through the engine (greatPeopleUsed)."""
        from .greatpeople import great_person_points
        from .modifiers import engine_for
        r = self.r
        player = self.player_env(st)
        for cls, pts in great_person_points(r, engine_for(r, player, st.cities), st.cities).items():
            st.gp[cls] = st.gp.get(cls, 0.0) + pts
        for cls in self.GP_CLASSES:
            ind = self.sc.gp_offer(cls, st.turn, st.gp_ours)
            if not ind:
                continue
            cost = self.gp_cost(st, cls, ind)
            if st.gp.get(cls, 0.0) < cost:
                continue
            pick = self.decide(st, "gp", cls, [ind, "wait"], chooser)
            if pick == ind:
                st.gp[cls] -= cost
                st.gp_ours.append(ind)
                st.gp_pending.append({"individual": ind, "cls": cls, "turn": st.turn})
                _tr(st, "gp_recruit", st.turn, cls, ind, cost)
        for g in list(st.gp_pending):
            city = self.gp_place(st, g["individual"])
            if city is None:
                continue
            st.gp_pending.remove(g)
            n = max(1, r.q("select ActionCharges from GreatPersonIndividuals where GreatPersonIndividualType=?", g["individual"])[0][0] or 1)
            st.player.setdefault("greatPeopleUsed", []).append(dict(g, city=city))
            for _ in range(n):
                self.apply_grants(st, "GreatPersonIndividualActionModifiers", "GreatPersonIndividualType", g["individual"],
                                  city, self.player_env(st), chooser)
            _tr(st, "gp_use", st.turn, g["cls"], g["individual"], city)

    def player_env(self, st):
        p = dict(st.player)
        p["techs"], p["civics"] = sorted(st.acc.techs), sorted(st.acc.civics)
        p["unitsTrained"] = st.acc.units_trained
        p["envoys"] = self.sc.envoys(st.turn)
        p["prophetOnOffer"], p["prophetRecruitedTurn"] = self.sc.prophet_on_offer(st.turn), None
        # the turn's research completes against the next turn's era costs (the era switches first)
        p["gameEra"] = self.sc.game_era(st.turn + 1)
        return p

    def advance(self, st, chooser):
        r, w, T = self.r, self.w, st.turn
        # Research / civics.
        if st.research is None or st.research in st.acc.techs:
            st.research = self.decide(st, "research", "", self.available_techs(st), chooser)
        if st.civic is None or st.civic in st.acc.civics:
            st.civic = self.decide(st, "civic", "", self.available_civics(st), chooser)
        # Government.
        govs = self.available_governments(st)
        if set(govs) - st.known_govs:
            st.known_govs = set(govs)
            g = self.decide(st, "gov", "", govs, chooser)
            if g and g != st.player["government"]:
                st.player["government"] = g
                st.policy_dirty = True
        # Policies.
        if st.policy_dirty:
            chosen = []
            for i, slot in enumerate(self.slots(st.player["government"], st)):
                opts = [p for p in self.policies_for_slot(st, slot) if p not in chosen] + ["NONE"]
                pick = self.decide(st, "policy", i, opts, chooser)
                if pick and pick != "NONE":
                    chosen.append(pick)
            st.player["policies"] = chosen
            st.policy_dirty = False
        # Governors: a title (from civics, the government plaza's buildings, wonders, traits) appoints a new governor
        # or promotes one.
        self.governor_step(st, chooser)
        player = self.player_env(st)
        # Pantheon.
        cost = r.param("RELIGION_PANTHEON_MIN_FAITH") * r.game_speed_cost_multiplier(self.speed)
        if not st.player.get("pantheon") and st.acc.faith >= cost:
            opts = [b for (b,) in r.q("select BeliefType from Beliefs where BeliefClassType='BELIEF_CLASS_PANTHEON'")]
            pick = self.decide(st, "pantheon", "", opts, chooser)
            if pick:
                st.player["pantheon"] = pick
                _tr(st, "pantheon", T, pick)
                st.acc.faith -= cost
                player = self.player_env(st)
        # Settlers arriving.
        for s in [s for s in st.settlers if s["arrive"] <= T]:
            st.settlers.remove(s)
            if s["target"] is None or w.site_score(s["target"], st.cities, player) <= 0 and not self.s.sites:
                continue
            name = s.get("name") or NEW_CITY_NAMES[len(st.cities) - 1]
            c = w.found_city(name, s["target"], st.cities, player)
            st.cities.append(c)
            _tr(st, "found", T, name, tuple(s["target"]), s.get("id"))
            st.acc.cities[name] = CityAcc(population=1)
            st.border[name] = [0.0, 0]
            st.focus[name] = "balanced"
        # Rome (BBG): free cheapest building in every city after Early Empire.
        if "CIVIC_EARLY_EMPIRE" in st.acc.civics and st.player["civilization"] == "CIVILIZATION_ROME":
            for c in st.cities:
                if c["name"] not in st.free_building_given:
                    for b in ("BUILDING_MONUMENT", "BUILDING_GRANARY"):
                        if b not in c["buildings"]:
                            c["buildings"].append(b)
                            break
                    st.free_building_given.add(c["name"])
        # Citizens (focus decided when population changes).
        for c in st.cities:
            pop = st.acc.cities[c["name"]].population
            if c.get("_pop") != pop:
                c["_pop"] = pop
                st.focus[c["name"]] = self.decide(st, "focus", c["name"], list(FOCUS), chooser) or "balanced"
            c["population"] = pop
            w.assign_citizens(c, player, weights=FOCUS[st.focus[c["name"]]], cities=st.cities)
        # Builders. A builder works a tile within 1 step this turn; farther tiles cost a turn per 2 steps of travel
        # first (it walks there and works on arrival).
        for b in st.builders:
            if "id" not in b:
                b["id"] = st.counters["_builder"] = st.counters.get("_builder", 0) + 1
                _tr(st, "builder_new", T, b["id"], tuple(b["at"]))
            if b["ready"] > T:
                continue
            if b.get("pending"):
                act = b.pop("pending")
            else:
                # a builder that chose to wait decides again once something it could use changes
                if b.get("idle") is not None and b["idle"] == self.idle_mark(st):
                    continue
                b.pop("idle", None)
                act = self.decide(st, "builder", "", self.builder_options(st, player, tuple(b["at"])), chooser)
                if not act or act[0] == "wait":
                    b["idle"] = self.idle_mark(st)
                    continue
                _tr(st, "builder_go", T, b["id"], act)
                k = self.travel(st, b["at"], act[2], "UNIT_BUILDER")
                if k is None:
                    continue
                if k > 0:
                    b["ready"] = T + k
                    b["at"] = act[2]
                    b["pending"] = act
                    continue
            b["at"] = act[2]
            city = next((c for c in st.cities if c["name"] == act[1]), None)
            plot = next((p for p in city["owned"] if (p["x"], p["y"]) == act[2]), None) if city else None
            if act[0] == "rush":
                if city is None or city["production"]["current"] != act[3]:
                    continue
                from .modifiers import engine_for as _ef
                cost = item_cost(r, dict(player, unitsTrained=st.acc.units_trained), act[3], self.speed)
                if cost:
                    st.acc.cities[city["name"]].overflow += cost * self.rush_percent(_ef(r, player, st.cities), act[3]) / 100
                _tr(st, "builder_do", T, b["id"], act)
                b["charges"] -= 1
                continue
            if plot is None or (act[0] in ("improve", "plant") and (plot["improvement"] or (act[0] == "plant" and plot["feature"]))):
                continue
            if act[0] == "improve":
                plot["improvement"] = act[3]
            elif act[0] == "plant":
                plot["feature"] = act[3]
            else:
                ys = dict(self.harvest_yields(plot, st.acc.techs, st, city)).get(act[3])
                if ys is None:  # the tile changed while the builder was walking there
                    continue
                if act[3] == "feature":
                    plot["feature"] = None
                else:
                    plot["resource"] = None
                ca = st.acc.cities[city["name"]]
                ca.overflow += ys.get("YIELD_PRODUCTION", 0)
                ca.food_store += ys.get("YIELD_FOOD", 0)
                st.acc.gold += ys.get("YIELD_GOLD", 0)
            _tr(st, "builder_do", T, b["id"], act)
            b["charges"] -= 1
        st.builders = [b for b in st.builders if b["charges"] > 0]
        if st.builders:  # improvements change which tiles are best
            for c in st.cities:
                w.assign_citizens(c, player, weights=FOCUS[st.focus[c["name"]]], cities=st.cities)
        # A wonder another civilization finished first: the city gives it up (its production stays with the item).
        taken = self.sc.wonders_taken(T)
        for c in st.cities:
            cur = c["production"]["current"]
            if cur and cur in taken:
                c["production"]["current"] = None
                for x in list(c["districts"]):
                    if x["type"] == "DISTRICT_WONDER" and x["plot"].get("wonder") == cur and not x["complete"]:
                        c["districts"].remove(x)
                        x["plot"]["wonder"] = None
        # Production choices.
        for c in st.cities:
            if c["production"]["current"] is None:
                item = self.decide(st, "prod", c["name"], self.buildable(st, c), chooser)
                if item and item_kind(item) == "DISTRICT":
                    site = self.district_site(st, c, item)
                    d = dict(site[2], complete=False)
                    c["districts"].append(d)
                elif item and item_kind(item) == "BUILDING" and r.is_wonder(item):
                    _, xy, p = self.wonder_site(st, c, item)
                    p["wonder"] = item
                    c["districts"].append({"type": "DISTRICT_WONDER", "x": xy[0], "y": xy[1], "complete": False,
                                           "plot": p, "neighbours": self.neighbours_with_districts(st, xy)})
                c["production"]["current"] = item
                site = next(((x["x"], x["y"]) for x in c["districts"] if not x["complete"] and
                             (x["type"] == item or x["plot"].get("wonder") == item)), None) if item else None
                _tr(st, "prod", T, c["name"], item, site)
        # Purchase.
        buys = [("none",)]
        from .modifiers import engine_for
        from .production import faith_discount_percent, faith_price, purchase_discount_percent, purchase_yields
        eng = engine_for(r, player, st.cities)
        for c in st.cities:
            for item in self.buildable(st, c):
                if item_kind(item) == "DISTRICT":
                    continue  # bought districts need a site; placed with the production choice below
                currencies = purchase_yields(r, eng, c, item)
                if not currencies:
                    continue
                cost = item_cost(r, dict(player, unitsTrained=st.acc.units_trained), item, self.speed, exact=True)
                if cost is None:
                    continue
                # The price is for the whole item; production already put in stays with the item type.
                if "YIELD_GOLD" in currencies:
                    gold = gold_price(cost, purchase_discount_percent(r, eng, c, item))
                    if 0 < gold <= st.acc.gold:
                        buys.append(("item", c["name"], item, gold))
                if "YIELD_FAITH" in currencies:
                    faith = faith_price(cost, faith_discount_percent(r, eng, c, item))
                    if 0 < faith <= st.acc.faith:
                        buys.append(("item", c["name"], item, faith, "YIELD_FAITH"))
            # districts that can be bought (Reyna's Contractor, Moksha's Divine Architect): the one being built
            cur = c["production"]["current"]
            if cur and item_kind(cur) == "DISTRICT":
                for cy in purchase_yields(r, eng, c, cur):
                    cost = item_cost(r, dict(player, unitsTrained=st.acc.units_trained), cur, self.speed)
                    if cost is None:
                        continue
                    if cy == "YIELD_GOLD":
                        price = gold_price(cost, purchase_discount_percent(r, eng, c, cur))
                        if 0 < price <= st.acc.gold:
                            buys.append(("item", c["name"], cur, price))
                    else:
                        price = faith_price(cost, faith_discount_percent(r, eng, c, cur))
                        if 0 < price <= st.acc.faith:
                            buys.append(("item", c["name"], cur, price, "YIELD_FAITH"))
        from .production import tile_price
        eng = engine_for(r, player, st.cities)
        for c in st.cities:
            if (c["center"]["x"], c["center"]["y"]) in (self.s.recorded_tiles or {}):
                continue  # its tiles come from the recorded game
            xy = w.next_border_tile(c, st.cities, player)
            if xy is None:
                continue
            gold = tile_price(r, eng, c, w.map.plots[xy], self.speed, st.acc.techs, st.acc.civics)
            if gold <= st.acc.gold:
                buys.append(("tile", c["name"], gold, xy))
        buy = self.decide(st, "buy", "", buys, chooser) if len(buys) > 1 else ("none",)
        if buy and buy[0] != "none":
            _tr(st, "buy", T, *buy)
        if buy and buy[0] == "tile":
            _, name, gold, xy = buy
            c = next(x for x in st.cities if x["name"] == name)
            if w.grow_border(c, st.cities, player, xy=xy):
                st.acc.gold -= gold
                st.tiles_bought += 1
        if buy and buy[0] == "item":
            name, item, price = buy[1:4]
            if len(buy) > 4 and buy[4] == "YIELD_FAITH":
                st.acc.faith -= price
            else:
                st.acc.gold -= price
            st.bought.append((name, item))
            if item_kind(item) == "DISTRICT":
                # a bought district is finished now; the city picks a new item next turn
                c = next(x for x in st.cities if x["name"] == name)
                c["production"]["current"] = None
        # Output for the score, and border culture.
        env_cities = {c["name"]: c for c in st.cities}
        # routes that ran their course: the trader picks a new destination
        for i, t in enumerate(st.traders):
            if (t.get("until") is not None and T >= t["until"]) or not t.get("dest"):
                st.traders[i] = {"origin": t["origin"], "dest": None, "info": None, "until": None}  # free its slot
                st.traders[i] = self.start_route(st, t["origin"], chooser)
        routes = self.routes_now(st)
        scores = amenity_scores(r, [dict(c, population=st.acc.cities[c["name"]].population) for c in st.cities], player)
        out = {}
        snap = []
        for c in st.cities:
            y = city_turn_yields(r, c, player, st.acc.cities[c["name"]].population, routes, env_cities,
                                 happiness(r, scores[c["name"]])[0], scores[c["name"]])
            snap.append({"name": c["name"], "xy": (c["center"]["x"], c["center"]["y"]), "item": c["production"]["current"],
                         "pop": st.acc.cities[c["name"]].population, "food_store": st.acc.cities[c["name"]].food_store,
                         "worked": [(p["x"], p["y"], p.get("workers", 1)) for p in c["worked"]], "yields": dict(y)})
            for k, v in y.items():
                out[k] = out.get(k, 0) + v
            # border growth rate modifiers (Religious Settlements +50%, Reyna's Land Acquisition +20%)
            from .modifiers import engine_for as _ef
            bpct = sum(float(a.get("Amount") or 0) for a, m, src in
                       _ef(r, player, st.cities).city_effects("EFFECT_ADJUST_CITY_CULTURE_BORDER_EXPANSION", c["name"]))
            st.border[c["name"]][0] += y["YIELD_CULTURE"] * (1 + bpct / 100)
        _tr(st, "turn", T, {"research": st.research, "civic": st.civic, "government": st.player["government"],
                            "policies": list(st.player["policies"]), "gold": st.acc.gold, "faith": st.acc.faith,
                            "cities": snap})
        st.score += sum(self.s.score_weights.get(k, 0) * v for k, v in out.items())
        st.last_output = out
        # Seoul's suzerain bonus: a random eureka of each new era (the cheapest stands in)
        if self.sc.game_era(T + 1) != self.sc.game_era(T):
            from .modifiers import engine_for as _ef2
            for a, m, s_ in _ef2(r, player, st.cities).player_effects("EFFECT_GRANT_RANDOM_TECHNOLOGY_BOOST_ON_NEW_ERA"):
                for _ in range(int(float(a.get("Amount") or 1))):
                    self.boost_cheapest(st, self.sc.game_era(T + 1), civic=False)
        # The turn itself.
        explored_now = st.acc.units_trained.get("UNIT_SCOUT", 0) >= self.sc.scouts_by_turn.get(T, 0)
        boosts = self.endogenous_boosts(st) + (self.sc.exo_boosts.get(T, []) if explored_now else [])
        boosted = st.player.setdefault("_boosted", set())
        boosts = [b for b in dict.fromkeys(boosts) if b not in st.acc.techs and b not in st.acc.civics and b not in boosted]
        st.player["_boosted"] |= set(boosts)
        events = step(r, st.acc, {"cities": st.cities, "player": player, "units": st.units}, self.speed,
                      research=st.research, civic=st.civic, boosts=boosts, routes=routes)
        for name, item in st.bought:  # purchased items appear at the end of the turn
            st.acc.units_trained[item] = st.acc.units_trained.get(item, 0) + (1 if item_kind(item) == "UNIT" else 0)
            events.append(("completed_purchase", name, item))
        st.bought = []
        explored = st.acc.units_trained.get("UNIT_SCOUT", 0) >= self.sc.scouts_by_turn.get(T, 0)
        st.acc.faith += self.sc.faith_gift.get(T, 0) if explored else 0
        st.acc.gold += self.sc.gold_gift.get(T, 0) if explored else 0
        for u in (self.sc.unit_gifts.get(T, []) if explored else []):
            if u == "UNIT_BUILDER":
                cap = st.cities[0]["center"]
                st.builders.append({"charges": r.q("select BuildCharges from Units where UnitType='UNIT_BUILDER'")[0][0],
                                    "at": (cap["x"], cap["y"]), "ready": T})
            elif u not in ("UNIT_SETTLER", "UNIT_TRADER"):
                st.units[u] = st.units.get(u, 0) + 1
        for ev in events:
            if ev[0] == "researched" and ev[1].startswith("CIVIC_"):
                st.policy_dirty = True
            if ev[0] not in ("completed", "completed_purchase"):
                continue
            _, name, item = ev
            c = env_cities[name]
            if ev[0] == "completed":
                c["production"]["current"] = None
            if item == "UNIT_BUILDER":
                from .modifiers import engine_for
                from .production import builder_charges
                charges = builder_charges(r, engine_for(r, player, st.cities), "UNIT_BUILDER", name)
                st.builders.append({"charges": charges, "at": (c["center"]["x"], c["center"]["y"]), "ready": T + 1})
            elif item == "UNIT_SETTLER":
                origin = c
                if self.s.sites and len(st.cities) - 1 + len(st.settlers) < len(self.s.sites):
                    nm, xy = self.s.sites[len(st.cities) - 1 + len(st.settlers)]
                    target, label = tuple(xy), nm
                else:
                    target = self.decide(st, "settle", "", self.settle_sites(st, origin), chooser)
                    label = (self.s.site_names or {}).get(tuple(target)) if target else None
                    if label and any(c["name"] == label for c in st.cities):
                        label = None
                k = self.travel(st, (origin["center"]["x"], origin["center"]["y"]), target, "UNIT_SETTLER") if target else None
                if target and k is not None:
                    # built at the end of this turn; it moves from the next turn with full points
                    sid = st.counters["_settler"] = st.counters.get("_settler", 0) + 1
                    st.settlers.append({"from": name, "target": target, "name": label, "arrive": T + 1 + k, "id": sid})
                    _tr(st, "settler", T, sid, name, tuple(target), T + 1 + k)
            elif item == "UNIT_TRADER":
                st.traders.append(self.start_route(st, name, chooser))
            elif item_kind(item) == "UNIT":
                st.units[item] = st.units.get(item, 0) + 1
            elif item_kind(item) == "BUILDING":
                c["buildings"].append(item)
                for x in c["districts"]:
                    if x["type"] == "DISTRICT_WONDER" and x["plot"].get("wonder") == item:
                        x["complete"] = True
            elif item_kind(item) == "DISTRICT":
                for x in c["districts"]:
                    if x["type"] == item:
                        x["complete"] = True
                if item == "DISTRICT_GOVERNMENT":
                    st.gov_plaza = True
        # One-time grants of what was gained this turn (wonders' free units, population, techs, boosts, gold).
        for ev in events:
            if ev[0] in ("completed", "completed_purchase") and item_kind(ev[2]) in ("BUILDING", "DISTRICT"):
                table, col = ("BuildingModifiers", "BuildingType") if ev[2].startswith("BUILDING_") else ("DistrictModifiers", "DistrictType")
                self.apply_grants(st, table, col, ev[2], ev[1], player, chooser)
                self.on_completed(st, env_cities[ev[1]], ev[2], player)
            elif ev[0] == "researched":
                table, col = ("TechnologyModifiers", "TechnologyType") if ev[1].startswith("TECH_") else ("CivicModifiers", "CivicType")
                self.apply_grants(st, table, col, ev[1], None, player, chooser)
        # Great people.
        self.great_people(st, chooser)
        # Borders.
        owned_all = {(p["x"], p["y"]) for c in st.cities for p in c["owned"]}
        for c in st.cities:
            b = st.border[c["name"]]
            rt = (self.s.recorded_tiles or {}).get((c["center"]["x"], c["center"]["y"]))
            if rt is not None:
                while b[0] >= w.border_cost(b[1]):
                    b[0] -= w.border_cost(b[1])
                    b[1] += 1
                for t, xy in rt:
                    if t <= T and tuple(xy) not in owned_all:
                        w.grow_border(c, st.cities, player, xy=tuple(xy))
                        owned_all.add(tuple(xy))
                continue
            while b[0] >= w.border_cost(b[1]):
                b[0] -= w.border_cost(b[1])
                forced = (self.s.borders or {}).get(((c["center"]["x"], c["center"]["y"]), b[1]))
                b[1] += 1
                w.grow_border(c, st.cities, player, xy=tuple(forced) if forced else None)
        st.turn += 1
        return st

    GRANT_EFFECTS = ("EFFECT_GRANT_UNIT_IN_CITY", "EFFECT_ADJUST_CITY_POPULATION", "EFFECT_GRANT_PLAYER_SPECIFIC_TECHNOLOGY",
                     "EFFECT_GRANT_PLAYER_RANDOM_TECHNOLOGY", "EFFECT_GRANT_ALL_TECHNOLOGY_BOOST_BY_ERA",
                     "EFFECT_GRANT_RANDOM_TECHNOLOGY_BOOST_BY_ERA", "EFFECT_GRANT_RANDOM_CIVIC_BOOST_BY_ERA", "EFFECT_GRANT_YIELD",
                     "EFFECT_GRANT_CHEAPEST_BUILDING_IN_CITY", "EFFECT_GRANT_BUILDING_IN_CITY_IGNORE", "EFFECT_GRANT_PRODUCTION_IN_CITY",
                     "EFFECT_ADJUST_MULTIPLY_TREASURY", "EFFECT_GRANT_PLAYER_RANDOM_CIVIC",
                     "EFFECT_GRANT_PLAYER_SPECIFIC_TECH_BOOST_GREAT_PERSON")

    def apply_grants(self, st, table, col, key, city_name, player, chooser):
        """Apply the one-time grants of a source just gained. Random grants (a random tech / boost) are not
        reproducible; the cheapest eligible items stand in for them."""
        from .modifiers import engine_for
        from .production import builder_charges
        from .sim import boost_amount
        r = self.r
        owner = ("city", city_name) if city_name else ("player",)
        eng = engine_for(r, player, st.cities)
        era = self.sc.game_era(st.turn + 1)
        for eff, subj, a, mid in eng.grants_of(table, col, key, owner):
            if eff not in self.GRANT_EFFECTS:
                continue
            cname = subj[1] if subj and subj[0] in ("city", "district", "plot") else (city_name or st.cities[0]["name"])
            c = next((x for x in st.cities if x["name"] == cname), st.cities[0])
            n = int(float(a.get("Amount") or 1))
            if eff == "EFFECT_GRANT_UNIT_IN_CITY":
                u = a.get("UnitType")
                for _ in range(n):
                    if u == "UNIT_BUILDER":
                        st.builders.append({"charges": builder_charges(r, eng, u, cname),
                                            "at": (c["center"]["x"], c["center"]["y"]), "ready": st.turn + 1})
                    elif u == "UNIT_TRADER":
                        st.traders.append(self.start_route(st, cname, chooser))
                    elif u:
                        st.units[u] = st.units.get(u, 0) + 1
            elif eff == "EFFECT_ADJUST_CITY_POPULATION":
                st.acc.cities[cname].population += n
            elif eff == "EFFECT_GRANT_PLAYER_SPECIFIC_TECHNOLOGY":
                st.acc.techs.add(a["TechType"])
            elif eff == "EFFECT_GRANT_PLAYER_SPECIFIC_TECH_BOOST_GREAT_PERSON":
                # the tech's boost; if it was boosted already, the tech itself (Euclid, Zhang Heng, Bi Sheng)
                t = a.get("TechType")
                boosted = st.player.setdefault("_boosted", set())
                if t and t not in st.acc.techs:
                    if t in boosted:
                        st.acc.techs.add(t)
                    else:
                        st.acc.tech_progress[t] = st.acc.tech_progress.get(t, 0.0) + boost_amount(research_cost(r, t, self.speed, era))
                        boosted.add(t)
            elif eff in ("EFFECT_GRANT_PLAYER_RANDOM_TECHNOLOGY",):
                for t in sorted(self.available_techs(st), key=lambda t: research_cost(r, t, self.speed, era))[:n]:
                    st.acc.techs.add(t)
            elif eff == "EFFECT_GRANT_PLAYER_RANDOM_CIVIC":  # Bolshoi Theatre
                for t in sorted(self.available_civics(st), key=lambda t: research_cost(r, t, self.speed, era))[:n]:
                    st.acc.civics.add(t)
            elif eff == "EFFECT_ADJUST_MULTIPLY_TREASURY":  # Big Ben
                st.acc.gold *= 1 + float(a.get("Amount") or 0) / 100
            elif eff in ("EFFECT_GRANT_ALL_TECHNOLOGY_BOOST_BY_ERA", "EFFECT_GRANT_RANDOM_TECHNOLOGY_BOOST_BY_ERA",
                         "EFFECT_GRANT_RANDOM_CIVIC_BOOST_BY_ERA"):
                civic = "CIVIC" in eff
                tab, c2 = ("Civics", "CivicType") if civic else ("Technologies", "TechnologyType")
                lo, hi = (r.q("select ChronologyIndex from Eras where EraType=?", a.get(k))[0][0] for k in ("StartEraType", "EndEraType"))
                done = st.acc.civics if civic else st.acc.techs
                boosted = st.player.setdefault("_boosted", set())
                items = [t for (t,) in r.q(f"select {c2} from {tab} t join Eras e on e.EraType=t.EraType "
                                           f"where e.ChronologyIndex between ? and ?", lo, hi)
                         if t not in done and t not in boosted and r.has_boost(t)]
                if eff != "EFFECT_GRANT_ALL_TECHNOLOGY_BOOST_BY_ERA":
                    items = sorted(items, key=lambda t: research_cost(r, t, self.speed, era))[:n]
                prog = st.acc.civic_progress if civic else st.acc.tech_progress
                for t in items:
                    prog[t] = prog.get(t, 0.0) + boost_amount(research_cost(r, t, self.speed, era))
                    boosted.add(t)
            elif eff == "EFFECT_GRANT_YIELD":
                amt = float(a.get("Amount") or 0) * (r.game_speed_cost_multiplier(self.speed) if a.get("Scale") == "1" else 1)
                if a.get("YieldType") == "YIELD_GOLD":
                    st.acc.gold += amt
                elif a.get("YieldType") == "YIELD_FAITH":
                    st.acc.faith += amt
            elif eff in ("EFFECT_GRANT_BUILDING_IN_CITY_IGNORE",) and a.get("BuildingType") not in c["buildings"]:
                c["buildings"].append(a["BuildingType"])
            elif eff == "EFFECT_GRANT_CHEAPEST_BUILDING_IN_CITY":
                opts = [b for b in self.buildable(st, c) if b.startswith("BUILDING_") and not r.q(
                    "select IsWonder from Buildings where BuildingType=?", b)[0][0]]
                if opts:
                    c["buildings"].append(min(opts, key=lambda b: r.cost(b)))
            elif eff == "EFFECT_GRANT_PRODUCTION_IN_CITY":
                cur = c["production"]["current"]
                if cur:
                    ca = st.acc.cities[cname]
                    ca.progress[cur] = ca.progress.get(cur, 0.0) + float(a.get("Amount") or 0)

    def boost_cheapest(self, st, era_type, civic):
        """A random boost of an era, stood in for by the cheapest item of that era not yet boosted or known."""
        from .sim import boost_amount
        r = self.r
        tab, col = ("Civics", "CivicType") if civic else ("Technologies", "TechnologyType")
        done = st.acc.civics if civic else st.acc.techs
        boosted = st.player.setdefault("_boosted", set())
        era = self.sc.game_era(st.turn + 1)
        items = [t for (t,) in r.q(f"select {col} from {tab} where EraType=?", era_type)
                 if t not in done and t not in boosted and r.has_boost(t)]
        if not items:
            return
        t = min(items, key=lambda t: research_cost(r, t, self.speed, era))
        prog = st.acc.civic_progress if civic else st.acc.tech_progress
        prog[t] = prog.get(t, 0.0) + boost_amount(research_cost(r, t, self.speed, era))
        boosted.add(t)

    def on_completed(self, st, city, item, player):
        """Trait reactions to a finished district or wonder: Hammurabi's free first building in a new specialty
        district; Qin's random eureka and inspiration of the wonder's era (the cheapest stand in)."""
        from .modifiers import engine_for
        r = self.r
        eng = engine_for(r, player, st.cities)
        if item_kind(item) == "DISTRICT" and r.district_requires_population(item):
            free = eng.player_effects("EFFECT_ADJUST_PLAYER_FREE_BUILDING_WHEN_SPECIALTY_DISTRICT_CONSTRUCTED")
            except_ = {a.get("DistrictType") for a, m, s_ in
                       eng.player_effects("EFFECT_ADJUST_PLAYER_FREE_BUILDING_WHEN_SPECIALTY_DISTRICT_CONSTRUCTED_EXCEPT")}
            if free and not any(r.district_is(item, d) for d in except_ if d):
                first = [b for (b,) in r.q("select BuildingType from Buildings where PrereqDistrict=? and IsWonder=0", item)
                         if not r.q("select 1 from BuildingPrereqs where Building=?", b)
                         and not r.building_replaced(b, st.player["civilization"], st.player["leader"])
                         and b not in city["buildings"]]
                if first:
                    city["buildings"].append(min(first, key=lambda b: r.cost(b)))
        if item_kind(item) == "BUILDING" and r.is_wonder(item):
            era = (r.q("select t.EraType from Buildings b join Technologies t on t.TechnologyType=b.PrereqTech where b.BuildingType=?", item)
                   or r.q("select c.EraType from Buildings b join Civics c on c.CivicType=b.PrereqCivic where b.BuildingType=?", item))
            if era:
                if eng.player_effects("EFFECT_ADJUST_FREE_TECH_BOOST_WONDER_ERA"):
                    self.boost_cheapest(st, era[0][0], civic=False)
                if eng.player_effects("EFFECT_ADJUST_FREE_CIVIC_BOOST_WONDER_ERA"):
                    self.boost_cheapest(st, era[0][0], civic=True)

    # ------------------------------------------------------------------ boosts from the economy
    def endogenous_boosts(self, st):
        r = self.r
        out = []
        improvements = {}
        improved_tiles = 0
        for c in st.cities:
            for p in c["owned"]:
                if p["improvement"]:
                    improvements[p["improvement"]] = improvements.get(p["improvement"], 0) + 1
                    improved_tiles += 1
        pop = sum(st.acc.cities[c["name"]].population for c in st.cities)
        districts = {}
        for c in st.cities:
            for x in c["districts"]:
                if x["complete"] and r.district_requires_population(x["type"]):
                    districts[x["type"]] = districts.get(x["type"], 0) + 1
        buildings = {b for c in st.cities for b in c["buildings"]}
        units = dict(st.units)
        for row in r.boost_rows():
            item, cls, n, unit, bldg, imp, tech, res, dist = row
            n = n or 1
            ok = False
            if cls == "BOOST_TRIGGER_NUM_IMPROVED_TILES":
                ok = improved_tiles >= n
            elif cls == "BOOST_TRIGGER_HAVE_X_IMPROVEMENTS":
                ok = improvements.get(imp, 0) >= n
            elif cls == "BOOST_TRIGGER_EMPIRE_POPULATION":
                ok = pop >= n
            elif cls == "BOOST_TRIGGER_OWN_X_UNITS_OF_TYPE":
                ok = units.get(unit, 0) >= n or st.acc.units_trained.get(unit, 0) >= n
            elif cls == "BOOST_TRIGGER_HAVE_X_UNIQUE_SPECIALTY_DISTRICTS":
                ok = len(districts) >= n
            elif cls == "BOOST_TRIGGER_HAVE_X_DISTRICTS":
                ok = districts.get(dist, 0) >= n
            elif cls == "BOOST_TRIGGER_CONSTRUCT_BUILDING":
                ok = bldg in buildings
            elif cls == "BOOST_TRIGGER_MAINTAIN_X_TRADE_ROUTES":
                ok = sum(1 for t in st.traders if t.get("dest")) >= n
            elif cls == "BOOST_TRIGGER_RESEARCH_TECH":
                ok = tech in st.acc.techs
            elif cls == "BOOST_TRIGGER_CREATE_PANTHEON":
                ok = bool(st.player.get("pantheon"))
            elif cls == "BOOST_TRIGGER_IMPROVE_SPECIFIC_RESOURCE":
                ok = any(p["improvement"] == imp and p["resource"] == res for c in st.cities for p in c["owned"])
            if ok:
                out.append(item)
        return out

    # ------------------------------------------------------------------ search
    def run(self, st0, plan, base_factory=None):
        """Simulate to the horizon with `plan` overrides; other decisions from base_factory() (a fresh chooser,
        e.g. the recorded game's) or the built-in base policy."""
        st = copy.deepcopy(st0)
        base = base_factory() if base_factory else None

        def chooser(s, key, options):
            fallback = base(s, key, options) if base else self.base(s, key, options)
            want = plan.get(key)
            if want is not None and want in options:
                return want
            return fallback

        while st.turn <= self.s.horizon:
            self.advance(st, chooser)
        return st, st.score + self.s.terminal_turns * sum(self.s.score_weights.get(k, 0) * v for k, v in st.last_output.items())

    def options_at(self, st0, plan, key, base_factory=None):
        st = copy.deepcopy(st0)
        found = []
        base = base_factory() if base_factory else None

        def chooser(s, k, options):
            if k == key:
                found.append(list(options))
            fallback = base(s, k, options) if base else self.base(s, k, options)
            want = plan.get(k)
            return want if want is not None and want in options else fallback

        while st.turn <= self.s.horizon and not found:
            self.advance(st, chooser)
        return found[0] if found else []

    def search(self, st0, passes=3, max_options=None, log=print, kinds=None, base_factory=None):
        """Rollout improvement over every decision key, repeated until a pass changes nothing.
        Returns the plan plus a list of improvements (key, turn, old choice, new choice, value gain)."""
        plan = {}
        improvements = []
        final, best = self.run(st0, plan, base_factory)
        log(f"base policy value {best:.1f} ({len(final.log)} decisions)")
        import time as _t
        for p in range(passes):
            changed = 0
            keys = [k for k, _, _ in final.log if kinds is None or k[0] in kinds]
            t0 = _t.time()
            for key in keys:
                options = self.options_at(st0, plan, key, base_factory)
                turn_of = next((t for k, t, _ in final.log if k == key), None)
                old_choice = next((c for k, _, c in final.log if k == key), None)
                if len(options) < 2:
                    continue
                if max_options:
                    options = options[:max_options]
                current = plan.get(key)
                for opt in options:
                    if opt == current:
                        continue
                    trial = dict(plan)
                    trial[key] = opt
                    _, v = self.run(st0, trial, base_factory)
                    if v > best + 1e-6:
                        improvements.append((key, turn_of, old_choice, opt, v - best))
                        plan, best, changed = trial, v, changed + 1
                        log(f"  pass {p + 1} {key}: -> {opt}  value {best:.1f}  ({_t.time() - t0:.0f}s)")
                final, best = self.run(st0, plan, base_factory)
            log(f"pass {p + 1}: {changed} changes, value {best:.1f}")
            if not changed:
                break
        self.improvements = improvements
        return plan, final, best


# ---------------------------------------------------------------------------------- the recorded game
def recorded_choices(rows):
    """What the player actually did, in order, per decision kind."""
    by_turn = {d["turn"]: d for d in rows}
    tech_order, civic_order, queues, improvements, sites = [], [], {}, [], []
    policy_sets, govs = [], []
    seen_cities, seen_imp = set(), set()
    harvests, prev_plots = [], {}
    tiles = {}  # city centre -> [(turn, xy)]: the tiles each city got after its founding
    for a, b in zip(rows, rows[1:]):
        before = {c["name"]: {(p["x"], p["y"]) for p in c["owned"]} for c in a["cities"]}
        for c in b["cities"]:
            if c["name"] in before:
                new = sorted({(p["x"], p["y"]) for p in c["owned"]} - before[c["name"]])
                tiles.setdefault((c["center"]["x"], c["center"]["y"]), []).extend((a["turn"], xy) for xy in new)
    for d in rows:
        p = d["player"]
        for t in p["techs"]:
            if t not in tech_order:
                tech_order.append(t)
        for t in p["civics"]:
            if t not in civic_order:
                civic_order.append(t)
        if not policy_sets or policy_sets[-1] != p["policies"]:
            policy_sets.append(list(p["policies"]))
        if p["government"] and (not govs or govs[-1] != p["government"]):
            govs.append(p["government"])
        prev_d = by_turn.get(d["turn"] - 1)
        prev_c = {c["name"]: c for c in prev_d["cities"]} if prev_d else {}
        for c in d["cities"]:
            item = c["production"]["current"]
            q = queues.setdefault(c["name"], [])
            pc = prev_c.get(c["name"])
            restarted = pc is not None and pc["production"]["current"] == item and \
                c["production"]["progress"].get(item, 0) < pc["production"]["progress"].get(item, 0)
            if item and (not q or q[-1] != item or restarted):
                q.append(item)
            if c["name"] not in seen_cities:
                seen_cities.add(c["name"])
                if not c["capital"]:
                    sites.append((c["name"], (c["center"]["x"], c["center"]["y"])))
            for pl in c["owned"]:
                key = (pl["x"], pl["y"], pl["improvement"])
                if pl["improvement"] and key not in seen_imp:
                    seen_imp.add(key)
                    improvements.append(((pl["x"], pl["y"]), pl["improvement"]))
                # harvests: a feature or resource gone from a plot without a district or wonder (chops, Liang ...)
                was = prev_plots.get((pl["x"], pl["y"]))
                if was and not pl.get("district") and not pl.get("wonder"):
                    if was[0] and not pl["feature"]:
                        harvests.append(((pl["x"], pl["y"]), "feature"))
                    if was[1] and not pl["resource"]:
                        harvests.append(((pl["x"], pl["y"]), "resource"))
        prev_plots = {(pl["x"], pl["y"]): (pl["feature"], pl["resource"]) for c in d["cities"] for pl in c["owned"]}
    last = rows[-1]["player"]
    tech_order += [t for t in last["techProgress"] if t not in tech_order]
    civic_order += [t for t in last["civicProgress"] if t not in civic_order]
    # Purchases: units that appeared without any city finishing them (turn of the purchase -> item).
    purchases, tile_buys = {}, {}
    for a, b in zip(rows, rows[1:]):
        huts = sum((b["player"].get("goodyHutsReceived") or {}).values()) > sum((a["player"].get("goodyHutsReceived") or {}).values())
        for u, n in b["player"]["unitsTrained"].items():
            if n - a["player"]["unitsTrained"].get(u, 0) > produced_between(a, b, u) and not huts:
                purchases[a["turn"]] = u
        # Tile purchases: an unexplained gold drop of a tile's price, and a city that gained a tile.
        city_gold = sum(y["modified"]["YIELD_GOLD"] for y in a["cityYields"]) if a.get("cityYields") else 0
        drop = a["player"]["gold"] + city_gold - b["player"]["gold"]
        if 20 <= drop <= 40 and a["turn"] not in purchases:
            before = {c["name"]: len(c["owned"]) for c in a["cities"]}
            grown = [c["name"] for c in b["cities"] if len(c["owned"]) > before.get(c["name"], 7)]
            if grown:
                tile_buys[a["turn"]] = grown[-1]
    return {"purchases": purchases, "tile_buys": tile_buys, "tech": tech_order, "civic": civic_order, "queues": queues, "improvements": improvements,
            "sites": sites, "policies": policy_sets, "govs": govs, "pantheon": last.get("pantheon"), "harvests": harvests, "tiles": tiles}


def recorded_chooser(planner, rec):
    """A chooser that follows the recorded game wherever an equivalent option exists."""
    queues = {k: list(v) for k, v in rec["queues"].items()}
    imps = list(rec["improvements"])
    harvests = list(rec.get("harvests", []))
    state = {"policy_i": 0}

    def choose(st, key, options):
        kind = key[0]
        if kind == "research":
            for t in rec["tech"]:
                if t in options:
                    return t
        elif kind == "civic":
            for t in rec["civic"]:
                if t in options:
                    return t
        elif kind == "gov":
            for g in rec["govs"]:
                if g in options and g != st.player["government"]:
                    return g
            return st.player["government"] if st.player["government"] in options else options[0]
        elif kind == "policy":
            # The recorded set that contains cards valid now: pick its i-th card that fits this slot.
            for pset in reversed(rec["policies"]):
                cands = [p for p in pset if p in options]
                if cands:
                    return cands[0]
            return "NONE"
        elif kind == "pantheon":
            if rec["pantheon"] in options:
                return rec["pantheon"]
        elif kind == "prod":
            q = queues.get(key[1], [])
            while q:
                item = q.pop(0)
                if item in options:
                    return item
        elif kind == "builder":
            for i, (xy, imp) in enumerate(imps):
                for o in options:
                    if o[0] == "improve" and o[2] == xy and o[3] == imp:
                        imps.pop(i)
                        return o
            for i, (xy, what) in enumerate(harvests):
                for o in options:
                    if o[0] == "harvest" and o[2] == xy and o[3] == what:
                        harvests.pop(i)
                        return o
            return ("wait",) if ("wait",) in options else options[-1]
        elif kind == "buy":
            want = rec["purchases"].get(st.turn)
            for o in options:
                if want and o[0] == "item" and o[2] == want:
                    return o
            tile = rec["tile_buys"].get(st.turn)
            for o in options:
                if tile and o[0] == "tile":
                    return o if o[1] == tile else next((x for x in options if x[0] == "tile" and x[1] == tile), o)
            return ("none",)
        elif kind == "focus":
            return "balanced"
        return planner.base(st, key, options)

    return choose


def recorded_factory(planner, rows):
    """A factory of fresh recorded-game choosers (they consume queues, so each run needs its own)."""
    rec = recorded_choices(rows)
    return lambda: recorded_chooser(planner, rec)


def run_recorded(planner, st0, rows, tiles=False):
    """The recorded game played in the model; tiles: its cities' tiles as recorded (Settings.recorded_tiles)."""
    rec = recorded_choices(rows)
    old_sites, old_bc, old_rt = planner.s.sites, planner.s.builder_candidates, planner.s.recorded_tiles
    planner.s.sites, planner.s.builder_candidates = rec["sites"], None
    if tiles:
        planner.s.recorded_tiles = rec["tiles"]
    st = copy.deepcopy(st0)
    choose = recorded_chooser(planner, rec)
    while st.turn <= planner.s.horizon:
        planner.advance(st, choose)
    planner.s.sites, planner.s.builder_candidates, planner.s.recorded_tiles = old_sites, old_bc, old_rt
    return st, st.score + planner.s.terminal_turns * sum(planner.s.score_weights.get(k, 0) * v for k, v in st.last_output.items())
