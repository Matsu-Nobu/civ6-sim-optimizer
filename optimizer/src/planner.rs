//! Full-action planner (civ6sim/plan2.py FullPlanner): every economic decision is a choice; advance() plays a turn.
use crate::amenities::{amenity_scores, happiness};
use crate::engine::{ss, Engine, EngineCache, Subj};
use crate::model::*;
use crate::placement::{district_ok, improvement_options, wonder_ok};
use crate::production::*;
use crate::rules::{Rules, Y};
use crate::scenario::{Scenario, Settings};
use crate::sim::{boost_amount, city_turn_yields, research_cost, step, Event};
use crate::sym::{sym, Sym};
use crate::world::{plot_value, travel_turns, weighted, World, Weights, DEFAULT_WEIGHTS};
use crate::yields::{district_adjacency, excluded_adjacencies, get_y, modifier_adjacency_rows, route_yields, tile_yields};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

/// The game's city focus (citizen management: food, production, gold, science, culture, faith): the yield focused
/// on weighs double; "balanced" is the default weights.
pub fn focus_weights(f: Sym) -> Weights {
    let mut w = DEFAULT_WEIGHTS;
    if let Some(i) = FOCUS.iter().skip(1).position(|x| ss(x) == f) {
        w[i] = 2.0 * DEFAULT_WEIGHTS[i];
    }
    w
}
pub const FOCUS: [&str; 7] = ["balanced", "food", "production", "gold", "science", "culture", "faith"];

pub struct NearMaps {
    at: HashMap<XY, Sym>,
    imp: HashMap<XY, Option<Sym>>,
}

pub trait Chooser {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice>;
}

pub struct Planner<'a> {
    pub r: &'a Rules,
    pub w: World<'a>,
    pub sc: Scenario,
    pub s: Settings,
    pub speed: Sym,
    pub establish: HashMap<Sym, i64>,
    cache: RefCell<EngineCache<'a>>,
    /// replaying the recorded game (crate::record): items, district sites, worked tiles, owned tiles, gold and faith
    /// follow the record
    pub rec: Option<Rc<crate::record::Record>>,
    /// the recorded player's play under the model's rules: the citizens work the recorded tiles (a player's
    /// choice the plans leave to the rule); nothing else is taken from the record
    pub tiles_rec: Option<Rc<crate::record::Record>>,
    /// replay: (turn, what the record did that the rules would not, or the other way round)
    pub rec_notes: RefCell<Vec<(i64, String)>>,
    /// science / culture floors checked at the end of the evaluation (crate::milestones)
    pub milestones: Option<std::sync::Arc<crate::milestones::Milestones>>,
}

const GP_CLASSES: [&str; 5] = ["GREAT_PERSON_CLASS_ENGINEER", "GREAT_PERSON_CLASS_MERCHANT", "GREAT_PERSON_CLASS_SCIENTIST", "GREAT_PERSON_CLASS_WRITER", "GREAT_PERSON_CLASS_GENERAL"];

impl<'a> Planner<'a> {
    pub fn new(r: &'a Rules, w: World<'a>, sc: Scenario, s: Settings, speed: Sym, establish: HashMap<Sym, i64>) -> Planner<'a> {
        Planner { r, w, sc, s, speed, establish, cache: RefCell::new(EngineCache::new()), rec: None, tiles_rec: None, rec_notes: RefCell::new(vec![]), milestones: None }
    }

    pub fn engine(&self, env: &Env, cities: &[City]) -> Rc<Engine<'a>> {
        self.cache.borrow_mut().get(self.r, env, cities)
    }

    pub fn player_env(&self, st: &State) -> Env {
        let _g = crate::prof::start(13);
        let mut p = st.player.clone();
        p.envoys = self.sc.envoys(st.turn).clone();
        Env {
            p,
            techs: st.acc.techs.clone(),
            civics: st.acc.civics.clone(),
            units_trained: st.acc.units_trained.clone(),
            game_era: self.sc.game_era(st.turn + 1),
            prophet_on_offer: self.sc.prophet_on_offer(st.turn),
            districts_completed: None,
            extra_luxuries: self.sc.extra_luxuries.get(&st.turn).copied().unwrap_or(0),
            allies: self.sc.allies.clone(),
        }
    }

    fn eng(&self, st: &State) -> Rc<Engine<'a>> {
        self.engine(&self.player_env(st), &st.cities)
    }

    // ---------------------------------------------------------------- decisions
    pub fn decide(&self, st: &mut State, kind: &str, ent: Ent, options: Vec<Choice>, ch: &mut dyn Chooser) -> Option<Choice> {
        let kind_s = sym(kind);
        let ck = (kind_s, ent.clone());
        let n = *st.counters.get(&ck).unwrap_or(&0);
        st.counters.insert(ck, n + 1);
        let key = Key { kind: kind_s, ent: ent.clone(), n };
        let capital = st.cities.first().map(|c| c.name);
        let mut options = options;
        let forced = self.s.fixed.iter().find(|(k, _)| *k == key).map(|(_, c)| c.clone()).or_else(|| {
            if capital.is_some() && ent == Ent::S(capital.unwrap()) {
                let k2 = Key { kind: kind_s, ent: Ent::S(s!("CAPITAL")), n };
                self.s.fixed.iter().find(|(k, _)| *k == k2).map(|(_, c)| c.clone())
            } else {
                None
            }
        });
        if let Some(f) = forced {
            if options.contains(&f) {
                options = vec![f];
            }
        }
        let choice = if options.is_empty() { None } else { ch.choose(self, st, &key, &options) };
        st.log.push((key, st.turn, choice.clone()));
        choice
    }

    /// The built-in base policy (plan2.base).
    pub fn base(&self, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        if options.is_empty() {
            return None;
        }
        let k = key.kind;
        let r = self.r;
        if k == s!("research") || k == s!("civic") {
            let prog = if k == s!("research") { &st.acc.tech_progress } else { &st.acc.civic_progress };
            let era = self.sc.game_era(st.turn);
            return options.iter().min_by(|a, b| {
                let f = |c: &Choice| { let t = c.sym().unwrap(); research_cost(r, t, self.speed, era) - prog.get(&t).copied().unwrap_or(0.0) };
                f(a).partial_cmp(&f(b)).unwrap()
            }).cloned();
        }
        if k == s!("gov") {
            // max by (number of slots, is current); ties: the first
            let mut best: Option<(usize, bool, &Choice)> = None;
            for o in options {
                let g = o.sym().unwrap();
                let v = (self.slots(g, st).len(), Some(g) == st.player.government);
                if best.map(|b| (v.0, v.1) > (b.0, b.1)).unwrap_or(true) {
                    best = Some((v.0, v.1, o));
                }
            }
            return best.map(|b| b.2.clone());
        }
        if k == s!("buy") {
            return Some(Choice::Nothing);
        }
        if k == s!("governor") {
            let pref = ["GOVERNOR_THE_RESOURCE_MANAGER", "GOVERNOR_THE_BUILDER", "GOVERNOR_THE_EDUCATOR", "GOVERNOR_THE_MERCHANT", "GOVERNOR_THE_CARDINAL"];
            let capital = st.cities.first().map(|c| c.name);
            let rank = |o: &Choice| -> (usize, i32, i32) {
                let (g, promote, city) = match o {
                    Choice::Appoint(g, c) => (*g, false, Some(*c)),
                    Choice::Promote(g, p) => (*g, true, Some(*p)),
                    _ => (Sym(0), false, None),
                };
                let gi = pref.iter().position(|p| ss(p) == g).unwrap_or(pref.len());
                (gi, if promote { 0 } else { 1 }, if city.is_some() && city == capital { 0 } else { 1 })
            };
            let real: Vec<&Choice> = options.iter().filter(|o| **o != Choice::Nothing).collect();
            if real.is_empty() {
                return Some(options[0].clone());
            }
            return real.into_iter().min_by_key(|o| rank(o)).cloned();
        }
        if k == s!("prod") {
            let order = ["UNIT_SETTLER", "UNIT_BUILDER", "BUILDING_MONUMENT", "DISTRICT_GOVERNMENT", "DISTRICT_COMMERCIAL_HUB",
                "DISTRICT_CAMPUS", "BUILDING_GRANARY", "BUILDING_LIBRARY", "BUILDING_MARKET", "UNIT_TRADER", "BUILDING_WATER_MILL"];
            if st.builders.is_empty() && options.contains(&Choice::S(s!("UNIT_BUILDER"))) {
                return Some(Choice::S(s!("UNIT_BUILDER")));
            }
            for it in order {
                if options.contains(&Choice::S(ss(it))) {
                    return Some(Choice::S(ss(it)));
                }
            }
            return Some(options[0].clone());
        }
        Some(options[0].clone())
    }

    // ---------------------------------------------------------------- governors
    fn governor_titles(&self, st: &State) -> i64 {
        let e = self.eng(st);
        let from_effects = e.player_effects(s!("EFFECT_ADJUST_PLAYER_GOVERNOR_POINTS")).iter().map(|x| x.args.f("Delta")).sum::<f64>() as i64;
        // the Government Plaza and each of its buildings award a title (game rule, not a modifier: their descriptions)
        let plaza = st.cities.iter().any(|c| c.has_complete_district(s!("DISTRICT_GOVERNMENT"))) as i64;
        let gov_buildings = st.cities.iter().flat_map(|c| c.buildings.iter()).filter(|b| b.starts_with("BUILDING_GOV_")).count() as i64;
        from_effects + plaza + gov_buildings
    }

    fn governor_options(&self, st: &mut State) -> Vec<Choice> {
        let r = self.r;
        if st.player.governors.is_none() {
            st.player.governors = Some(vec![]);
        }
        let govs = st.player.governors.as_ref().unwrap();
        let mut opts = vec![Choice::Nothing];
        let used: Vec<Option<i64>> = govs.iter().map(|g| g.city_id).collect();
        let t = r.tab("Governors");
        for i in r.scan("Governors", "Governors") {
            let gov = t.v(i, "GovernorType").sym().unwrap();
            let cs = t.v(i, "AssignCityState").truthy();
            if let Some(tr) = t.v(i, "TraitType").sym().filter(|x| !x.is_empty()) {
                if !r.civ_has_trait(st.player.civilization, st.player.leader, tr) {
                    continue;
                }
            }
            if let Some(g) = govs.iter().find(|g| g.gtype == gov) {
                for pr in r.list_sym("GovernorPromotionSets", "GovernorType", gov, "GovernorPromotion") {
                    if g.promotions.contains(&pr) {
                        continue;
                    }
                    let pre = r.list_sym("GovernorPromotionPrereqs", "GovernorPromotionType", pr, "PrereqGovernorPromotion");
                    if !pre.is_empty() && !pre.iter().any(|x| g.promotions.contains(x)) {
                        continue;
                    }
                    opts.push(Choice::Promote(gov, pr));
                }
            } else if !cs {
                for c in &st.cities {
                    if !used.contains(&Some(c.id)) {
                        opts.push(Choice::Appoint(gov, c.name));
                    }
                }
            }
        }
        opts
    }

    fn governor_step(&self, st: &mut State, ch: &mut dyn Chooser) {
        if st.player.governors.is_none() {
            st.player.governors = Some(vec![]);
        }
        let turn = st.turn;
        for g in st.player.governors.as_mut().unwrap().iter_mut() {
            if !g.established && turn >= g.ready {
                g.established = true;
            }
        }
        let spent: i64 = st.player.governors.as_ref().unwrap().iter().map(|g| g.promotions.len() as i64).sum();
        if self.governor_titles(st) <= spent {
            return;
        }
        let opts = self.governor_options(st);
        let pick = self.decide(st, "governor", Ent::S(Sym(0)), opts, ch);
        match pick {
            Some(Choice::Appoint(gov, cname)) => {
                let cid = st.city(cname).unwrap().id;
                let r = self.r;
                let mut base = vec![];
                let gps = r.tab("GovernorPromotionSets");
                let gp = r.tab("GovernorPromotions");
                for &i in gps.find("GovernorType", gov) {
                    let pr = gps.v(i, "GovernorPromotion").sym().unwrap();
                    for &j in gp.find("GovernorPromotionType", pr) {
                        if gp.v(j, "Level").i() == 0 {
                            base.push(pr);
                        }
                    }
                }
                let ready = st.turn + *self.establish.get(&gov).unwrap_or(&5);
                st.player.governors.as_mut().unwrap().push(Governor { gtype: gov, promotions: base, city_id: Some(cid), city_owner_id: None, established: false, ready });
            }
            Some(Choice::Promote(gov, pr)) => {
                if let Some(g) = st.player.governors.as_mut().unwrap().iter_mut().find(|g| g.gtype == gov) {
                    g.promotions.push(pr);
                }
            }
            _ => {}
        }
    }

    // ---------------------------------------------------------------- options
    fn available(&self, table: &str, col: &str, pre_table: &str, pre_key: &str, pre_col: &str, done: &BTreeSet<Sym>) -> Vec<Choice> {
        let r = self.r;
        let t = r.tab(table);
        let mut out = vec![];
        for i in r.scan(table, table) {
            let item = t.v(i, col).sym().unwrap();
            if done.contains(&item) {
                continue;
            }
            let pre = r.list_sym(pre_table, pre_key, item, pre_col);
            let era = t.v(i, "EraType").sym();
            if pre.is_empty() && era != Some(s!("ERA_ANCIENT")) {
                continue;
            }
            if pre.iter().all(|x| done.contains(x)) {
                out.push(Choice::S(item));
            }
        }
        out
    }
    pub fn available_techs(&self, st: &State) -> Vec<Choice> {
        self.available("Technologies", "TechnologyType", "TechnologyPrereqs", "Technology", "PrereqTech", &st.acc.techs)
    }
    pub fn available_civics(&self, st: &State) -> Vec<Choice> {
        self.available("Civics", "CivicType", "CivicPrereqs", "Civic", "PrereqCivic", &st.acc.civics)
    }
    fn available_governments(&self, st: &State) -> Vec<Sym> {
        let t = self.r.tab("Governments");
        self.r.scan("Governments", "Governments").into_iter().filter_map(|i| {
            let g = t.v(i, "GovernmentType").sym().unwrap();
            let pc = t.v(i, "PrereqCivic").sym().filter(|x| !x.is_empty());
            if (pc.is_none() || st.acc.civics.contains(&pc.unwrap())) && g != s!("GOVERNMENT_ANARCHY") { Some(g) } else { None }
        }).collect()
    }
    fn slots(&self, gov: Sym, st: &State) -> Vec<Sym> {
        policy_slots(self.r, &self.eng(st), gov)
    }
    /// ObsoletePolicies: a policy can no longer be slotted once the policy replacing it is unlocked (Ilkum once
    /// Feudalism unlocks Serfdom). Rows conditioned on a great person class are not modelled.
    fn obsolete_policy(&self, st: &State, pol: Sym) -> bool {
        let o = self.r.tab("ObsoletePolicies");
        let p = self.r.tab("Policies");
        o.find("PolicyType", pol).iter().any(|&i| {
            if o.v(i, "RequiresAvailableGreatPersonClass").sym().is_some_and(|x| !x.is_empty()) {
                return false;
            }
            let Some(by) = o.v(i, "ObsoletePolicy").sym().filter(|x| !x.is_empty()) else { return false };
            let j = p.first("PolicyType", by).unwrap_or_else(|| panic!("ObsoletePolicies: no policy {}", by.as_string()));
            let pc = p.v(j, "PrereqCivic").sym().filter(|x| !x.is_empty());
            let pt = p.v(j, "PrereqTech").sym().filter(|x| !x.is_empty());
            pc.map(|x| st.acc.civics.contains(&x)).unwrap_or(true) && pt.map(|x| st.acc.techs.contains(&x)).unwrap_or(true)
        })
    }
    fn policies_for_slot(&self, st: &State, slot: Sym) -> Vec<Sym> {
        let t = self.r.tab("Policies");
        let mut out = vec![];
        for i in self.r.scan("Policies", "Policies") {
            let pol = t.v(i, "PolicyType").sym().unwrap();
            if pol.starts_with("POLICY_GOV_") {
                continue;
            }
            let pc = t.v(i, "PrereqCivic").sym().filter(|x| !x.is_empty());
            let pt = t.v(i, "PrereqTech").sym().filter(|x| !x.is_empty());
            if pc.map(|x| !st.acc.civics.contains(&x)).unwrap_or(false) || pt.map(|x| !st.acc.techs.contains(&x)).unwrap_or(false) {
                continue;
            }
            if self.obsolete_policy(st, pol) {
                continue;
            }
            // Policies_XP1: game-era range and dark / golden age (the age from the record, scenario.age)
            let x = self.r.tab("Policies_XP1");
            if let Some(j) = x.first("PolicyType", pol) {
                let now = self.r.era_index(self.sc.game_era(st.turn)).unwrap_or(0);
                let era = |c: &str| x.v(j, c).sym().filter(|e| !e.is_empty()).and_then(|e| self.r.era_index(e));
                let golden = st.player.age == Some(s!("GOLDEN")) || st.player.age == Some(s!("HEROIC"));
                if (x.v(j, "RequiresDarkAge").i() != 0 && st.player.age != Some(s!("DARK"))) || (x.v(j, "RequiresGoldenAge").i() != 0 && !golden)
                    || era("MinimumGameEra").map(|lo| now < lo).unwrap_or(false) || era("MaximumGameEra").map(|hi| now > hi).unwrap_or(false) {
                    continue;
                }
            }
            if slot == s!("SLOT_WILDCARD") || t.v(i, "GovernmentSlotType").sym() == Some(slot) {
                out.push(pol);
            }
        }
        // a government the player had before leaves its legacy card (Governments.PolicyToUnlock, a wildcard)
        for &g in &st.past_govs {
            if Some(g) == st.player.government {
                continue;
            }
            if let Some(card) = self.r.lookup_sym("Governments", "GovernmentType", g, "PolicyToUnlock") {
                if (slot == s!("SLOT_WILDCARD") || t.first("PolicyType", card).and_then(|i| t.v(i, "GovernmentSlotType").sym()) == Some(slot)) && !out.contains(&card) {
                    out.push(card);
                }
            }
        }
        out
    }

    /// The state at a recorded turn (model::state_from_row) with what needs the planner: governments known, the
    /// player's trade routes running, the era its dedications belong to, and (from the record) the research and civic
    /// being worked on.
    pub fn state_at(&self, d: &crate::json::Value) -> State {
        let mut st = crate::model::state_from_row(d);
        st.known_govs = self.available_governments(&st).into_iter().collect();
        st.era_mark = Some(self.sc.game_era(st.turn));
        let me = st.player.player_id;
        for r in d["tradeRoutes"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
            if r["origin"]["playerId"].as_i64() != me {
                continue;
            }
            let (Some(o), Some(dn)) = (osym(&r["origin"]["name"]), osym(&r["destination"]["name"])) else { continue };
            let foreign = r["destination"]["playerId"].as_i64() != me;
            let info = if foreign { self.sc.world_cities(st.turn).iter().find(|c| c.name == dn).cloned() } else { None };
            let start = r["started"].as_i64().unwrap_or(st.turn);
            let until = Some(start + (self.r.param("TRADE_ROUTE_TURN_DURATION_BASE") as i64).max(30));
            st.traders.push(Trader { origin: o, dest: Some(dn), info, until, start });
        }
        if let Some(rec) = self.rec.as_ref() {
            // governments held before this turn (their legacy cards)
            for (_, r) in rec.turns.range(..st.turn) {
                if let Some(g) = r.gov.filter(|g| Some(*g) != st.player.government) {
                    st.past_govs.insert(g);
                }
            }
            st.research = rec.research.get(&st.turn).copied();
            st.civic = rec.civic.get(&st.turn).copied();
        }
        st
    }

    /// Rome (BBG): a free cheapest building (monument, else granary) in every city once Early Empire is done (new
    /// cities on founding; the cities of the turn it completes at that turn's end, as the saves show).
    fn rome_free_buildings(&self, st: &mut State) {
        if !(st.acc.civics.contains(&s!("CIVIC_EARLY_EMPIRE")) && st.player.civilization == Some(s!("CIVILIZATION_ROME"))) {
            return;
        }
        for ci in 0..st.cities.len() {
            let n = st.cities[ci].name;
            if !st.free_building_given.contains(&n) {
                for b in ["BUILDING_MONUMENT", "BUILDING_GRANARY"] {
                    if !st.cities[ci].buildings.contains(&ss(b)) {
                        st.cities[ci].buildings.push(ss(b));
                        break;
                    }
                }
                st.free_building_given.insert(n);
            }
        }
    }

    /// The recorded game's boosts its rules do not explain (from the next save's state): meeting, exploring, combat,
    /// tribal villages, city-states ... These come from the record (Scenario.exo_boosts, once the plan has scouted as
    /// much), the rest from the rules.
    pub fn record_exo_boosts(&mut self, rows: &crate::json::Value) {
        let rows: Vec<&crate::json::Value> = rows.as_array().unwrap().iter().collect();
        let mut exo: HashMap<i64, Vec<Sym>> = HashMap::new();
        let boosted = |d: &crate::json::Value| -> BTreeSet<Sym> {
            ["techsBoosted", "civicsInspired"].iter().flat_map(|k| d["player"][*k].as_array().cloned().unwrap_or_default()).filter_map(|x| osym(&x)).collect()
        };
        for w in rows.windows(2) {
            let (a, b) = (w[0], w[1]);
            let new: Vec<Sym> = boosted(b).difference(&boosted(a)).copied().collect();
            if new.is_empty() {
                continue;
            }
            let explained = if b["cities"].as_array().map(|c| c.is_empty()).unwrap_or(true) { vec![] } else { self.endogenous_boosts(&self.state_at(b)) };
            let t = a["turn"].as_i64().unwrap();
            for x in new {
                if !explained.contains(&x) {
                    exo.entry(t).or_default().push(x);
                }
            }
        }
        self.sc.exo_boosts = exo;
    }

    /// Units granted to each city the player founds (MODIFIER_PLAYER_BUILT_CITIES_GRANT_FREE_UNIT of its buildings).
    fn founding_units(&self, st: &State) -> Vec<(Sym, i64)> {
        let r = self.r;
        let bm = r.tab("BuildingModifiers");
        let mut out = vec![];
        for c in &st.cities {
            for &b in &c.buildings {
                for &i in bm.find("BuildingType", b) {
                    let Some(mid) = bm.v(i, "ModifierId").sym() else { continue };
                    if r.lookup_sym("Modifiers", "ModifierId", mid, "ModifierType") != Some(s!("MODIFIER_PLAYER_BUILT_CITIES_GRANT_FREE_UNIT")) {
                        continue;
                    }
                    let args = r.tab("ModifierArguments");
                    let arg = |name: &str| args.find("ModifierId", mid).iter().find(|&&j| args.v(j, "Name").sym() == Some(sym(name))).map(|&j| args.v(j, "Value").clone());
                    if let Some(u) = arg("UnitType").and_then(|v| v.sym()) {
                        out.push((u, arg("Amount").map(|v| v.i()).unwrap_or(1).max(1)));
                    }
                }
            }
        }
        out
    }

    /// A Government Plaza building: one per player, none of its tier's alternatives built
    /// (MutuallyExclusiveBuildings), and a government of its tier or higher (Buildings.GovernmentTierRequirement).
    fn gov_building_ok(&self, st: &State, b: Sym) -> bool {
        let r = self.r;
        let all: Vec<Sym> = st.cities.iter().flat_map(|c| c.buildings.iter().copied()).collect();
        if all.contains(&b) || st.cities.iter().any(|c| c.current == Some(b)) {
            return false;
        }
        let me = r.tab("MutuallyExclusiveBuildings");
        if me.find("Building", b).iter().any(|&i| me.v(i, "MutuallyExclusiveBuilding").sym().map(|x| all.contains(&x)).unwrap_or(false)) {
            return false;
        }
        let tier_of = |t: Option<Sym>| t.and_then(|t| r.lookup("GovernmentTiers", "TierType", t, "Sorting").map(|v| v.i()));
        let need = tier_of(r.lookup_sym("Buildings", "BuildingType", b, "GovernmentTierRequirement"));
        let have = tier_of(st.player.government.and_then(|g| r.lookup_sym("Governments", "GovernmentType", g, "Tier")));
        match need {
            None => true,
            Some(n) => have.map(|h| h >= n).unwrap_or(false),
        }
    }

    pub fn trade_capacity(&self, st: &State) -> i64 {
        trade_route_capacity(&self.eng(st))
    }


    pub fn buildable(&self, st: &State, ci: usize) -> Vec<Choice> {
        self.buildable_for(st, ci, false)
    }

    /// purchase: only what can be bought here (units and buildings but wonders; no districts, wonders or projects)
    fn buildable_for(&self, st: &State, ci: usize, purchase: bool) -> Vec<Choice> {
        let _g = crate::prof::start(3);
        let r = self.r;
        let city = &st.cities[ci];
        let techs = &st.acc.techs;
        let civics = &st.acc.civics;
        let ca = st.cacc(city.name);
        let done_d: Vec<Sym> = city.districts.iter().filter(|x| x.complete).map(|x| x.dtype).collect();
        let planned_d: Vec<Sym> = city.districts.iter().map(|x| x.dtype).collect();
        let specialty = city.districts.iter().filter(|x| r.district_requires_population(x.dtype)).count() as i64;
        let mut allowed = 1 + (ca.population as i64 - 1).div_euclid(r.param("DISTRICT_POPULATION_REQUIRED_PER") as i64);
        let (civ, leader) = (st.player.civilization, st.player.leader);
        let mut out = vec![];
        let e = self.eng(st);
        let site_ctx = std::cell::OnceCell::new();
        let site_possible = |item: Sym, wonder: bool| {
            let (nm, taken) = site_ctx.get_or_init(|| (self.near_maps(st), self.taken(st, ci)));
            self.site_possible_with(st, ci, item, wonder, &e, nm, taken)
        };
        let t = r.tab("Units");
        let s = |t: &crate::rules::Table, i: u32, c: &str| t.v(i, c).sym().filter(|x| !x.is_empty());
        for i in r.scan("Units", "Units") {
            let u = t.v(i, "UnitType").sym().unwrap();
            let (pt, pc) = (s(t, i, "PrereqTech"), s(t, i, "PrereqCivic"));
            if !t.v(i, "CanTrain").truthy() || t.v(i, "Domain").sym() != Some(s!("DOMAIN_LAND"))
                || pt.map(|x| !techs.contains(&x)).unwrap_or(false) || pc.map(|x| !civics.contains(&x)).unwrap_or(false) {
                continue;
            }
            let rel = t.v(i, "ReligiousStrength").f() > 0.0 || t.v(i, "SpreadCharges").f() > 0.0 || t.v(i, "FoundReligion").truthy() || t.v(i, "EnabledByReligion").truthy();
            if rel {
                continue;
            }
            if let Some(tr) = s(t, i, "TraitType") {
                if !r.civ_has_trait(civ, leader, tr) {
                    continue;
                }
            }
            if u == s!("UNIT_SETTLER") && ((ca.population < 2 && !settler_free_pop(&e, city.name)) || st.cities.len() + st.settlers.len() >= self.s.max_cities) {
                continue;
            }
            // the game builds traders beyond the route capacity (they wait for a free route); the search does not offer
            // them (pruning: a trader without a route yields nothing), the replay takes them as recorded
            if u == s!("UNIT_TRADER") && self.rec.is_none() && st.traders.len() as i64 >= trade_route_capacity(&e) {
                continue;
            }
            // spies: as many as EFFECT_GRANT_SPY of the civics (Diplomatic Service, ...) allow; none before
            if u == s!("UNIT_SPY") && *st.acc.units_trained.get(&u).unwrap_or(&0)
                >= e.player_effects(s!("EFFECT_GRANT_SPY")).iter().map(|x| x.args.get("Amount").map(|_| x.args.f("Amount") as i64).unwrap_or(1)).sum::<i64>() {
                continue;
            }
            if r.unit_replaced(u, civ, leader) {
                continue;
            }
            // units that do nothing for the objective (military, spies) are not offered to a plan; the recorded
            // player's play (replay, record genome) keeps them
            if self.rec.is_none() && self.tiles_rec.is_none() && !self.s.allow_idle_units && crate::opt::idle_unit(u) {
                continue;
            }
            let cls = s(t, i, "PromotionClass");
            if u.contains("GREAT") || u.starts_with("UNIT_ARCHAEOLOGIST")
                || (cls.is_none() && u != s!("UNIT_SETTLER") && u != s!("UNIT_BUILDER") && u != s!("UNIT_TRADER")) {
                continue;
            }
            out.push(Choice::S(u));
        }
        let t = r.tab("Buildings");
        for i in r.scan("Buildings", "Buildings") {
            if t.v(i, "InternalOnly").truthy() {
                continue;
            }
            let b = t.v(i, "BuildingType").sym().unwrap();
            let (pt, pc, pd) = (s(t, i, "PrereqTech"), s(t, i, "PrereqCivic"), s(t, i, "PrereqDistrict"));
            if city.buildings.contains(&b) || pt.map(|x| !techs.contains(&x)).unwrap_or(false) || pc.map(|x| !civics.contains(&x)).unwrap_or(false) {
                continue;
            }
            if let Some(pd) = pd {
                if !done_d.contains(&pd) && !r.district_replaces_done(pd, &done_d) {
                    continue;
                }
            }
            if let Some(tr) = s(t, i, "TraitType") {
                if !r.civ_has_trait(civ, leader, tr) {
                    continue;
                }
            }
            if r.building_replaced(b, civ, leader) || !r.building_prereqs_met(b, &city.buildings) {
                continue;
            }
            if t.v(i, "IsWonder").truthy() {
                if purchase {
                    continue;
                }
                if self.sc.wonders_taken(st.turn).contains(&b) || st.cities.iter().any(|cc| cc.buildings.contains(&b))
                    || st.cities.iter().enumerate().any(|(j, cc)| j != ci && cc.current == Some(b))
                    || !site_possible(b, true) {
                    continue;
                }
            }
            if b == s!("BUILDING_PALACE") {
                continue;
            }
            if b.starts_with("BUILDING_GOV_") && !self.gov_building_ok(st, b) {
                continue;
            }
            out.push(Choice::S(b));
        }
        if purchase {
            return out;
        }
        allowed += e.city_effects(s!("EFFECT_ADJUST_CITY_EXTRA_DISTRICTS"), city.name).iter().map(|x| x.args.f("Amount") as i64).sum::<i64>();
        let prereq: Vec<(Sym, Sym)> = e.player_effects(s!("EFFECT_ADJUST_DISTRICT_PREREQ")).iter().filter_map(|x| Some((x.args.get("DistrictType")?, x.args.get("TechType")?))).collect();
        let t = r.tab("Districts");
        for i in r.scan("Districts", "Districts") {
            if t.v(i, "InternalOnly").truthy() || t.v(i, "CityCenter").truthy() {
                continue;
            }
            let dist = t.v(i, "DistrictType").sym().unwrap();
            if dist == s!("DISTRICT_SPACEPORT") || dist == s!("DISTRICT_CANAL") {
                continue;
            }
            let pt = prereq.iter().rev().find(|(d, _)| *d == dist).map(|(_, t)| Some(*t)).unwrap_or(s(t, i, "PrereqTech"));
            let pc = s(t, i, "PrereqCivic");
            let pop = t.v(i, "RequiresPopulation").truthy();
            if pt.map(|x| !techs.contains(&x)).unwrap_or(false) || pc.map(|x| !civics.contains(&x)).unwrap_or(false) || (pop && specialty >= allowed) {
                continue;
            }
            if (t.v(i, "OnePerCity").truthy() || pop) && planned_d.contains(&dist) {
                continue;
            }
            let maxp = t.v(i, "MaxPerPlayer").i();
            if maxp > 0 && st.cities.iter().map(|cc| cc.districts.iter().filter(|x| x.dtype == dist).count() as i64).sum::<i64>() >= maxp {
                continue;
            }
            if let Some(tr) = s(t, i, "TraitType") {
                if !r.civ_has_trait(civ, leader, tr) {
                    continue;
                }
            }
            if r.district_replaced(dist, civ, leader) {
                continue;
            }
            if dist == s!("DISTRICT_GOVERNMENT") && st.cities.iter().any(|cc| cc.districts.iter().any(|x| x.dtype == s!("DISTRICT_GOVERNMENT"))) {
                continue;
            }
            if !site_possible(dist, false) {
                continue;
            }
            out.push(Choice::S(dist));
        }
        let unlocked: Vec<Sym> = e.player_effects(s!("EFFECT_ADD_PLAYER_PROJECT_AVAILABILITY")).iter().filter_map(|x| x.args.get("ProjectType")).collect();
        let t = r.tab("Projects");
        for i in r.scan("Projects", "Projects") {
            let proj = t.v(i, "ProjectType").sym().unwrap();
            let (pt, pc, pd) = (s(t, i, "PrereqTech"), s(t, i, "PrereqCivic"), s(t, i, "PrereqDistrict"));
            if t.v(i, "SpaceRace").truthy() || t.v(i, "WMD").truthy() || t.v(i, "MaxPlayerInstances").truthy()
                || pt.map(|x| !techs.contains(&x)).unwrap_or(false) || pc.map(|x| !civics.contains(&x)).unwrap_or(false) {
                continue;
            }
            if t.v(i, "UnlocksFromEffect").truthy() && !unlocked.contains(&proj) {
                continue;
            }
            if ["PROJECT_REPAIR_OUTER_DEFENSES", "PROJECT_SEND_AID", "PROJECT_TRAIN_ATHLETES"].iter().any(|p| ss(p) == proj) {
                continue;
            }
            if let Some(pd) = pd {
                if !done_d.contains(&pd) && !r.district_replaces_done(pd, &done_d) {
                    continue;
                }
            }
            out.push(Choice::S(proj));
        }
        out
    }

    pub fn route_endpoint(&self, st: &State, c: &City) -> Endpoint {
        Endpoint {
            name: c.name,
            id: c.id,
            player_id: st.player.player_id,
            civilization: st.player.civilization,
            leader: st.player.leader,
            x: c.center.0,
            y: c.center.1,
            districts: c.districts.iter().filter(|x| x.complete).map(|x| x.dtype).collect(),
            tech_count: st.acc.techs.len() as i64,
            civic_count: st.acc.civics.len() as i64,
            plots: c.owned.iter().map(|p| (p.terrain, p.resource, p.improvement)).collect(),
        }
    }

    fn route_options(&self, st: &State, origin: Sym) -> Vec<(f64, Sym, Endpoint)> {
        let _g = crate::prof::start(8);
        let r = self.r;
        let oc = st.city(origin).unwrap();
        let rng = r.param("TRADE_ROUTE_BASE_RANGE") as i32;
        let e = self.eng(st);
        let mut dests: Vec<Endpoint> = st.cities.iter().filter(|c| c.name != origin).map(|c| self.route_endpoint(st, c)).collect();
        dests.extend(self.sc.world_cities(st.turn).iter().cloned());
        let mut out = vec![];
        for d in dests {
            if self.w.map.distance(oc.center, (d.x, d.y)) > rng {
                continue;
            }
            let route = Route { origin: self.route_endpoint(st, oc), dest: d.clone() };
            let v: f64 = route_yields(r, &e, &route, oc).iter().map(|(_, v)| v).sum();
            out.push((v, d.name, d));
        }
        out.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        out
    }

    /// A trader's next route: any of the player's cities as its home (it walks there first; the route starts on
    /// arrival), to one of the best destinations in that city's range.
    fn start_route(&self, st: &mut State, origin: Sym, ch: &mut dyn Chooser) -> Trader {
        let idle = Trader { origin, dest: None, info: None, until: None, start: 0 };
        if st.traders.iter().filter(|t| t.dest.is_some()).count() as i64 >= self.trade_capacity(st) {
            return idle;
        }
        let Some(at) = st.city(origin).map(|c| c.center) else { return idle };
        // (value, choice, destination, turns to the home city)
        let mut opts: Vec<(f64, Choice, Endpoint, i64)> = vec![];
        let homes: Vec<(Sym, XY)> = st.cities.iter().map(|c| (c.name, c.center)).collect();
        for (home, xy) in homes {
            // replay: the route starts on the recorded turn (the trader's walk is not in the saves)
            let k = if home == origin || self.rec.is_some() || self.tiles_rec.is_some() { 0 } else { match self.travel(st, at, xy, "UNIT_TRADER") { Some(k) => k, None => continue } };
            for (v, dn, d) in self.route_options(st, home).into_iter().take(3) {
                opts.push((v * (1.0 - k as f64 / 30.0).max(0.0), Choice::Route(home, dn), d, k));
            }
        }
        opts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let names: Vec<Choice> = opts.iter().map(|o| o.1.clone()).collect();
        if names.is_empty() {
            return idle;
        }
        // numbered per turn: a change elsewhere does not shift the later routes
        let Some(c) = self.decide(st, "trade", Ent::I(st.turn), names, ch) else { return idle };
        let Some(o) = opts.iter().find(|o| o.1 == c) else { return idle };
        let Choice::Route(home, dn) = c else { return idle };
        let d = o.2.clone();
        let k = o.3;
        let hc = st.city(home).unwrap();
        let dist = self.w.map.distance(hc.center, (d.x, d.y)) as i64;
        let info = if d.player_id != st.player.player_id { Some(d) } else { None };
        let start = st.turn + k;
        Trader { origin: home, dest: Some(dn), info, until: Some(start + (self.r.param("TRADE_ROUTE_TURN_DURATION_BASE") as i64).max(2 * dist + 4)), start }
    }

    fn routes_now(&self, st: &State) -> Vec<Route> {
        let mut out = vec![];
        for t in &st.traders {
            let Some(dn) = t.dest else { continue };
            if t.start > st.turn {
                continue;
            }
            let Some(oc) = st.city(t.origin) else { continue };
            let dest = match &t.info {
                None => match st.city(dn) {
                    Some(dc) => self.route_endpoint(st, dc),
                    None => continue,
                },
                Some(info) => self.sc.world_cities(st.turn).iter().find(|c| c.name == dn).cloned().unwrap_or_else(|| info.clone()),
            };
            out.push(Route { origin: self.route_endpoint(st, oc), dest });
        }
        out
    }

    // ---------------------------------------------------------------- sites
    fn neighbours_with_districts(&self, st: &State, xy: XY) -> Vec<Plot> {
        let at: HashMap<XY, Sym> = st.cities.iter().flat_map(|c| c.districts.iter().map(|d| ((d.x, d.y), d.dtype))).collect();
        self.w.map.neighbours(xy.0, xy.1).into_iter().map(|n| { let mut p = self.w.map.plots[&n].clone(); p.district_type = at.get(&n).copied(); p }).collect()
    }

    /// near_fn's lookups: where districts stand and the improvements we built (built once per question).
    fn near_maps(&self, st: &State) -> NearMaps {
        NearMaps {
            at: st.cities.iter().flat_map(|c| c.districts.iter().map(|d| ((d.x, d.y), d.dtype))).collect(),
            imp: st.cities.iter().flat_map(|c| c.owned.iter().map(|p| (p.xy(), p.improvement))).collect(),
        }
    }

    /// near_fn: neighbours with districts and the improvements we built.
    fn near(&self, st: &State, xy: XY) -> Vec<Plot> {
        let m = self.near_maps(st);
        self.near_with(&m, xy)
    }

    fn near_with(&self, m: &NearMaps, xy: XY) -> Vec<Plot> {
        let (at, imp) = (&m.at, &m.imp);
        self.w.map.neighbours(xy.0, xy.1).into_iter().map(|n| {
            let mut p = self.w.map.plots[&n].clone();
            p.district_type = at.get(&n).copied();
            if let Some(i) = imp.get(&n) {
                p.improvement = *i;
            }
            p
        }).collect()
    }

    fn taken(&self, st: &State, ci: usize) -> HashSet<XY> {
        let mut t: HashSet<XY> = st.cities.iter().flat_map(|c| c.districts.iter().map(|d| (d.x, d.y))).collect();
        t.insert(st.cities[ci].center);
        t
    }

    #[allow(clippy::too_many_arguments)]
    fn site_possible_with(&self, st: &State, ci: usize, item: Sym, wonder: bool, e: &Engine, nm: &NearMaps, taken: &HashSet<XY>) -> bool {
        let _g = crate::prof::start(9);
        let city = &st.cities[ci];
        let lake = |xy: XY| self.w.map.is_lake(xy);
        city.owned.iter().any(|p| {
            self.w.map.distance(p.xy(), city.center) <= 3 && {
                let nb = self.near_with(&nm, p.xy());
                if wonder { wonder_ok(self.r, &e, item, p, &nb, city, &taken, &lake) } else { district_ok(self.r, &e, item, p, &nb, city, &taken, &lake) }
            }
        })
    }

    fn district_site(&self, st: &State, ci: usize, dist: Sym) -> Option<(f64, XY, District)> {
        let city = &st.cities[ci];
        let taken = self.taken(st, ci);
        let e = self.eng(st);
        let lake = |xy: XY| self.w.map.is_lake(xy);
        let mut best: Option<(f64, XY, District)> = None;
        let extra = modifier_adjacency_rows(&e, city.name, dist);
        let excl = excluded_adjacencies(self.r, &st.player);
        let nm = self.near_maps(st);
        for p in &city.owned {
            let xy = p.xy();
            if self.w.map.distance(xy, city.center) > 3 || !district_ok(self.r, &e, dist, p, &self.near_with(&nm, xy), city, &taken, &lake) {
                continue;
            }
            let d = District { dtype: dist, x: xy.0, y: xy.1, complete: true, plot: p.clone(), neighbours: self.neighbours_with_districts(st, xy) };
            let adj: f64 = district_adjacency(self.r, &d, &st.acc.techs, &st.acc.civics, None, &extra, &excl).iter().map(|(_, v)| v).sum();
            if best.as_ref().map(|b| adj > b.0).unwrap_or(true) {
                best = Some((adj, xy, d));
            }
        }
        best
    }

    fn w_value(&self, p: &Plot, st: &State, w: &Weights) -> f64 {
        weighted(w, &tile_yields(self.r, p, &st.acc.techs, &st.acc.civics, false, &[], None, None, None))
    }

    fn wonder_site(&self, st: &State, ci: usize, wonder: Sym) -> Option<(f64, XY)> {
        let city = &st.cities[ci];
        let taken = self.taken(st, ci);
        let e = self.eng(st);
        let lake = |xy: XY| self.w.map.is_lake(xy);
        let mut best: Option<(f64, XY)> = None;
        let nm = self.near_maps(st);
        for p in &city.owned {
            let xy = p.xy();
            if self.w.map.distance(xy, city.center) > 3 || !wonder_ok(self.r, &e, wonder, p, &self.near_with(&nm, xy), city, &taken, &lake) {
                continue;
            }
            let v = self.w_value(p, st, &DEFAULT_WEIGHTS);
            if best.map(|b| v < b.0).unwrap_or(true) {
                best = Some((v, xy));
            }
        }
        best
    }

    fn settle_sites(&self, st: &State, origin: XY) -> Vec<Choice> {
        let _g = crate::prof::start(7);
        let taken: Vec<XY> = st.settlers.iter().filter_map(|s| s.target).collect();
        // no site within 3 of another player's city (the game's rule), recorded cities included once they exist
        let foreign = self.sc.foreign_centers(st.turn, st.player.player_id);
        let blocked = |xy: XY| taken.contains(&xy) || foreign.iter().any(|&c| self.w.map.distance(xy, c) < 4);
        let mut sc: Vec<(f64, XY)> = vec![];
        for xy in self.w.map.within(origin, self.s.settle_radius) {
            if blocked(xy) {
                continue;
            }
            let v = self.w.site_score(xy, &st.cities, &st.acc.techs, &st.acc.civics);
            if v > 0.0 {
                sc.push((v - 0.7 * self.w.map.distance(origin, xy) as f64, xy));
            }
        }
        sc.sort_by(|a, b| b.partial_cmp(a).unwrap());
        let mut out: Vec<XY> = vec![];
        for (_, xy) in sc {
            if out.iter().all(|&y| self.w.map.distance(xy, y) >= 3) {
                out.push(xy);
            }
            if out.len() >= self.s.settle_candidates {
                break;
            }
        }
        for &xy in &self.s.extra_sites {
            if !out.contains(&xy) && !blocked(xy) && self.w.site_score(xy, &st.cities, &st.acc.techs, &st.acc.civics) > 0.0 {
                out.push(xy);
            }
        }
        out.into_iter().map(|(x, y)| Choice::Xy(x, y)).collect()
    }

    // ---------------------------------------------------------------- builders
    fn plantable(&self, e: &Engine, p: &Plot, civics: &BTreeSet<Sym>) -> Vec<Sym> {
        let r = self.r;
        let early: Vec<(Sym, Sym)> = e.player_effects(s!("EFFECT_ADJUST_FEATURE_PREREQ")).iter().filter_map(|x| Some((x.args.get("FeatureType")?, x.args.get("CivicType")?))).collect();
        let t = r.tab("Features");
        let mut out = vec![];
        for i in r.scan("PlantableFeatures", "Features") {
            let Some(civic) = t.v(i, "AddCivic").sym().filter(|x| !x.is_empty()) else { continue };
            let f = t.v(i, "FeatureType").sym().unwrap();
            let e_c = early.iter().rev().find(|(ff, _)| *ff == f).map(|(_, c)| *c);
            if civics.contains(&civic) || e_c.map(|c| civics.contains(&c)).unwrap_or(false) {
                let vt = r.tab("Feature_ValidTerrains");
                if vt.find("FeatureType", f).iter().any(|&j| vt.v(j, "TerrainType").sym() == Some(p.terrain)) {
                    out.push(f);
                }
            }
        }
        out
    }

    /// Chopped (harvested) production goes into the current item with the item's production percent (policies,
    /// buildings); one it finishes completes and carries over only the plain amount beyond what it lacked
    /// (CivFanatics, "Is chopping overflow overcorrected?").
    fn chop_into(&self, st: &mut State, ci: usize, amount: f64, env: &Env) {
        let c = &st.cities[ci];
        let n = c.name;
        let item = c.current;
        let ca = st.acc.cities.get(&n).cloned().unwrap_or_default();
        let Some(item) = item.filter(|_| amount > 0.0 && ca.chopped.is_none()) else {
            st.cacc_mut(n).raw += amount;
            return;
        };
        let e = self.eng(st);
        let pct = item_production_percent(self.r, &e, c, item);
        let Some(cost) = crate::sim::current_cost(self.r, &st.acc, &st.cities, env, n, item, self.speed) else {
            st.cacc_mut(n).raw += amount;
            return;
        };
        let ca = st.cacc_mut(n);
        let prog = ca.progress.get(&item).copied().unwrap_or(0.0) + ca.overflow;
        ca.overflow = 0.0;
        let lack = cost - prog;
        let boosted = amount * (1.0 + pct / 100.0);
        if boosted >= lack {
            ca.progress.insert(item, cost + (amount - lack).max(0.0));
            ca.chopped = Some(item);
        } else {
            ca.progress.insert(item, prog + boosted);
        }
    }

    /// What the player could buy now: items with gold or faith, the district a city is making, the best plots
    /// next to its borders (and, for the recorded player's play, the plots the recorded city got this turn).
    fn buy_options(&self, st: &State, env: &Env, t_now: i64) -> Vec<Choice> {
        let r = self.r;
        let mut buys = vec![Choice::Nothing];
        let e = self.engine(env, &st.cities);
        let mut env_cost = env.clone();
        env_cost.units_trained = st.acc.units_trained.clone();
        for ci in 0..st.cities.len() {
            let c = st.cities[ci].clone();
            for item in self.buildable_for(st, ci, true).into_iter().filter_map(|x| x.sym()) {
                if item_kind(item) == Kind::District {
                    continue;
                }
                let currencies = purchase_yields(r, &e, &c, item);
                if currencies.is_empty() {
                    continue;
                }
                let Some(cost) = item_cost(r, &env_cost, item, self.speed, None, true) else { continue };
                if currencies.contains(&s!("YIELD_GOLD")) {
                    let gold = gold_price(cost, purchase_discount_percent(r, &e, &c, item, true));
                    if 0.0 < gold && gold <= st.acc.gold {
                        buys.push(Choice::BuyItem(c.name, item, gold.to_bits(), s!("YIELD_GOLD")));
                    }
                }
                if currencies.contains(&s!("YIELD_FAITH")) {
                    let faith = faith_price(cost, faith_discount_percent(r, &e, &c, item));
                    if 0.0 < faith && faith <= st.acc.faith {
                        buys.push(Choice::BuyItem(c.name, item, faith.to_bits(), s!("YIELD_FAITH")));
                    }
                }
            }
            if let Some(cur) = c.current {
                if item_kind(cur) == Kind::District {
                    for cy in purchase_yields(r, &e, &c, cur) {
                        let Some(cost) = item_cost(r, &env_cost, cur, self.speed, None, false) else { continue };
                        if cy == s!("YIELD_GOLD") {
                            let price = gold_price(cost, purchase_discount_percent(r, &e, &c, cur, true));
                            if 0.0 < price && price <= st.acc.gold {
                                buys.push(Choice::BuyItem(c.name, cur, price.to_bits(), s!("YIELD_GOLD")));
                            }
                        } else {
                            let price = faith_price(cost, faith_discount_percent(r, &e, &c, cur));
                            if 0.0 < price && price <= st.acc.faith {
                                buys.push(Choice::BuyItem(c.name, cur, price.to_bits(), s!("YIELD_FAITH")));
                            }
                        }
                    }
                }
            }
        }
        for ci in 0..st.cities.len() {
            if self.s.recorded_tiles.as_ref().map(|m| m.contains_key(&st.cities[ci].center)).unwrap_or(false) {
                continue; // its tiles come from the recorded game
            }
            // the three best plots the city could take next
            let best = self.w.border_tiles(ci, &st.cities, &st.acc.techs, &st.acc.civics, &DEFAULT_WEIGHTS, 3);
            for &xy in &best {
                let gold = tile_price(r, &e, &st.cities[ci], &self.w.map.plots[&xy], self.speed, st.acc.techs.len(), st.acc.civics.len());
                if gold <= st.acc.gold {
                    buys.push(Choice::BuyTile(st.cities[ci].name, gold.to_bits(), xy.0, xy.1));
                }
            }
            // the recorded player's play: the plots the recorded city got this turn can be bought too
            if let Some(rt) = self.tiles_rec.as_ref().and_then(|r| r.tiles.get(&st.cities[ci].center)) {
                let owned: HashSet<XY> = st.cities.iter().flat_map(|c| c.owned.iter().map(|p| p.xy())).collect();
                for &(t, x) in rt {
                    if t != t_now || best.contains(&x) || owned.contains(&x) || self.w.map.distance(st.cities[ci].center, x) > 3
                        || !self.w.map.neighbours(x.0, x.1).iter().any(|n| st.cities[ci].owned.iter().any(|p| p.xy() == *n)) {
                        continue;
                    }
                    let Some(pl) = self.w.map.plots.get(&x) else { continue };
                    let g = tile_price(r, &e, &st.cities[ci], pl, self.speed, st.acc.techs.len(), st.acc.civics.len());
                    if g <= st.acc.gold {
                        buys.push(Choice::BuyTile(st.cities[ci].name, g.to_bits(), x.0, x.1));
                    }
                }
            }
        }
        buys
    }

    fn rush_percent(&self, e: &Engine, item: Option<Sym>) -> f64 {
        let Some(item) = item else { return 0.0 };
        let eff = match item_kind(item) {
            Kind::District => "EFFECT_ADJUST_PLAYER_UNIT_DISTRICT_PERCENT",
            Kind::Building if self.r.is_wonder(item) => "EFFECT_ADJUST_PLAYER_UNIT_WONDER_PERCENT",
            Kind::Project => "EFFECT_ADJUST_PLAYER_UNIT_PROJECT_PERCENT",
            _ => return 0.0,
        };
        e.player_effects(crate::sym::sym_static(eff)).iter().map(|x| x.args.f("Amount")).sum()
    }

    /// [(kind, yields)] a builder could harvest on p: "feature" (chop) or "resource".
    fn harvest_yields(&self, e: &Engine, p: &Plot, st: &State, city: Option<Sym>) -> Vec<(Sym, Vec<(Sym, f64)>)> {
        let r = self.r;
        let mut out = vec![];
        let techs = &st.acc.techs;
        if let Some(f) = p.feature {
            let tech_row = r.lookup("Features", "FeatureType", f, "RemoveTech");
            let rows = r.tab("Feature_Removes").find("FeatureType", f);
            if !rows.is_empty() && tech_row.is_some() {
                let tech = tech_row.unwrap().sym().filter(|x| !x.is_empty());
                if tech.map(|t| techs.contains(&t)).unwrap_or(true) && p.improvement.is_none() {
                    out.push((s!("feature"), harvest_yield(r, Some(&e), true, f, techs.len(), st.acc.civics.len(), city, self.speed)));
                }
            }
        }
        let no_res = !e.player_effects(s!("EFFECT_ADJUST_PLAYER_PREVENT_HARVEST_RESOURCE")).is_empty();
        if let Some(res) = p.resource {
            if !no_res {
                let t = r.tab("Resource_Harvests");
                let rows = t.find("ResourceType", res);
                if !rows.is_empty() && rows.iter().all(|&i| t.v(i, "PrereqTech").sym().filter(|x| !x.is_empty()).map(|x| techs.contains(&x)).unwrap_or(true)) && p.improvement.is_none() {
                    out.push((s!("resource"), harvest_yield(r, Some(&e), false, res, techs.len(), st.acc.civics.len(), city, self.speed)));
                }
            }
        }
        out
    }

    /// The builder's candidate actions, best first by their return over the rest of the horizon (Settings.horizon,
    /// the last turn counted): an improvement (or planted feature) gives its gain in weighted full yields (the
    /// city's focus) every turn from the builder's arrival (2 tiles a turn from `at`), counted on a worked tile, or
    /// beyond the weakest worked tile on one not worked; a harvest or a rush gives its lump once (x 0.7, less with
    /// under 10 turns left). The best builder_candidates, the recorded game's actions (extra_builder), then wait.
    fn builder_options(&self, st: &State, env: &Env, at: XY) -> Vec<Choice> {
        let _g = crate::prof::start(4);
        let r = self.r;
        let e = self.engine(env, &st.cities);
        let nm = self.near_maps(st);
        let (t_now, end) = (st.turn, self.s.horizon);
        let left = |xy: XY| (end - (t_now + (self.w.map.distance(at, xy) as i64 + 1).div_euclid(2))).max(0);
        let lump = |v: f64, n: i64| 0.7 * v * (n as f64 / 10.0).min(1.0);
        let mut opts: Vec<(f64, Choice)> = vec![];
        for c in &st.cities {
            let worked: Vec<XY> = c.worked.iter().map(|w| w.xy()).collect();
            let dtiles: Vec<XY> = c.districts.iter().map(|d| (d.x, d.y)).collect();
            let w = focus_weights(*st.focus.get(&c.name).unwrap_or(&s!("balanced")));
            let mods: &[crate::engine::PlotMod] = e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]);
            let amods = crate::yields::appeal_modifiers(&e, c.name);
            let wv = |p: &Plot| weighted(&w, &tile_yields(r, p, &st.acc.techs, &st.acc.civics, false, mods, Some(&e.plots), Some(c), Some(&amods)));
            let wk: Vec<f64> = c.owned.iter().filter(|p| worked.contains(&p.xy()) && p.xy() != c.center).map(&wv).collect();
            let vmin = if wk.is_empty() { 0.0 } else { wk.iter().cloned().fold(f64::INFINITY, f64::min) };
            let gain = |xy: XY, before: f64, after: f64| if worked.contains(&xy) { after - before } else { (after - vmin).max(0.0) };
            for p in &c.owned {
                let xy = p.xy();
                if xy == c.center || dtiles.contains(&xy) || (!self.w.map.is_land(xy) && p.resource.is_none()) {
                    continue;
                }
                let n = left(xy);
                if p.improvement.is_none() {
                    let base = wv(p);
                    let lake = |q: XY| self.w.map.is_lake(q);
                    for imp in improvement_options(r, &e, p, &self.near_with(&nm, xy), &st.acc.techs, &st.acc.civics, Some(c), &lake) {
                        let mut q = p.clone();
                        q.improvement = Some(imp);
                        opts.push((gain(xy, base, wv(&q)) * n as f64, Choice::Act(s!("improve"), c.name, xy.0, xy.1, imp)));
                    }
                }
                if p.feature.is_none() && p.improvement.is_none() && p.resource.is_none() {
                    for feat in self.plantable(&e, p, &st.acc.civics) {
                        let mut q = p.clone();
                        q.feature = Some(feat);
                        opts.push((gain(xy, wv(p), wv(&q)) * n as f64, Choice::Act(s!("plant"), c.name, xy.0, xy.1, feat)));
                    }
                }
                for (kind, ys) in self.harvest_yields(&e, p, st, Some(c.name)).into_iter().filter(|_| st.turn >= self.s.no_harvest_before) {
                    let total: f64 = ys.iter().map(|(_, v)| v).sum();
                    opts.push((lump(total, n), Choice::Act(s!("harvest"), c.name, xy.0, xy.1, kind)));
                }
            }
        }
        for c in &st.cities {
            let pct = self.rush_percent(&e, c.current);
            if pct != 0.0 {
                let mut env_cost = env.clone();
                env_cost.units_trained = st.acc.units_trained.clone();
                let cost = item_cost(r, &env_cost, c.current.unwrap(), self.speed, None, false).unwrap_or(0.0);
                opts.push((lump(cost * pct / 100.0, left(c.center)), Choice::Act(s!("rush"), c.name, c.center.0, c.center.1, c.current.unwrap())));
            }
        }
        opts.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        let keep: Vec<Choice> = match self.s.builder_candidates {
            None => opts.iter().map(|o| o.1.clone()).collect(),
            Some(n) => opts.iter().take(n).map(|o| o.1.clone()).collect(),
        };
        let mut out = keep;
        for (_, o) in &opts {
            if self.s.extra_builder.contains(o) && !out.contains(o) {
                out.push(o.clone());
            }
        }
        out.push(Choice::Wait);
        out
    }

    /// What a waiting builder waits for: a change of techs, civics, cities, tiles (and their improvements, features
    /// and resources), population or items.
    fn idle_mark(&self, st: &State) -> IdleMark {
        let owned = || st.cities.iter().flat_map(|c| c.owned.iter());
        ([st.acc.techs.len(), st.acc.civics.len(), st.cities.len(), owned().count(), owned().filter(|p| p.improvement.is_some()).count(),
          owned().filter(|p| p.feature.is_some()).count(), owned().filter(|p| p.resource.is_some()).count()],
         st.cities.iter().map(|c| st.cacc(c.name).population as i64).sum(), st.cities.iter().map(|c| c.current).collect())
    }

    fn road_type(&self, turn: i64) -> Sym {
        let r = self.r;
        let now = r.era_index(self.sc.game_era(turn)).unwrap_or(0);
        let t = r.tab("Routes");
        let mut rows: Vec<(i64, Sym, Option<Sym>)> = (0..t.len() as u32).map(|i| (t.v(i, "PlacementValue").i(), t.v(i, "RouteType").sym().unwrap(), t.v(i, "PrereqEra").sym().filter(|x| !x.is_empty()))).collect();
        rows.sort_by_key(|x| x.0);
        let mut best = s!("ROUTE_ANCIENT_ROAD");
        for (_, rt, era) in rows {
            if rt == s!("ROUTE_RAILROAD") {
                continue;
            }
            if era.map(|e| r.era_index(e).map(|i| i <= now).unwrap_or(false)).unwrap_or(true) {
                best = rt;
            }
        }
        best
    }

    pub fn travel(&self, st: &State, a: XY, b: XY, unit: &str) -> Option<i64> {
        let e = self.eng(st);
        let extra: f64 = e.unit_effects(s!("EFFECT_ADJUST_UNIT_MOVEMENT"), ss(unit), None).iter().map(|x| x.args.f("Amount")).sum();
        let moves = self.r.lookup_f("Units", "UnitType", ss(unit), "BaseMoves") + extra;
        let mut cities: HashSet<XY> = st.cities.iter().map(|c| c.center).collect();
        cities.extend(self.sc.world_cities(st.turn).iter().map(|c| (c.x, c.y)));
        travel_turns(self.r, &self.w.map, a, b, moves, self.road_type(st.turn), &cities)
    }

    // ---------------------------------------------------------------- great people
    fn gp_cost(&self, st: &State, cls: Sym, ind: Sym) -> f64 {
        let r = self.r;
        let era = r.lookup_sym("GreatPersonIndividuals", "GreatPersonIndividualType", ind, "EraType").unwrap();
        let base = r.lookup_f("Eras", "EraType", era, "GreatPersonBaseCost") * r.game_speed_cost_multiplier(self.speed);
        let same_era_before = self.sc.gp_list(cls).iter().any(|(i, _, taken)| *i != ind
            && (st.gp_ours.contains(i) || taken.map(|t| t <= st.turn).unwrap_or(false))
            && r.lookup_sym("GreatPersonIndividuals", "GreatPersonIndividualType", *i, "EraType") == Some(era));
        if !same_era_before && r.era_index(self.sc.game_era(st.turn)).unwrap_or(0) < r.era_index(era).unwrap_or(0) {
            return (base * 1.3 / 5.0).floor() * 5.0;
        }
        base
    }

    fn gp_place(&self, st: &State, ind: Sym) -> Option<Sym> {
        let r = self.r;
        let need = r.lookup_sym("GreatPersonIndividuals", "GreatPersonIndividualType", ind, "ActionRequiresCompletedDistrictType");
        let targets = r.list_sym("GreatPersonIndividualActionModifiers", "GreatPersonIndividualType", ind, "AttachmentTargetType");
        let wonder_target = targets.iter().any(|t| t.contains("TARGET_DISTRICT_WONDER_IN_TILE"));
        for c in &st.cities {
            if let Some(n) = need {
                if !c.districts.iter().any(|d| r.district_is(d.dtype, n) && d.complete) {
                    continue;
                }
            }
            if wonder_target && !c.current.map(|x| r.is_wonder(x)).unwrap_or(false) {
                continue;
            }
            return Some(c.name);
        }
        None
    }

    fn great_people(&self, st: &mut State, ch: &mut dyn Chooser) {
        let r = self.r;
        let e = self.eng(st);
        for (cls, pts) in crate::gp::great_person_points(r, &e) {
            *st.gp.entry_or(cls, 0.0) += pts;
        }
        for cls in GP_CLASSES {
            let cls = ss(cls);
            let Some(ind) = self.sc.gp_offer(cls, st.turn, &st.gp_ours) else { continue };
            let cost = self.gp_cost(st, cls, ind);
            // replay: recruited on the turn the record shows it (its points can run a little behind)
            let recorded = self.rec.as_ref().or(self.tiles_rec.as_ref()).and_then(|r| r.turn(st.turn + 1)).map(|r| r.great_people.contains(&ind)).unwrap_or(false);
            if st.gp.get(&cls).copied().unwrap_or(0.0) < cost && !recorded {
                continue;
            }
            let pick = self.decide(st, "gp", Ent::S(cls), vec![Choice::S(ind), Choice::S(s!("wait"))], ch);
            if pick == Some(Choice::S(ind)) {
                *st.gp.entry_or(cls, 0.0) -= cost;
                st.gp_ours.push(ind);
                st.gp_pending.push(GpUse { individual: ind, cls, turn: st.turn, city: None });
            }
        }
        let pending = st.gp_pending.clone();
        for g in pending {
            let Some(city) = self.gp_place(st, g.individual) else { continue };
            st.gp_pending.retain(|x| x != &g);
            let n = (r.lookup_f("GreatPersonIndividuals", "GreatPersonIndividualType", g.individual, "ActionCharges") as i64).max(1);
            st.player.great_people_used.push(GpUse { city: Some(city), ..g.clone() });
            for _ in 0..n {
                let env = self.player_env(st);
                self.apply_grants(st, "GreatPersonIndividualActionModifiers", "GreatPersonIndividualType", g.individual, Some(city), &env, ch);
            }
        }
    }

    // ---------------------------------------------------------------- one-time grants
    fn apply_grants(&self, st: &mut State, table: &str, col: &str, key: Sym, city: Option<Sym>, env: &Env, ch: &mut dyn Chooser) {
        let _g = crate::prof::start(10);
        let r = self.r;
        let owner = match city { Some(c) => Subj::City(c), None => Subj::Player };
        let grants = {
            let mut e = Engine::bare(r, env, &st.cities);
            e.grants_of(table, col, key, owner)
        };
        let era = self.sc.game_era(st.turn + 1);
        for (eff, subj, a) in grants {
            let e_s = eff.as_string();
            if !GRANT_EFFECTS.contains(&e_s.as_str()) {
                continue;
            }
            let cname = match &subj {
                Subj::City(n) | Subj::District(n, _, _) | Subj::Plot(n, _, _) => *n,
                _ => city.unwrap_or(st.cities[0].name),
            };
            let ci = st.cities.iter().position(|x| x.name == cname).unwrap_or(0);
            let cname = st.cities[ci].name;
            let n = a.get("Amount").filter(|x| !x.is_empty()).map(|x| x.with(|s| s.parse::<f64>().unwrap_or(1.0)) as i64).unwrap_or(1);
            match e_s.as_str() {
                "EFFECT_GRANT_UNIT_IN_CITY" => {
                    let u = a.get("UnitType");
                    for _ in 0..n {
                        if u == Some(s!("UNIT_BUILDER")) {
                            let eng = self.engine(env, &st.cities);
                            let charges = builder_charges(r, &eng, s!("UNIT_BUILDER"), cname);
                            st.builders.push(Builder { charges, at: st.cities[ci].center, ready: st.turn + 1, id: None, pending: None, idle: None });
                        } else if u == Some(s!("UNIT_TRADER")) {
                            let t = self.start_route(st, cname, ch);
                            st.traders.push(t);
                        } else if let Some(u) = u.filter(|x| !x.is_empty()) {
                            *st.units.entry_or(u, 0) += 1;
                        }
                    }
                }
                "EFFECT_ADJUST_CITY_POPULATION" => {
                    st.cacc_mut(cname).population += n as i32;
                }
                "EFFECT_GRANT_PLAYER_SPECIFIC_TECHNOLOGY" => {
                    if let Some(t) = a.get("TechType") {
                        st.acc.techs.insert(t);
                    }
                }
                "EFFECT_GRANT_PLAYER_SPECIFIC_TECH_BOOST_GREAT_PERSON" => {
                    if let Some(t) = a.get("TechType") {
                        if !st.acc.techs.contains(&t) {
                            if st.player.boosted.contains(&t) {
                                st.acc.techs.insert(t);
                            } else {
                                let amt = boost_amount(research_cost(r, t, self.speed, era), 0.0);
                                *st.acc.tech_progress.entry_or(t, 0.0) += amt;
                                st.player.boosted.insert(t);
                            }
                        }
                    }
                }
                "EFFECT_GRANT_PLAYER_RANDOM_TECHNOLOGY" => {
                    let mut av: Vec<Sym> = self.available_techs(st).iter().filter_map(|c| c.sym()).collect();
                    av.sort_by(|x, y| research_cost(r, *x, self.speed, era).partial_cmp(&research_cost(r, *y, self.speed, era)).unwrap());
                    for t in av.into_iter().take(n as usize) {
                        st.acc.techs.insert(t);
                    }
                }
                "EFFECT_GRANT_PLAYER_RANDOM_CIVIC" => {
                    let mut av: Vec<Sym> = self.available_civics(st).iter().filter_map(|c| c.sym()).collect();
                    av.sort_by(|x, y| research_cost(r, *x, self.speed, era).partial_cmp(&research_cost(r, *y, self.speed, era)).unwrap());
                    for t in av.into_iter().take(n as usize) {
                        st.acc.civics.insert(t);
                    }
                }
                "EFFECT_ADJUST_MULTIPLY_TREASURY" => {
                    st.acc.gold *= 1.0 + a.f("Amount") / 100.0;
                }
                "EFFECT_GRANT_ALL_TECHNOLOGY_BOOST_BY_ERA" | "EFFECT_GRANT_RANDOM_TECHNOLOGY_BOOST_BY_ERA" | "EFFECT_GRANT_RANDOM_CIVIC_BOOST_BY_ERA" => {
                    let civic = e_s.contains("CIVIC");
                    let (tab, c2) = if civic { ("Civics", "CivicType") } else { ("Technologies", "TechnologyType") };
                    let lo = a.get("StartEraType").and_then(|x| r.era_index(x)).unwrap_or(0);
                    let hi = a.get("EndEraType").and_then(|x| r.era_index(x)).unwrap_or(0);
                    let t = r.tab(tab);
                    let done = if civic { &st.acc.civics } else { &st.acc.techs };
                    let mut items: Vec<Sym> = vec![];
                    // join with Eras on EraType where ChronologyIndex between lo and hi: in the table's order
                    for i in 0..t.len() as u32 {
                        let it = t.v(i, c2).sym().unwrap();
                        let ei = t.v(i, "EraType").sym().and_then(|x| r.era_index(x));
                        if ei.map(|x| x >= lo && x <= hi).unwrap_or(false) && !done.contains(&it) && !st.player.boosted.contains(&it) && r.has_boost(it) {
                            items.push(it);
                        }
                    }
                    if e_s != "EFFECT_GRANT_ALL_TECHNOLOGY_BOOST_BY_ERA" {
                        items.sort_by(|x, y| research_cost(r, *x, self.speed, era).partial_cmp(&research_cost(r, *y, self.speed, era)).unwrap());
                        items.truncate(n as usize);
                    }
                    for it in items {
                        let amt = boost_amount(research_cost(r, it, self.speed, era), 0.0);
                        let prog = if civic { &mut st.acc.civic_progress } else { &mut st.acc.tech_progress };
                        *prog.entry_or(it, 0.0) += amt;
                        st.player.boosted.insert(it);
                    }
                }
                "EFFECT_GRANT_YIELD" => {
                    let amt = a.f("Amount") * if a.get("Scale") == Some(s!("1")) { r.game_speed_cost_multiplier(self.speed) } else { 1.0 };
                    if a.get("YieldType") == Some(s!("YIELD_GOLD")) {
                        st.acc.gold += amt;
                    } else if a.get("YieldType") == Some(s!("YIELD_FAITH")) {
                        st.acc.faith += amt;
                    }
                }
                "EFFECT_GRANT_BUILDING_IN_CITY_IGNORE" => {
                    if let Some(b) = a.get("BuildingType") {
                        if !st.cities[ci].buildings.contains(&b) {
                            st.cities[ci].buildings.push(b);
                        }
                    }
                }
                "EFFECT_GRANT_CHEAPEST_BUILDING_IN_CITY" => {
                    let opts: Vec<Sym> = self.buildable(st, ci).iter().filter_map(|c| c.sym()).filter(|b| b.starts_with("BUILDING_") && !r.is_wonder(*b)).collect();
                    if let Some(b) = opts.into_iter().min_by(|x, y| r.cost(*x).partial_cmp(&r.cost(*y)).unwrap()) {
                        st.cities[ci].buildings.push(b);
                    }
                }
                "EFFECT_GRANT_PRODUCTION_IN_CITY" => {
                    if let Some(cur) = st.cities[ci].current {
                        *st.cacc_mut(cname).progress.entry_or(cur, 0.0) += a.f("Amount");
                    }
                }
                _ => {}
            }
        }
    }

    fn boost_cheapest(&self, st: &mut State, era_type: Sym, civic: bool) {
        let r = self.r;
        let (tab, col) = if civic { ("Civics", "CivicType") } else { ("Technologies", "TechnologyType") };
        let era = self.sc.game_era(st.turn + 1);
        let done = if civic { &st.acc.civics } else { &st.acc.techs };
        let items: Vec<Sym> = r.list_sym(tab, "EraType", era_type, col).into_iter().filter(|t| !done.contains(t) && !st.player.boosted.contains(t) && r.has_boost(*t)).collect();
        let Some(t) = items.into_iter().min_by(|x, y| research_cost(r, *x, self.speed, era).partial_cmp(&research_cost(r, *y, self.speed, era)).unwrap()) else { return };
        let amt = boost_amount(research_cost(r, t, self.speed, era), 0.0);
        let prog = if civic { &mut st.acc.civic_progress } else { &mut st.acc.tech_progress };
        *prog.entry_or(t, 0.0) += amt;
        st.player.boosted.insert(t);
    }

    fn on_completed(&self, st: &mut State, ci: usize, item: Sym, env: &Env) {
        let r = self.r;
        let e = self.engine(env, &st.cities);
        if item_kind(item) == Kind::District && r.district_requires_population(item) {
            let free = !e.player_effects(s!("EFFECT_ADJUST_PLAYER_FREE_BUILDING_WHEN_SPECIALTY_DISTRICT_CONSTRUCTED")).is_empty();
            let except: Vec<Sym> = e.player_effects(s!("EFFECT_ADJUST_PLAYER_FREE_BUILDING_WHEN_SPECIALTY_DISTRICT_CONSTRUCTED_EXCEPT")).iter().filter_map(|x| x.args.get("DistrictType")).collect();
            if free && !except.iter().any(|&d| r.district_is(item, d)) {
                let t = r.tab("Buildings");
                let first: Vec<Sym> = t.find("PrereqDistrict", item).iter().filter(|&&i| !t.v(i, "IsWonder").truthy()).map(|&i| t.v(i, "BuildingType").sym().unwrap())
                    .filter(|&b| r.tab("BuildingPrereqs").find("Building", b).is_empty() && !r.building_replaced(b, st.player.civilization, st.player.leader) && !st.cities[ci].buildings.contains(&b)).collect();
                if let Some(b) = first.into_iter().min_by(|x, y| r.cost(*x).partial_cmp(&r.cost(*y)).unwrap()) {
                    st.cities[ci].buildings.push(b);
                }
            }
        }
        if item_kind(item) == Kind::Building && r.is_wonder(item) {
            let era = r.lookup_sym("Buildings", "BuildingType", item, "PrereqTech").and_then(|t| r.lookup_sym("Technologies", "TechnologyType", t, "EraType"))
                .or_else(|| r.lookup_sym("Buildings", "BuildingType", item, "PrereqCivic").and_then(|t| r.lookup_sym("Civics", "CivicType", t, "EraType")));
            if let Some(era) = era {
                if !e.player_effects(s!("EFFECT_ADJUST_FREE_TECH_BOOST_WONDER_ERA")).is_empty() {
                    self.boost_cheapest(st, era, false);
                }
                if !e.player_effects(s!("EFFECT_ADJUST_FREE_CIVIC_BOOST_WONDER_ERA")).is_empty() {
                    self.boost_cheapest(st, era, true);
                }
            }
        }
    }

    pub fn endogenous_boosts(&self, st: &State) -> Vec<Sym> {
        let r = self.r;
        let mut improvements: Vec<(Sym, i64)> = vec![];
        let mut improved = 0;
        for c in &st.cities {
            for p in &c.owned {
                if let Some(i) = p.improvement {
                    if let Some(x) = improvements.iter_mut().find(|(k, _)| *k == i) { x.1 += 1 } else { improvements.push((i, 1)) }
                    improved += 1;
                }
            }
        }
        let pop: i64 = st.cities.iter().map(|c| st.cacc(c.name).population as i64).sum();
        let mut districts: Vec<(Sym, i64)> = vec![];
        for c in &st.cities {
            for x in &c.districts {
                if x.complete && r.district_requires_population(x.dtype) {
                    if let Some(d) = districts.iter_mut().find(|(k, _)| *k == x.dtype) { d.1 += 1 } else { districts.push((x.dtype, 1)) }
                }
            }
        }
        let buildings: HashSet<Sym> = st.cities.iter().flat_map(|c| c.buildings.iter().copied()).collect();
        let t = r.tab("Boosts");
        let s = |i: u32, c: &str| t.v(i, c).sym().filter(|x| !x.is_empty());
        let mut out = vec![];
        for i in 0..t.len() as u32 {
            let item = s(i, "TechnologyType").or(s(i, "CivicType"));
            let Some(item) = item else { continue };
            let cls = s(i, "BoostClass").unwrap_or(Sym(0));
            let n = { let x = t.v(i, "NumItems").i(); if x == 0 { 1 } else { x } };
            let get = |v: &Vec<(Sym, i64)>, k: Option<Sym>| k.and_then(|k| v.iter().find(|(a, _)| *a == k).map(|x| x.1)).unwrap_or(0);
            let ok = if (cls == s!("BOOST_TRIGGER_NUM_IMPROVED_TILES")) {
                improved >= n
            } else if (cls == s!("BOOST_TRIGGER_HAVE_X_IMPROVEMENTS")) {
                get(&improvements, s(i, "ImprovementType")) >= n
            } else if (cls == s!("BOOST_TRIGGER_EMPIRE_POPULATION")) {
                pop >= n
            } else if (cls == s!("BOOST_TRIGGER_OWN_X_UNITS_OF_TYPE")) {
                let u = s(i, "Unit1Type");
                u.map(|u| *st.units.get(&u).unwrap_or(&0) >= n || *st.acc.units_trained.get(&u).unwrap_or(&0) >= n).unwrap_or(false)
            } else if (cls == s!("BOOST_TRIGGER_HAVE_X_UNIQUE_SPECIALTY_DISTRICTS")) {
                districts.len() as i64 >= n
            } else if (cls == s!("BOOST_TRIGGER_HAVE_X_DISTRICTS")) {
                get(&districts, s(i, "DistrictType")) >= n
            } else if (cls == s!("BOOST_TRIGGER_CONSTRUCT_BUILDING")) {
                s(i, "BuildingType").map(|b| buildings.contains(&b)).unwrap_or(false)
            } else if (cls == s!("BOOST_TRIGGER_MAINTAIN_X_TRADE_ROUTES")) {
                st.traders.iter().filter(|t| t.dest.is_some()).count() as i64 >= n
            } else if (cls == s!("BOOST_TRIGGER_RESEARCH_TECH")) {
                s(i, "BoostingTechType").map(|t| st.acc.techs.contains(&t)).unwrap_or(false)
            } else if (cls == s!("BOOST_TRIGGER_CREATE_PANTHEON")) {
                st.player.pantheon.is_some()
            } else if (cls == s!("BOOST_TRIGGER_IMPROVE_SPECIFIC_RESOURCE")) {
                let (imp, res) = (s(i, "ImprovementType"), s(i, "ResourceType"));
                imp.is_some() && res.is_some() && r.resource_revealed(res.unwrap(), &st.acc.techs)
                    && st.cities.iter().any(|c| c.owned.iter().any(|p| p.improvement == imp && p.resource == res))
            } else if cls == s!("BOOST_TRIGGER_HAVE_X_BUILDINGS") {
                let b = s(i, "BuildingType");
                st.cities.iter().filter(|c| b.map(|b| c.buildings.contains(&b)).unwrap_or(false)).count() as i64 >= n
            } else if cls == s!("BOOST_TRIGGER_CULTURVATE_CIVIC") {
                s(i, "BoostingCivicType").map(|c| st.acc.civics.contains(&c)).unwrap_or(false)
            } else if cls == s!("BOOST_TRIGGER_TRAIN_UNIT") {
                // a great person's unit: one of its class recruited; any other unit: trained
                let u = s(i, "Unit1Type");
                let gp_class = u.and_then(|u| r.tab("GreatPersonClasses").first("UnitType", u).and_then(|j| r.tab("GreatPersonClasses").v(j, "GreatPersonClassType").sym()));
                match (u, gp_class) {
                    (_, Some(cls_)) => st.gp_ours.iter().any(|g| r.lookup_sym("GreatPersonIndividuals", "GreatPersonIndividualType", *g, "GreatPersonClassType") == Some(cls_)),
                    (Some(u), None) => *st.acc.units_trained.get(&u).unwrap_or(&0) >= 1,
                    _ => false,
                }
            } else if cls == s!("BOOST_TRIGGER_HAVE_GOVERNMENT_TIER") {
                // Castles: "a government with 6 policy slots" (NumItems); without a count: a government of the tier
                let tier_of = |x: Option<Sym>| x.and_then(|x| r.lookup("GovernmentTiers", "TierType", x, "Sorting").map(|v| v.i()));
                let mine = tier_of(st.player.government.and_then(|g| r.lookup_sym("Governments", "GovernmentType", g, "Tier")));
                if t.v(i, "NumItems").i() > 0 {
                    st.player.government.map(|g| self.slots(g, st).len() as i64 >= n).unwrap_or(false)
                } else {
                    match (mine, tier_of(s(i, "GovernmentTierType"))) { (Some(a), Some(b)) => a >= b, _ => false }
                }
            } else if cls == s!("BOOST_TRIGGER_CITY_POPULATION") {
                st.cities.iter().any(|c| st.cacc(c.name).population as i64 >= n)
            } else if cls == s!("BOOST_TRIGGER_SETTLE_COAST") {
                st.cities.iter().any(|c| self.w.map.neighbours(c.center.0, c.center.1).iter()
                    .any(|xy| self.w.map.plots.get(xy).map(|p| p.terrain == s!("TERRAIN_COAST") && !self.w.map.is_lake(*xy)).unwrap_or(false)))
            } else if cls == s!("BOOST_TRIGGER_HAVE_X_WONDERS") {
                buildings.iter().filter(|b| r.is_wonder(**b)).count() as i64 >= n
            } else if cls == s!("BOOST_TRIGGER_HAVE_X_GREAT_PEOPLE") {
                st.gp_ours.len() as i64 >= n
            } else {
                false
            };
            if ok {
                out.push(item);
            }
        }
        out
    }

    fn found_city(&self, st: &State, name: Sym, site: XY, env: &Env) -> City {
        let mine = World::owned_by_us(&st.cities);
        let mut owned = vec![self.w.map.plots[&site].clone()];
        for n in self.w.map.neighbours(site.0, site.1) {
            if !mine.contains(&n) && !self.w.foreign.contains(&n) {
                owned.push(self.w.map.plots[&n].clone());
            }
        }
        let extra: i64 = if st.cities.is_empty() { 0 } else {
            self.engine(env, &st.cities).player_effects(s!("EFFECT_ADJUST_PLAYER_CITY_TILES")).iter().map(|x| x.args.f("Amount") as i64).sum()
        };
        if extra > 0 {
            let have: Vec<XY> = owned.iter().map(|p| p.xy()).collect();
            let mut cand: Vec<XY> = self.w.map.within(site, crate::world::WORKABLE_RADIUS).into_iter()
                .filter(|xy| !have.contains(xy) && !mine.contains(xy) && !self.w.foreign.contains(xy) && self.w.map.distance(*xy, site) == 2).collect();
            cand.sort_by(|a, b| plot_value(self.r, &self.w.map.plots[b], &env.techs, &env.civics, &DEFAULT_WEIGHTS)
                .partial_cmp(&plot_value(self.r, &self.w.map.plots[a], &env.techs, &env.civics, &DEFAULT_WEIGHTS)).unwrap());
            for xy in cand.into_iter().take(extra as usize) {
                owned.push(self.w.map.plots[&xy].clone());
            }
        }
        if let Some(first) = self.rec.as_ref().and_then(|r| r.first_owned.get(&site)) {
            // replay: the tiles the recorded city had on its first turn
            owned = first.iter().filter(|xy| !mine.contains(xy) && self.w.map.plots.contains_key(xy)).map(|xy| self.w.map.plots[xy].clone()).collect();
        }
        // founding a city removes a removable feature (woods, rainforest, marsh) from its centre
        let mut centre = self.w.map.plots[&site].clone();
        if centre.feature.is_some_and(|f| self.r.lookup_truthy("Features", "FeatureType", f, "Removable")) {
            centre.feature = None;
        }
        for p in owned.iter_mut().filter(|p| p.xy() == site) {
            *p = centre.clone();
        }
        let n = st.cities.len() as i64;
        let mut city = City {
            name,
            id: (n + 1) << 16 | n,
            capital: false,
            center: site,
            population: 1,
            religion: None,
            worked: vec![],
            owned,
            near: vec![],
            current: None,
            buildings: vec![],
            great_works: vec![],
            districts: vec![District { dtype: s!("DISTRICT_CITY_CENTER"), x: site.0, y: site.1, complete: true, plot: centre, neighbours: vec![] }],
            pop_mark: None,
        };
        self.refresh_near(&mut city);
        let e = self.engine(env, std::slice::from_ref(&city));
        self.w.assign_citizens(&mut city, &e, &DEFAULT_WEIGHTS, 1.0);
        city
    }

    /// The plots next to the city's territory (the save's "near"): yields that look past the border (a natural
    /// wonder's adjacency yields, fresh water) see them.
    fn refresh_near(&self, c: &mut City) {
        let own: HashSet<XY> = c.owned.iter().map(|p| p.xy()).collect();
        let mut seen: HashSet<XY> = HashSet::new();
        let mut near = vec![];
        for p in &c.owned {
            for n in self.w.map.neighbours(p.x, p.y) {
                if !own.contains(&n) && seen.insert(n) {
                    if let Some(q) = self.w.map.plots.get(&n) {
                        near.push(q.clone());
                    }
                }
            }
        }
        c.near = near;
    }

    fn assign(&self, st: &mut State, ci: usize, env: &Env) {
        let rec = self.rec.as_ref().or(self.tiles_rec.as_ref());
        let name = st.cities[ci].name;
        let rec_now = rec.and_then(|r| r.turn(st.turn)).and_then(|rt| rt.city(name));
        let rec_next = rec.and_then(|r| r.turn(st.turn + 1)).and_then(|rt| rt.city(name));
        if rec_now.is_some() || rec_next.is_some() {
            // replay: a tile the recorded city works that another of the player's cities owns was swapped to it
            // (a tile bought by one city and later worked by a neighbour)
            let want: Vec<XY> = rec_now.iter().flat_map(|r| r.worked.iter().copied()).collect();
            let center = st.cities[ci].center;
            for xy in want {
                if xy == center || st.cities[ci].owned_at(xy).is_some() || self.w.map.distance(xy, center) > 3 {
                    continue;
                }
                let Some(j) = st.cities.iter().position(|o| o.name != name && o.center != xy && o.owned_at(xy).is_some()) else { continue };
                if st.cities[j].districts.iter().any(|d| (d.x, d.y) == xy) {
                    continue;
                }
                let k = st.cities[j].owned.iter().position(|p| p.xy() == xy).unwrap();
                let mut p = st.cities[j].owned.remove(k);
                st.cities[j].worked.retain(|q| q.xy() != xy);
                p.workers = 0;
                st.cities[ci].owned.push(p);
            }
        }
        let e = self.engine(env, &st.cities);
        let w = focus_weights(*st.focus.get(&st.cities[ci].name).unwrap());
        let mut c = st.cities[ci].clone();
        self.w.assign_citizens(&mut c, &e, &w, 1.0);
        if rec_now.is_some() || rec_next.is_some() {
            // replay: the tiles the recorded citizens worked during the turn: the next save's (reassignments made
            // during the turn), those also worked at its start first; then the start's; the rest by the rule
            let now: Vec<XY> = rec_now.map(|r| r.worked.clone()).unwrap_or_default();
            let next: Vec<XY> = rec_next.map(|r| r.worked.clone()).unwrap_or_default();
            let order: Vec<XY> = now.iter().chain(next.iter().filter(|x| !now.contains(x))).copied().collect();
            let pop = c.population.max(0) as usize;
            let dtiles: Vec<XY> = c.districts.iter().map(|d| (d.x, d.y)).collect();
            let mut pick: Vec<XY> = order.into_iter()
                .filter(|xy| *xy != c.center && !dtiles.contains(xy) && c.owned_at(*xy).is_some() && self.w.workable(*xy, c.center, &e)).take(pop).collect();
            for p in c.worked.iter().skip(1) {
                if pick.len() >= pop {
                    break;
                }
                if !pick.contains(&p.xy()) {
                    pick.push(p.xy());
                }
            }
            let mut worked = vec![c.worked[0].clone()];
            for xy in pick {
                let mut p = c.owned_at(xy).unwrap().clone();
                p.workers = 1;
                worked.push(p);
            }
            c.worked = worked;
        }
        st.cities[ci].worked = c.worked;
    }

    // ---------------------------------------------------------------- the turn
    pub fn advance(&self, st: &mut State, ch: &mut dyn Chooser) {
        crate::value::turn_start(st);
        let r = self.r;
        let t_now = st.turn;
        if let Some(rt) = self.rec.as_ref().and_then(|r| r.turn(t_now)) {
            // replay: the treasury as recorded (tile purchases and some upkeep are not reconstructed)
            st.acc.gold = rt.gold;
            st.acc.faith = rt.faith;
        }
        // the age (from the record; scenario.age) and, when a new era begins, its dedications (one; three in a
        // heroic age)
        st.player.age = Some(self.sc.age(t_now));
        let era = self.sc.game_era(t_now);
        if st.era_mark != Some(era) {
            let first = st.era_mark.is_none();
            st.era_mark = Some(era);
            if !first {
                st.player.dedications.clear();
                let n = if st.player.age == Some(s!("HEROIC")) { 3 } else { 1 };
                let now = self.r.era_index(era).unwrap_or(0);
                let t = self.r.tab("CommemorationTypes");
                let era_of = |i: u32, c: &str| t.v(i, c).sym().filter(|e| !e.is_empty()).and_then(|e| self.r.era_index(e));
                let all: Vec<Sym> = (0..t.len() as u32).filter(|&i| era_of(i, "MinimumGameEra").map(|lo| lo <= now).unwrap_or(true)
                    && era_of(i, "MaximumGameEra").map(|hi| now <= hi).unwrap_or(true)).filter_map(|i| t.v(i, "CommemorationType").sym()).collect();
                for _ in 0..n {
                    let opts: Vec<Choice> = all.iter().filter(|c| !st.player.dedications.contains(c)).map(|c| Choice::S(*c)).collect();
                    if let Some(c) = self.decide(st, "dedication", Ent::S(Sym(0)), opts, ch).and_then(|c| c.sym()) {
                        st.player.dedications.push(c);
                    }
                }
            }
        }
        if let Some(next) = self.rec.as_ref().and_then(|r| r.turn(t_now + 1)) {
            // replay: tiles the player moved between its cities this turn (the next save's owner), from the start of the
            // turn, with their state (a chop on the moved tile goes to the new owner)
            for ci in 0..st.cities.len() {
                let name = st.cities[ci].name;
                let Some(rc) = next.city(name) else { continue };
                let mut changed = false;
                for &xy in &rc.owned {
                    if st.cities[ci].owned_at(xy).is_some() {
                        continue;
                    }
                    let from = (0..st.cities.len()).find(|&j| j != ci && st.cities[j].owned_at(xy).is_some());
                    // (new tiles: at the end of the turn, as borders grow; see "borders")
                    let Some(j) = from else { continue };
                    let k = st.cities[j].owned.iter().position(|p| p.xy() == xy).unwrap();
                    st.cities[j].worked.retain(|w| w.xy() != xy);
                    let plot = st.cities[j].owned.remove(k);
                    self.refresh_near(&mut st.cities[j]);
                    st.cities[ci].owned.push(plot);
                    changed = true;
                }
                if changed {
                    self.refresh_near(&mut st.cities[ci]);
                }
            }
        }
        if let Some(rt) = self.rec.as_ref().or(self.tiles_rec.as_ref()).and_then(|r| r.turn(t_now)) {
            // replay: government and policies as recorded this turn (the player may swap them at any time; only the
            // slots the government has and the policies known so far)
            if let Some(g) = rt.gov {
                if self.available_governments(st).contains(&g) || Some(g) == st.player.government {
                    if let Some(old) = st.player.government.filter(|o| *o != g) {
                        st.past_govs.insert(old);
                    }
                    st.player.government = Some(g);
                }
            }
            if let Some(g) = st.player.government {
                let slots = self.slots(g, st);
                let mut chosen: Vec<Sym> = vec![];
                for slot in slots {
                    let ok = self.policies_for_slot(st, slot);
                    if let Some(p) = rt.policies.iter().find(|p| ok.contains(p) && !chosen.contains(p)) {
                        chosen.push(*p);
                    }
                }
                st.player.policies = chosen;
                st.policy_dirty = false;
            }
        }
        if let Some(rec) = self.rec.as_ref().or(self.tiles_rec.as_ref()) {
            // replay: switch research / civic as recorded (the progress of the old one is kept)
            // (one the record has finished by now and the replay has not is kept until it is: the replay catches up)
            let rt = rec.turn(t_now);
            let behind = |cur: Option<Sym>, done: &BTreeSet<Sym>, rec_done: Option<&Vec<Sym>>| {
                cur.map(|c| !done.contains(&c) && rec_done.map(|d| d.contains(&c)).unwrap_or(false)).unwrap_or(false)
            };
            if let Some(x) = rec.research.get(&t_now) {
                if !behind(st.research, &st.acc.techs, rt.map(|r| &r.techs)) && !st.acc.techs.contains(x) && self.available_techs(st).contains(&Choice::S(*x)) {
                    st.research = Some(*x);
                }
            }
            if let Some(x) = rec.civic.get(&t_now) {
                if !behind(st.civic, &st.acc.civics, rt.map(|r| &r.civics)) && !st.acc.civics.contains(x) && self.available_civics(st).contains(&Choice::S(*x)) {
                    st.civic = Some(*x);
                }
            }
        }
        if st.research.map(|x| st.acc.techs.contains(&x)).unwrap_or(true) {
            let o = self.available_techs(st);
            st.research = self.decide(st, "research", Ent::S(Sym(0)), o, ch).and_then(|c| c.sym());
        }
        if st.civic.map(|x| st.acc.civics.contains(&x)).unwrap_or(true) {
            let o = self.available_civics(st);
            st.civic = self.decide(st, "civic", Ent::S(Sym(0)), o, ch).and_then(|c| c.sym());
        }
        let govs = self.available_governments(st);
        if govs.iter().any(|g| !st.known_govs.contains(g)) {
            st.known_govs = govs.iter().copied().collect();
            let g = self.decide(st, "gov", Ent::S(Sym(0)), govs.iter().map(|g| Choice::S(*g)).collect(), ch).and_then(|c| c.sym());
            if let Some(g) = g {
                if Some(g) != st.player.government {
                    if let Some(old) = st.player.government {
                        st.past_govs.insert(old);
                    }
                    st.player.government = Some(g);
                    st.policy_dirty = true;
                }
            }
        }
        if st.policy_dirty {
            let mut chosen: Vec<Sym> = vec![];
            let slots = self.slots(st.player.government.unwrap(), st);
            for (i, slot) in slots.iter().enumerate() {
                let mut opts: Vec<Choice> = self.policies_for_slot(st, *slot).into_iter().filter(|p| !chosen.contains(p)).map(Choice::S).collect();
                opts.push(Choice::S(s!("NONE")));
                let pick = self.decide(st, "policy", Ent::I(i as i64), opts, ch).and_then(|c| c.sym());
                if let Some(p) = pick {
                    if p != s!("NONE") {
                        chosen.push(p);
                    }
                }
            }
            st.player.policies = chosen;
            st.policy_dirty = false;
        }
        self.governor_step(st, ch);
        let mut env = self.player_env(st);
        // pantheon
        let cost = r.param("RELIGION_PANTHEON_MIN_FAITH") * r.game_speed_cost_multiplier(self.speed);
        // replay: on the turn the recorded player founded it (its faith count can differ a little)
        let rec_pantheon = self.rec.as_ref().or(self.tiles_rec.as_ref()).and_then(|r| r.turn(t_now + 1)).map(|r| r.pantheon.is_some()).unwrap_or(false);
        if st.player.pantheon.is_none() && (st.acc.faith >= cost || rec_pantheon) {
            let t = r.tab("Beliefs");
            let opts: Vec<Choice> = r.scan("Pantheons", "Beliefs").into_iter().filter(|&i| t.v(i, "BeliefClassType").sym() == Some(s!("BELIEF_CLASS_PANTHEON"))).map(|i| Choice::S(t.v(i, "BeliefType").sym().unwrap())).collect();
            if let Some(p) = self.decide(st, "pantheon", Ent::S(Sym(0)), opts, ch).and_then(|c| c.sym()) {
                st.player.pantheon = Some(p);
                st.acc.faith -= cost;
                env = self.player_env(st);
            }
        }
        // settlers arriving
        let arriving: Vec<Settler> = st.settlers.iter().filter(|s| s.arrive <= t_now).cloned().collect();
        for s in arriving {
            st.settlers.retain(|x| x != &s);
            let Some(target) = s.target else { continue };
            if self.w.site_score(target, &st.cities, &st.acc.techs, &st.acc.civics) <= 0.0 {
                continue;
            }
            let name = s.name.unwrap_or_else(|| sym(&format!("CITY_{}", st.cities.len() + 1)));
            let c = self.found_city(st, name, target, &env);
            st.cities.push(c);
            st.acc.cities.insert(name, CityAcc { population: 1, ..Default::default() });
            st.border.insert(name, (0.0, 0));
            st.focus.insert(name, s!("balanced"));
            // units every new city gets (BBG Ancestral Hall: a builder, which acts on the founding turn)
            for (u, n) in self.founding_units(st) {
                for _ in 0..n {
                    if u == s!("UNIT_BUILDER") {
                        let e = self.engine(&env, &st.cities);
                        let charges = builder_charges(r, &e, u, name);
                        st.builders.push(Builder { charges, at: target, ready: t_now, id: None, pending: None, idle: None });
                    } else {
                        *st.units.entry_or(u, 0) += 1;
                    }
                }
            }
        }
        self.rome_free_buildings(st);
        // replay: tiles the player swapped between their own cities belong to the city the record shows at the start
        // of the turn
        if let Some(rec) = self.rec.as_ref().or(self.tiles_rec.as_ref()) {
            for ci in 0..st.cities.len() {
                let name = st.cities[ci].name;
                let Some(rc) = rec.turn(t_now).and_then(|rt| rt.city(name)) else { continue };
                for &xy in &rc.owned {
                    let Some(j) = st.cities.iter().position(|o| o.name != name && o.center != xy && o.owned_at(xy).is_some()) else { continue };
                    if st.cities[j].districts.iter().any(|d| (d.x, d.y) == xy) {
                        continue;
                    }
                    let k = st.cities[j].owned.iter().position(|p| p.xy() == xy).unwrap();
                    let mut p = st.cities[j].owned.remove(k);
                    st.cities[j].worked.retain(|q| q.xy() != xy);
                    p.workers = 0;
                    st.cities[ci].owned.push(p);
                }
            }
        }
        // citizens
        for ci in 0..st.cities.len() {
            let n = st.cities[ci].name;
            let pop = st.cacc(n).population;
            if st.cities[ci].pop_mark != Some(pop) {
                st.cities[ci].pop_mark = Some(pop);
                let f = self.decide(st, "focus", Ent::S(n), FOCUS.iter().map(|f| Choice::S(ss(f))).collect(), ch).and_then(|c| c.sym()).unwrap_or(s!("balanced"));
                st.focus.insert(n, f);
            }
            st.cities[ci].population = pop;
            self.assign(st, ci, &env);
        }
        // production before the builders: a chop goes into the item the city works on
        // a wonder another civilization finished first
        let taken = self.sc.wonders_taken(t_now).clone();
        for c in st.cities.iter_mut() {
            if let Some(cur) = c.current {
                if taken.contains(&cur) {
                    c.current = None;
                    let mut kill = vec![];
                    for (i, x) in c.districts.iter().enumerate() {
                        if x.dtype == s!("DISTRICT_WONDER") && x.plot.wonder == Some(cur) && !x.complete {
                            kill.push(i);
                        }
                    }
                    for i in kill.into_iter().rev() {
                        let d = c.districts.remove(i);
                        if let Some(p) = c.owned_at_mut((d.x, d.y)) {
                            p.wonder = None;
                        }
                    }
                }
            }
        }
        // replay: the item each city works on as recorded (switching keeps the progress of the old one)
        if let Some(rt) = self.rec.as_ref().or(self.tiles_rec.as_ref()).and_then(|r| r.turn(t_now)) {
            for ci in 0..st.cities.len() {
                let Some(x) = rt.city(st.cities[ci].name).and_then(|c| c.current) else { continue };
                if st.cities[ci].current == Some(x) {
                    continue;
                }
                let placed = st.cities[ci].districts.iter().any(|d| !d.complete && (d.dtype == x || (d.dtype == s!("DISTRICT_WONDER") && d.plot.wonder == Some(x))));
                if placed {
                    st.cities[ci].current = Some(x);
                } else if self.buildable(st, ci).contains(&Choice::S(x)) {
                    st.cities[ci].current = None;
                }
            }
        }
        // production choices
        for ci in 0..st.cities.len() {
            if st.cities[ci].current.is_some() {
                continue;
            }
            let opts = self.buildable(st, ci);
            let n = st.cities[ci].name;
            let item = self.decide(st, "prod", Ent::S(n), opts, ch).and_then(|c| c.sym());
            if let Some(item) = item {
                let rec_xy = self.rec.as_ref().or(self.tiles_rec.as_ref()).and_then(|r| r.district_at(n, item)).filter(|xy| st.cities[ci].owned_at(*xy).is_some());
                if item_kind(item) == Kind::District && rec_xy.is_some() {
                    // replay: where the recorded city put it
                    let xy = rec_xy.unwrap();
                    let p = st.cities[ci].owned_at(xy).unwrap().clone();
                    let nb = self.neighbours_with_districts(st, xy);
                    st.cities[ci].districts.push(District { dtype: item, x: xy.0, y: xy.1, complete: false, plot: p, neighbours: nb });
                } else if item_kind(item) == Kind::District {
                    if let Some((_, _, d)) = self.district_site(st, ci, item) {
                        let mut d = d;
                        d.complete = false;
                        st.cities[ci].districts.push(d);
                    }
                } else if item_kind(item) == Kind::Building && r.is_wonder(item) {
                    if let Some((_, xy)) = self.wonder_site(st, ci, item) {
                        if let Some(p) = st.cities[ci].owned_at_mut(xy) {
                            p.wonder = Some(item);
                        }
                        let p = st.cities[ci].owned_at(xy).unwrap().clone();
                        let nb = self.neighbours_with_districts(st, xy);
                        st.cities[ci].districts.push(District { dtype: s!("DISTRICT_WONDER"), x: xy.0, y: xy.1, complete: false, plot: p, neighbours: nb });
                    }
                }
            }
            st.cities[ci].current = item;
        }
        // builders
        let mut bi = 0;
        while bi < st.builders.len() {
            if st.builders[bi].id.is_none() {
                st.builder_counter += 1;
                st.builders[bi].id = Some(st.builder_counter);
            }
            if st.builders[bi].ready > t_now {
                bi += 1;
                continue;
            }
            let act = if let Some(p) = st.builders[bi].pending.take() {
                p
            } else {
                // a builder that chose to wait decides again once something it could use changes
                let mark = self.idle_mark(st);
                if self.rec.is_none() && st.builders[bi].idle.as_ref() == Some(&mark) {
                    bi += 1;
                    continue;
                }
                st.builders[bi].idle = None;
                let opts = self.builder_options(st, &env, st.builders[bi].at);
                // a builder's decisions are numbered per builder (one builder's change does not shift the others')
                let bid = st.builders[bi].id.unwrap_or(0);
                let act = self.decide(st, "builder", Ent::I(bid), opts, ch);
                if act.is_none() || act == Some(Choice::Wait) {
                    st.builders[bi].idle = Some(mark);
                    bi += 1;
                    continue;
                }
                let act = act.unwrap();
                let Choice::Act(_, _, x, y, _) = act else { bi += 1; continue };
                // replay: the recorded builder did it on that turn (its walk is not in the saves)
                let k = if self.rec.is_some() || self.tiles_rec.is_some() { Some(0) } else { self.travel(st, st.builders[bi].at, (x, y), "UNIT_BUILDER") };
                let Some(k) = k else { bi += 1; continue };
                if k > 0 {
                    st.builders[bi].ready = t_now + k;
                    st.builders[bi].at = (x, y);
                    st.builders[bi].pending = Some(act);
                    bi += 1;
                    continue;
                }
                act
            };
            let Choice::Act(kind, cname, x, y, what) = act.clone() else { bi += 1; continue };
            st.builders[bi].at = (x, y);
            let ci = st.cities.iter().position(|c| c.name == cname);
            if kind == s!("rush") {
                if ci.is_none() || st.cities[ci.unwrap()].current != Some(what) {
                    bi += 1;
                    continue;
                }
                let mut env_cost = env.clone();
                env_cost.units_trained = st.acc.units_trained.clone();
                if let Some(cost) = item_cost(r, &env_cost, what, self.speed, None, false) {
                    if cost != 0.0 {
                        let pct = self.rush_percent(&self.engine(&env, &st.cities), Some(what));
                        st.cacc_mut(cname).overflow += cost * pct / 100.0;
                    }
                }
                st.builders[bi].charges -= 1;
                bi += 1;
                continue;
            }
            let Some(ci) = ci else { bi += 1; continue };
            let Some(pi) = st.cities[ci].owned.iter().position(|p| p.xy() == (x, y)) else { bi += 1; continue };
            {
                let p = &st.cities[ci].owned[pi];
                if (kind == s!("improve") || kind == s!("plant")) && (p.improvement.is_some() || (kind == s!("plant") && p.feature.is_some())) {
                    bi += 1;
                    continue;
                }
            }
            if kind == s!("improve") {
                st.cities[ci].owned[pi].improvement = Some(what);
            } else if kind == s!("plant") {
                st.cities[ci].owned[pi].feature = Some(what);
            } else {
                let p = st.cities[ci].owned[pi].clone();
                let ys = self.harvest_yields(&self.eng(st), &p, st, Some(cname)).into_iter().find(|(k, _)| *k == what).map(|(_, y)| y);
                let Some(ys) = ys else { bi += 1; continue };
                if what == s!("feature") {
                    st.cities[ci].owned[pi].feature = None;
                } else {
                    st.cities[ci].owned[pi].resource = None;
                }
                if crate::envcfg::trace_city(&cname.as_string()) {
                    eprintln!("HARVEST {cname} {:?} {:?} {ys:?}", (x, y), what);
                }
                self.chop_into(st, ci, get_y(&ys, s!("YIELD_PRODUCTION")), &env);
                let ca = st.cacc_mut(cname);
                ca.food_store += get_y(&ys, s!("YIELD_FOOD"));
                st.acc.gold += get_y(&ys, s!("YIELD_GOLD"));
            }
            st.builders[bi].charges -= 1;
            bi += 1;
        }
        st.builders.retain(|b| b.charges > 0);
        if !st.builders.is_empty() {
            for ci in 0..st.cities.len() {
                self.assign(st, ci, &env);
            }
        }
        // purchase: as many as the player chooses in a turn (each item once per city), the options offered again
        // after each (gold, faith and tile prices change)
        let mut bought_now: Vec<(Sym, Sym)> = vec![];
        for _ in 0..8 {
            let buys: Vec<Choice> = self.buy_options(st, &env, t_now).into_iter()
                .filter(|o| !matches!(o, Choice::BuyItem(c, it, _, _) if bought_now.contains(&(*c, *it)))).collect();
            let buy = if buys.len() > 1 { self.decide(st, "buy", Ent::I(t_now), buys, ch) } else { None };
            match buy {
                Some(Choice::BuyTile(name, gold, x, y)) => {
                    let ci = st.cities.iter().position(|c| c.name == name).unwrap();
                    let p = self.w.map.plots[&(x, y)].clone();
                    st.cities[ci].owned.push(p);
                    self.refresh_near(&mut st.cities[ci]);
                    st.acc.gold -= f64::from_bits(gold);
                    st.tiles_bought += 1;
                }
                Some(Choice::BuyItem(name, item, price, cur)) => {
                    if cur == s!("YIELD_FAITH") {
                        st.acc.faith -= f64::from_bits(price);
                    } else {
                        st.acc.gold -= f64::from_bits(price);
                    }
                    st.bought.push((name, item));
                    bought_now.push((name, item));
                    if item_kind(item) == Kind::District {
                        st.city_mut(name).unwrap().current = None;
                    }
                }
                _ => break,
            }
        }
        // routes that ran their course
        for i in 0..st.traders.len() {
            let t = st.traders[i].clone();
            if t.until.map(|u| t_now >= u).unwrap_or(false) || t.dest.is_none() {
                st.traders[i] = Trader { origin: t.origin, dest: None, info: None, until: None, start: 0 };
                let nt = self.start_route(st, t.origin, ch);
                st.traders[i] = nt;
            }
        }
        let routes = self.routes_now(st);
        let e = self.engine(&env, &st.cities);
        let sims: Vec<City> = st.cities.iter().map(|c| { let mut x = c.clone(); x.population = st.cacc(c.name).population; x }).collect();
        let es = self.engine(&env, &sims);
        let scores = amenity_scores(r, &sims, &es, env.extra_luxuries);
        let mut out: Y = [0.0; 6];
        st.city_output.clear();
        for ci in 0..st.cities.len() {
            let c = &st.cities[ci];
            let score = scores.iter().find(|(n, _)| *n == c.name).map(|x| x.1).unwrap_or(0.0);
            let y = city_turn_yields(r, &e, c, st.cacc(c.name).population, &routes, happiness(r, score).0, Some(score));
            st.city_output.push((c.name, y));
            if crate::envcfg::trace().map(|v| v == format!("{}:{}", c.name, t_now)).unwrap_or(false) {
                eprintln!("TRACE {} T{t_now} pop {} worked {:?} out {:?} static {:?} pct {:?} amenity {score} routes {:?}", c.name, st.cacc(c.name).population,
                          c.worked.iter().map(|w| w.xy()).collect::<Vec<_>>(), y, crate::yields::city_yields(r, &e, c, &routes),
                          crate::yields::city_yield_percents(r, &e, c, Some(score)), routes.iter().map(|x| (x.origin.name.as_string(), x.dest.name.as_string())).collect::<Vec<_>>());
                let mods = e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]);
                for w in &c.worked {
                    let mut wp = w.clone();
                    let center = w.xy() == c.center;
                    wp.district_type = if center { Some(s!("DISTRICT_CITY_CENTER")) } else { None };
                    eprintln!("   tile {:?} {:?} {:?} -> {:?}", w.xy(), w.feature, w.improvement, crate::yields::tile_yields_bbcc(r, &wp, &e.env.techs, &e.env.civics, center, center && c.capital, mods, Some(&e.plots), Some(c), None));
                }
                eprintln!("   techs {:?} buildings {:?}", e.env.techs, c.buildings);
            }
            for k in 0..6 {
                out[k] += y[k];
            }
            let bpct: f64 = e.city_effects(s!("EFFECT_ADJUST_CITY_CULTURE_BORDER_EXPANSION"), c.name).iter().map(|x| x.args.f("Amount")).sum();
            let n = c.name;
            st.border.get_mut(&n).unwrap().0 += y[4] * (1.0 + bpct / 100.0);
        }
        st.last_output = out;
        // Seoul: a random eureka of each new era
        if self.sc.game_era(t_now + 1) != self.sc.game_era(t_now) {
            let k: i64 = e.player_effects(s!("EFFECT_GRANT_RANDOM_TECHNOLOGY_BOOST_ON_NEW_ERA")).iter().map(|x| x.args.get("Amount").map(|_| x.args.f("Amount") as i64).unwrap_or(1)).sum();
            for _ in 0..k {
                self.boost_cheapest(st, self.sc.game_era(t_now + 1), false);
            }
        }
        // the turn itself
        // a recorded boost of turn t comes once the plan has built as many scouts as the record had built by t (gifted
        // scouts not counted on either side; on t, or later when the plan.s scouting lags)
        let scouts = *st.acc.units_trained.get(&s!("UNIT_SCOUT")).unwrap_or(&0) - self.sc.scouts_gifted_through(st.gifts_through);
        let mut boosts = self.endogenous_boosts(st);
        let mut exo_turns: Vec<i64> = self.sc.exo_boosts.keys().copied().filter(|t| *t <= t_now).collect();
        exo_turns.sort();
        for t in exo_turns {
            if scouts >= self.sc.scouts_built_by(t) {
                boosts.extend(self.sc.exo_boosts[&t].iter().copied().filter(|b| !st.player.boosted.contains(b)));
            }
        }
        if let Some(rec) = self.rec.as_ref() {
            // replay: the boosts as recorded (on the turn the record shows them, or at once if the record had one
            // earlier); what the rules would boost differently is noted as a gap of the model
            let had = rec.turn(t_now).map(|r| r.boosted.clone()).unwrap_or_default();
            let now = rec.turn(t_now + 1).map(|r| r.boosted.clone()).unwrap_or_default();
            let rec_b: Vec<Sym> = now.iter().copied().filter(|b| !st.player.boosted.contains(b)).collect();
            let mut notes = self.rec_notes.borrow_mut();
            for b in &rec_b {
                if !had.contains(b) && !boosts.contains(b) {
                    notes.push((t_now, format!("boost {b}: recorded, not by the rules")));
                }
            }
            for b in &boosts {
                if !now.contains(b) && !st.player.boosted.contains(b) && !st.acc.techs.contains(b) && !st.acc.civics.contains(b) {
                    notes.push((t_now, format!("boost {b}: by the rules, not recorded")));
                }
            }
            boosts = rec_b;
        }
        let mut uniq: Vec<Sym> = vec![];
        for b in boosts {
            if !uniq.contains(&b) && !st.acc.techs.contains(&b) && !st.acc.civics.contains(&b) && !st.player.boosted.contains(&b) {
                uniq.push(b);
            }
        }
        for &b in &uniq {
            st.player.boosted.insert(b);
        }
        // replay: a district's cost is fixed when it is placed; the record places some before it starts producing them
        if let Some(rec) = self.rec.as_ref().or(self.tiles_rec.as_ref()) {
            for ci in 0..st.cities.len() {
                let name = st.cities[ci].name;
                let Some(rc) = rec.turn(t_now + 1).and_then(|rt| rt.city(name)) else { continue };
                for &(d, _, complete) in &rc.districts {
                    let placed = st.cities[ci].districts.iter().any(|x| x.dtype == d);
                    if complete || placed || st.acc.cities.get(&name).map(|ca| ca.locked_cost.contains(&d)).unwrap_or(true) {
                        continue;
                    }
                    if let Some(cost) = crate::sim::current_cost(r, &st.acc, &st.cities, &env, name, d, self.speed) {
                        st.cacc_mut(name).locked_cost.insert(d, cost);
                    }
                }
            }
        }
        let cities_now = st.cities.clone();
        let events = {
            let mut cache = self.cache.borrow_mut();
            let _g = crate::prof::start(5);
            // replay: the tiles the recorded city works at the start of the next turn; else the citizens reassigned
            // by the rule
            let rec = self.rec.as_ref().or(self.tiles_rec.as_ref()).cloned();
            let focus = st.focus.clone();
            let after_settler = |c: &City, e: &Engine| -> City {
                let mut out = c.clone();
                if let Some(rc) = rec.as_ref().and_then(|r| r.turn(t_now + 1)).and_then(|rt| rt.city(c.name)) {
                    let mut worked = vec![c.worked[0].clone()];
                    for xy in rc.worked.iter().filter(|xy| **xy != c.center).take(c.population.max(0) as usize) {
                        if let Some(p) = c.owned_at(*xy) {
                            let mut p = p.clone();
                            p.workers = 1;
                            worked.push(p);
                        }
                    }
                    if worked.len() == c.population.max(0) as usize + 1 {
                        out.worked = worked;
                        return out;
                    }
                }
                let w = focus_weights(*focus.get(&c.name).unwrap_or(&s!("balanced")));
                self.w.assign_citizens(&mut out, e, &w, 1.0);
                out
            };
            step(r, &mut cache, &mut st.acc, &cities_now, &env, &st.units, self.speed, st.research, st.civic, &uniq, &routes, &after_settler)
        };
        let mut events = events;
        // the turn's output: the yields growth and production used (techs finished this turn count)
        let used: Vec<(Sym, Y)> = events.iter().filter_map(|ev| if let Event::Yields(n, y) = ev { Some((*n, *y)) } else { None }).collect();
        if !used.is_empty() {
            let mut out: Y = [0.0; 6];
            for (_, y) in &used {
                for k in 0..6 {
                    out[k] += y[k];
                }
            }
            st.last_output = out;
            st.city_output = used;
        }
        for (name, item) in std::mem::take(&mut st.bought) {
            if item_kind(item) == Kind::Unit {
                *st.acc.units_trained.entry_or(item, 0) += 1;
            } else {
                st.acc.units_trained.entry_or(item, 0);
            }
            events.push(Event::CompletedPurchase(name, item));
        }
        if let Some(g) = self.sc.gold_out.get(&t_now) {
            st.acc.gold = (st.acc.gold - g).max(0.0);
        }
        // the recorded gifts of turn t, as the recorded boosts: once the plan has built as many scouts as the record
        // had at t
        while st.gifts_through < t_now {
            let t = st.gifts_through + 1;
            let built = *st.acc.units_trained.get(&s!("UNIT_SCOUT")).unwrap_or(&0) - self.sc.scouts_gifted_through(st.gifts_through);
            if built < self.sc.scouts_built_by(t) {
                break;
            }
            st.gifts_through = t;
            st.acc.faith += self.sc.faith_gift.get(&t).copied().unwrap_or(0.0);
            st.acc.gold += self.sc.gold_gift.get(&t).copied().unwrap_or(0.0);
            for u in self.sc.unit_gifts.get(&t).cloned().unwrap_or_default() {
                // the game counts them among the units trained
                *st.acc.units_trained.entry_or(u, 0) += 1;
                if u == s!("UNIT_BUILDER") {
                    let cap = st.cities[0].center;
                    let ch0 = r.lookup_f("Units", "UnitType", s!("UNIT_BUILDER"), "BuildCharges") as i64;
                    st.builders.push(Builder { charges: ch0, at: cap, ready: t_now, id: None, pending: None, idle: None });
                } else if u == s!("UNIT_TRADER") {
                    let origin = st.cities[0].name;
                    let tr = self.start_route(st, origin, ch);
                    st.traders.push(tr);
                } else if u != s!("UNIT_SETTLER") {
                    *st.units.entry_or(u, 0) += 1;
                }
            }
        }
        for ev in &events {
            if let Event::Researched(t) = ev {
                if t.starts_with("CIVIC_") {
                    st.policy_dirty = true;
                }
            }
            let (name, item, completed) = match ev {
                Event::Completed(n, i) => (*n, *i, true),
                Event::CompletedPurchase(n, i) => (*n, *i, false),
                _ => continue,
            };
            let ci = st.cities.iter().position(|c| c.name == name).unwrap();
            if completed {
                st.cities[ci].current = None;
            }
            if item == s!("UNIT_BUILDER") {
                let eng = self.engine(&env, &st.cities);
                let charges = builder_charges(r, &eng, s!("UNIT_BUILDER"), name);
                st.builders.push(Builder { charges, at: st.cities[ci].center, ready: t_now + 1, id: None, pending: None, idle: None });
            } else if item == s!("UNIT_SETTLER") {
                let origin = st.cities[ci].center;
                let opts = self.settle_sites(st, origin);
                let target = self.decide(st, "settle", Ent::S(Sym(0)), opts, ch);
                let target = match target { Some(Choice::Xy(x, y)) => Some((x, y)), _ => None };
                let mut label = target.and_then(|t| self.s.site_names.iter().find(|(xy, _)| *xy == t).map(|(_, n)| *n));
                if let Some(l) = label {
                    if st.cities.iter().any(|c| c.name == l) {
                        label = None;
                    }
                }
                if let Some(t) = target {
                    if let Some(k) = self.travel(st, origin, t, "UNIT_SETTLER") {
                        st.settler_counter += 1;
                        // replay (validation, or the recorded player's choices replayed): the city is founded on the
                        // recorded turn (the settler's actual path is not recorded)
                        let arrive = match self.rec.as_ref().or(self.tiles_rec.as_ref()).and_then(|r| r.sites.iter().find(|x| x.2 == t)) {
                            Some(x) => (x.0 + 1).max(t_now + 1),
                            None => t_now + 1 + k,
                        };
                        st.settlers.push(Settler { from: name, target: Some(t), name: label, arrive, id: st.settler_counter });
                    }
                }
            } else if item == s!("UNIT_TRADER") {
                let t = self.start_route(st, name, ch);
                st.traders.push(t);
            } else if item_kind(item) == Kind::Unit {
                *st.units.entry_or(item, 0) += 1;
            } else if item_kind(item) == Kind::Building {
                st.cities[ci].buildings.push(item);
                for x in st.cities[ci].districts.iter_mut() {
                    if x.dtype == s!("DISTRICT_WONDER") && x.plot.wonder == Some(item) {
                        x.complete = true;
                    }
                }
            } else if item_kind(item) == Kind::District {
                for x in st.cities[ci].districts.iter_mut() {
                    if x.dtype == item {
                        x.complete = true;
                    }
                }
                if item == s!("DISTRICT_GOVERNMENT") {
                    st.gov_plaza = true;
                }
            }
        }
        for ev in &events {
            match ev {
                Event::Completed(n, i) | Event::CompletedPurchase(n, i) if matches!(item_kind(*i), Kind::Building | Kind::District) => {
                    let (table, col) = if item_kind(*i) == Kind::Building { ("BuildingModifiers", "BuildingType") } else { ("DistrictModifiers", "DistrictType") };
                    self.apply_grants(st, table, col, *i, Some(*n), &env, ch);
                    let ci = st.cities.iter().position(|c| c.name == *n).unwrap();
                    self.on_completed(st, ci, *i, &env);
                }
                Event::Researched(t) => {
                    let (table, col) = if t.starts_with("TECH_") { ("TechnologyModifiers", "TechnologyType") } else { ("CivicModifiers", "CivicType") };
                    self.apply_grants(st, table, col, *t, None, &env, ch);
                }
                _ => {}
            }
        }
        self.great_people(st, ch);
        self.rome_free_buildings(st);
        // replay: buildings and districts the next save shows done are done (bought, rushed, or finished on a
        // carry-over the saves do not hold); noted, so validation tells them from production the model missed
        if let Some(next) = self.rec.as_ref().and_then(|r| r.turn(t_now + 1)) {
            for ci in 0..st.cities.len() {
                let name = st.cities[ci].name;
                let Some(rc) = next.city(name) else { continue };
                for b in rc.buildings.clone() {
                    if !st.cities[ci].buildings.contains(&b) {
                        st.cities[ci].buildings.push(b);
                        if st.cities[ci].current == Some(b) {
                            st.cities[ci].current = None;
                        }
                        st.cacc_mut(name).progress.remove(&b);
                        self.rec_notes.borrow_mut().push((t_now, format!("{name} {b}: done by the record (bought or carried over)")));
                    }
                }
                for (dt, xy, done) in rc.districts.clone() {
                    if !done || dt == s!("DISTRICT_CITY_CENTER") || dt == s!("DISTRICT_WONDER") {
                        continue;
                    }
                    let have = st.cities[ci].districts.iter().position(|d| d.dtype == dt);
                    match have {
                        Some(i) if st.cities[ci].districts[i].complete => {}
                        Some(i) => {
                            st.cities[ci].districts[i].complete = true;
                            if st.cities[ci].current == Some(dt) {
                                st.cities[ci].current = None;
                            }
                            st.cacc_mut(name).progress.remove(&dt);
                            self.rec_notes.borrow_mut().push((t_now, format!("{name} {dt}: done by the record (bought or carried over)")));
                        }
                        None => {
                            if let Some(p) = st.cities[ci].owned_at(xy).cloned() {
                                let nb = self.neighbours_with_districts(st, xy);
                                st.cities[ci].districts.push(District { dtype: dt, x: xy.0, y: xy.1, complete: true, plot: p, neighbours: nb });
                                self.rec_notes.borrow_mut().push((t_now, format!("{name} {dt}: done by the record (bought or carried over)")));
                            }
                        }
                    }
                }
            }
        }
        // borders
        let mut owned_all: HashSet<XY> = st.cities.iter().flat_map(|c| c.owned.iter().map(|p| p.xy())).collect();
        for ci in 0..st.cities.len() {
            let n = st.cities[ci].name;
            if let Some(rt) = self.s.recorded_tiles.as_ref().and_then(|m| m.get(&st.cities[ci].center)) {
                loop {
                    let (b0, b1) = *st.border.get(&n).unwrap();
                    let cost = self.w.border_cost(b1);
                    if b0 < cost {
                        break;
                    }
                    st.border.insert(n, (b0 - cost, b1 + 1));
                }
                for &(t, xy) in rt {
                    if t <= t_now && !owned_all.contains(&xy) {
                        let p = self.w.map.plots[&xy].clone();
                        st.cities[ci].owned.push(p);
                        self.refresh_near(&mut st.cities[ci]);
                        owned_all.insert(xy);
                    }
                }
                continue;
            }
            loop {
                let (b0, b1) = *st.border.get(&n).unwrap();
                let cost = self.w.border_cost(b1);
                if b0 < cost {
                    break;
                }
                let mut forced = self.s.borders.iter().find(|((c, k), _)| *c == st.cities[ci].center && *k == b1).map(|(_, xy)| *xy);
                // the recorded player's play: the next plot the recorded city got (its timing is the model's)
                if let Some(r) = self.tiles_rec.as_ref() {
                    let next = r.tiles.get(&st.cities[ci].center).and_then(|v| v.iter().map(|x| x.1).find(|xy| !owned_all.contains(xy) && self.w.map.plots.contains_key(xy)));
                    forced = forced.or(next);
                }
                st.border.insert(n, (b0 - cost, b1 + 1));
                let xy = forced.or_else(|| self.w.next_border_tile(ci, &st.cities, &st.acc.techs, &st.acc.civics, &DEFAULT_WEIGHTS));
                if let Some(xy) = xy {
                    let p = self.w.map.plots[&xy].clone();
                    st.cities[ci].owned.push(p);
                    self.refresh_near(&mut st.cities[ci]);
                    owned_all.insert(xy);
                }
            }
        }
        st.turn += 1;
        if let Some(m) = self.milestones.as_ref() {
            m.record(st);
        }
    }
}

const GRANT_EFFECTS: [&str; 14] = ["EFFECT_GRANT_UNIT_IN_CITY", "EFFECT_ADJUST_CITY_POPULATION", "EFFECT_GRANT_PLAYER_SPECIFIC_TECHNOLOGY",
    "EFFECT_GRANT_PLAYER_RANDOM_TECHNOLOGY", "EFFECT_GRANT_ALL_TECHNOLOGY_BOOST_BY_ERA", "EFFECT_GRANT_RANDOM_TECHNOLOGY_BOOST_BY_ERA",
    "EFFECT_GRANT_RANDOM_CIVIC_BOOST_BY_ERA", "EFFECT_GRANT_YIELD", "EFFECT_GRANT_CHEAPEST_BUILDING_IN_CITY", "EFFECT_GRANT_BUILDING_IN_CITY_IGNORE",
    "EFFECT_GRANT_PRODUCTION_IN_CITY", "EFFECT_ADJUST_MULTIPLY_TREASURY", "EFFECT_GRANT_PLAYER_RANDOM_CIVIC",
    "EFFECT_GRANT_PLAYER_SPECIFIC_TECH_BOOST_GREAT_PERSON"];

fn settler_free_pop(e: &Engine, city: Sym) -> bool {
    e.city_effects(s!("EFFECT_ADJUST_CITY_SETTLER_CONSUME_POP"), city).iter().any(|x| x.args.is("Enabled", &["0", "false"]))
}
