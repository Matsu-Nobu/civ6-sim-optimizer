//! Generic Civ6 modifier engine (civ6sim/modifiers.py): sources -> collection -> requirements -> effects.
use crate::model::*;
use crate::rules::{Args, Rules};
use crate::sym::{sym, Sym};
use crate::yields;
use std::collections::{BTreeSet, HashMap};

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Subj {
    Player,
    Cs(usize), // index into the player's envoys
    City(Sym),
    District(Sym, i32, i32),
    Plot(Sym, i32, i32),
    Unit(Sym, Option<Sym>),
}

#[derive(Clone, Debug)]
pub struct Eff {
    pub subj: Subj,
    pub args: Args,
    pub mid: Sym,
    pub source: Sym,
}

/// A plot-yield effect kept for tile_yields: (yield, amount, plot requirements, test_any).
#[derive(Clone, Debug)]
pub struct PlotMod {
    pub yt: Sym,
    pub amount: f64,
    pub reqs: Vec<crate::rules::Req>,
    pub test_any: bool,
    pub at: Option<XY>,
}

pub struct Engine<'a> {
    pub r: &'a Rules,
    pub env: Env,
    pub cities: Vec<City>,
    pub plots: HashMap<XY, Plot>,
    pub plot_city: HashMap<XY, Sym>,
    /// effect type -> rows, in the order effect types were first seen (a Python defaultdict)
    pub effects: Effects,
    pub plot_effects: HashMap<Sym, Vec<PlotMod>>,
    religion: Option<Sym>,
    permanent: bool,
    owner: Option<Subj>,
    /// skip modifiers whose effect the simulator never reads (crate::consumed); off for grants_of
    only_consumed: bool,
}

pub static ENGINES: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

pub fn ss(s: &str) -> Sym {
    sym(s)
}

impl<'a> Engine<'a> {
    pub fn new(r: &'a Rules, env: &Env, cities: &[City]) -> Engine<'a> {
        ENGINES.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _g = crate::prof::start(0);
        let plots = yields::plots_by_xy_of(cities);
        let mut plot_city = HashMap::new();
        for c in cities {
            for p in &c.owned {
                plot_city.insert(p.xy(), c.name);
            }
        }
        let mut e = Engine {
            r,
            env: env.clone(),
            cities: cities.to_vec(),
            plots,
            plot_city,
            effects: Effects::default(),
            plot_effects: HashMap::new(),
            religion: None,
            permanent: false,
            owner: None,
            only_consumed: !crate::envcfg::all_effects(),
        };
        for (source, owner, mid) in e.sources() {
            e.religion = None;
            e.apply(mid, owner, source, 0);
        }
        e.religion = None;
        e
    }

    /// An engine with no sources applied (for grants_of: the Python engine empties its effects while resolving a
    /// source's grants, so nothing else is visible then either).
    pub fn bare(r: &'a Rules, env: &Env, cities: &[City]) -> Engine<'a> {
        let plots = yields::plots_by_xy_of(cities);
        let mut plot_city = HashMap::new();
        for c in cities {
            for p in &c.owned {
                plot_city.insert(p.xy(), c.name);
            }
        }
        Engine { r, env: env.clone(), cities: cities.to_vec(), plots, plot_city, effects: Effects::default(),
                 plot_effects: HashMap::new(), religion: None, permanent: false, owner: None, only_consumed: false }
    }

    pub fn city_by_name(&self, n: Sym) -> Option<&City> {
        self.cities.iter().find(|c| c.name == n)
    }

    fn ids(&self, table: &str, col: &str, key: Sym) -> Vec<Sym> {
        self.r.list_sym(table, col, key, "ModifierId")
    }

    fn gp_sources(&self, gp: &GpUse, out: &mut Vec<(Sym, Subj, Sym)>) {
        let r = self.r;
        let ind = gp.individual;
        let need = r.lookup_sym("GreatPersonIndividuals", "GreatPersonIndividualType", ind, "ActionRequiresCompletedDistrictType");
        let mut city: Option<&City> = gp.city.and_then(|n| self.city_by_name(n));
        if city.is_none() {
            let mut order: Vec<&City> = self.cities.iter().collect();
            order.sort_by_key(|c| !c.capital);
            for c in order {
                if need.is_none() || c.districts.iter().any(|d| d.complete && r.district_is(d.dtype, need.unwrap())) {
                    city = Some(c);
                    break;
                }
            }
        }
        let t = r.tab("GreatPersonIndividualActionModifiers");
        for &i in t.find("GreatPersonIndividualType", ind) {
            let mid = t.v(i, "ModifierId").sym().unwrap();
            let target = t.v(i, "AttachmentTargetType").sym().unwrap();
            if target.ends_with("PLAYER") || target.ends_with("UNIT_GREATPERSON") {
                out.push((ind, Subj::Player, mid));
            } else if let Some(c) = city {
                if target.ends_with("_CITY") || target.ends_with("WONDER_IN_TILE") {
                    out.push((ind, Subj::City(c.name), mid));
                } else if target.ends_with("DISTRICT_IN_TILE") {
                    if let Some(d) = c.districts.iter().find(|d| d.complete && (need.is_none() || r.district_is(d.dtype, need.unwrap()))) {
                        out.push((ind, Subj::District(c.name, d.x, d.y), mid));
                    }
                }
            }
        }
    }

    fn sources(&self) -> Vec<(Sym, Subj, Sym)> {
        let r = self.r;
        let pl = &self.env.p;
        let mut out = Vec::new();
        for &t in r.traits_of(pl.civilization, pl.leader).iter() {
            for m in self.ids("TraitModifiers", "TraitType", t) {
                out.push((t, Subj::Player, m));
            }
        }
        for &p in &pl.policies {
            for m in self.ids("PolicyModifiers", "PolicyType", p) {
                out.push((p, Subj::Player, m));
            }
        }
        if let Some(g) = pl.government {
            for m in self.ids("GovernmentModifiers", "GovernmentType", g) {
                out.push((g, Subj::Player, m));
            }
        }
        if let Some(b) = pl.pantheon {
            for m in self.ids("BeliefModifiers", "BeliefType", b) {
                out.push((b, Subj::Player, m));
            }
        }
        for &c in &pl.dedications {
            for m in self.ids("CommemorationModifiers", "CommemorationType", c) {
                out.push((c, Subj::Player, m));
            }
        }
        // religions: not part of the plan's player (none founded)
        for &t in &self.env.techs {
            for m in self.ids("TechnologyModifiers", "TechnologyType", t) {
                out.push((t, Subj::Player, m));
            }
        }
        for &t in &self.env.civics {
            for m in self.ids("CivicModifiers", "CivicType", t) {
                out.push((t, Subj::Player, m));
            }
        }
        let gm = r.tab("GameModifiers");
        for row in r.scan("GameModifiers", "GameModifiers") {
            out.push((s!("game"), Subj::Player, gm.v(row, "ModifierId").sym().unwrap()));
        }
        for c in &self.cities {
            for &b in &c.buildings {
                for m in self.ids("BuildingModifiers", "BuildingType", b) {
                    out.push((b, Subj::City(c.name), m));
                }
            }
            for d in &c.districts {
                if d.complete {
                    for m in self.ids("DistrictModifiers", "DistrictType", d.dtype) {
                        out.push((d.dtype, Subj::District(c.name, d.x, d.y), m));
                    }
                }
            }
            for p in &c.owned {
                if let Some(imp) = p.improvement {
                    if !p.pillaged {
                        for m in self.ids("ImprovementModifiers", "ImprovementType", imp) {
                            out.push((imp, Subj::Plot(c.name, p.x, p.y), m));
                        }
                    }
                }
            }
        }
        for gp in &pl.great_people_used {
            self.gp_sources(gp, &mut out);
        }
        for g in pl.governors.iter().flatten() {
            let city = self.cities.iter().find(|c| governor_here(g, c, pl));
            let Some(city) = city else { continue };
            if !g.established {
                continue;
            }
            let owner = Subj::City(city.name);
            for m in self.ids("GovernorModifiers", "GovernorType", g.gtype) {
                out.push((g.gtype, owner.clone(), m));
            }
            for &p in &g.promotions {
                for m in self.ids("GovernorPromotionModifiers", "GovernorPromotionType", p) {
                    out.push((p, owner.clone(), m));
                }
            }
        }
        for (ei, e) in self.env.p.envoys.iter().enumerate() {
            let mut leaders = vec![e.leader];
            if let Some(b) = r.lookup_sym("Leaders", "LeaderType", e.leader, "InheritFrom") {
                leaders.push(b);
            }
            let src = sym(&format!("{} envoys", e.civilization));
            let lt = r.tab("LeaderTraits");
            let tm = r.tab("TraitModifiers");
            for leader in leaders {
                for &i in lt.find("LeaderType", leader) {
                    let tr = lt.v(i, "TraitType").sym().unwrap();
                    for &j in tm.find("TraitType", tr) {
                        out.push((src, Subj::Cs(ei), tm.v(j, "ModifierId").sym().unwrap()));
                    }
                }
            }
        }
        out
    }

    fn apply(&mut self, mid: Sym, owner: Subj, source: Sym, depth: u32) {
        let Some(m) = self.r.modifier(mid) else { return };
        if depth > 4 {
            return;
        }
        let prev = self.permanent;
        self.permanent = prev || m.permanent;
        self.apply_inner(mid, owner, source, depth, &m);
        self.permanent = prev;
    }

    fn apply_inner(&mut self, mid: Sym, owner: Subj, source: Sym, depth: u32, m: &crate::rules::ModRow) {
        if self.only_consumed && !relevant(self.r, mid, 0) {
            return;
        }
        if let Some(ors) = m.owner_rs {
            if self.reqset(ors, &owner) != Some(true) {
                return;
            }
        }
        let effect = m.effect;
        if effect == Some(s!("EFFECT_ADJUST_PLOT_YIELD")) && m.args.get("ModifierId").is_none() {
            if !source.starts_with("IMPROVEMENT_") {
                self.plot_yield(mid, &owner, m.collection, m.subj_rs, &m.args);
            }
            return;
        }
        let subjects = self.collect(m.collection, &owner);
        self.owner = Some(owner.clone());
        for s in subjects {
            if let Some(srs) = m.subj_rs {
                if self.reqset(srs, &s) != Some(true) {
                    continue;
                }
            }
            if effect == Some(s!("EFFECT_ATTACH_MODIFIER")) {
                if let Some(inner) = m.args.get("ModifierId").filter(|x| !x.is_empty()) {
                    self.apply(inner, s, source, depth + 1);
                }
            } else if let Some(eff) = effect {
                self.effects.push(eff, Eff { subj: s, args: m.args.clone(), mid, source });
            }
        }
    }

    fn plot_yield(&mut self, _mid: Sym, owner: &Subj, collection: Option<Sym>, subj_rs: Option<Sym>, args: &Args) {
        let (Some(yt), Some(amt)) = (args.get("YieldType"), args.get("Amount")) else { return };
        let amount = amt.with(|s| s.trim().parse::<f64>().unwrap_or(0.0));
        let (reqs, mut test_any) = match subj_rs {
            Some(rs) => {
                let a = self.r.requirement_set(rs);
                (a.0.clone(), a.1)
            }
            None => (vec![], false),
        };
        let mut reqs = reqs;
        let mut at = None;
        let coll = collection.unwrap_or(Sym(0));
        let cities: Vec<Option<Sym>> = if coll == s!("COLLECTION_CITY_PLOT_YIELDS") {
            vec![city_of(owner)]
        } else if coll == s!("COLLECTION_PLAYER_PLOT_YIELDS") || coll == s!("COLLECTION_ALL_PLOT_YIELDS") {
            self.cities.iter().map(|c| Some(c.name)).collect()
        } else if coll == s!("COLLECTION_SINGLE_PLOT_YIELDS") {
            let Some(xy) = self.xy_of(owner) else { return };
            let Some(&c) = self.plot_city.get(&xy) else { return };
            at = Some(xy);
            if test_any {
                if self.reqset(subj_rs.unwrap(), &Subj::Plot(c, xy.0, xy.1)) != Some(true) {
                    return;
                }
                reqs = vec![];
                test_any = false;
            }
            vec![Some(c)]
        } else {
            return;
        };
        for city in cities {
            let Some(city) = city else { continue };
            let mut plot_reqs = Vec::new();
            let mut results = Vec::new();
            let mut unknown = false;
            for q in &reqs {
                if yields::is_plot_requirement(q.rtype) {
                    plot_reqs.push(q.clone());
                    continue;
                }
                match self.requirement(q.rtype, &q.args, &Subj::City(city)) {
                    None => {
                        unknown = true;
                        break;
                    }
                    Some(v) => results.push(v != q.inverse),
                }
            }
            if unknown {
                continue;
            }
            let t_any;
            if test_any {
                if results.iter().any(|&x| x) {
                    plot_reqs.clear();
                    t_any = false;
                } else if !plot_reqs.is_empty() {
                    t_any = true;
                } else {
                    continue;
                }
            } else if results.iter().all(|&x| x) {
                t_any = false;
            } else {
                continue;
            }
            self.plot_effects.entry(city).or_default().push(PlotMod { yt, amount, reqs: plot_reqs, test_any: t_any, at });
        }
    }

    fn xy_of(&self, s: &Subj) -> Option<XY> {
        match s {
            Subj::District(_, x, y) | Subj::Plot(_, x, y) => Some((*x, *y)),
            Subj::City(n) => self.city_by_name(*n).map(|c| c.center),
            _ => None,
        }
    }

    fn collect(&self, collection: Option<Sym>, owner: &Subj) -> Vec<Subj> {
        let c = collection.unwrap_or(Sym(0));
        let name = |s: &str| ss(s) == c;
        let cities = || self.cities.iter().map(|c| Subj::City(c.name)).collect::<Vec<_>>();
        if let Subj::Cs(_) = owner {
            if name("COLLECTION_ALL_PLAYERS") || name("COLLECTION_MAJOR_PLAYERS") {
                return vec![Subj::Player];
            }
            if name("COLLECTION_OWNER") {
                return vec![owner.clone()];
            }
            return vec![];
        }
        if name("COLLECTION_OWNER") {
            return vec![owner.clone()];
        }
        if name("COLLECTION_PLAYER_CITIES") || name("COLLECTION_ALL_CITIES") || name("COLLECTION_PLAYER_BUILT_CITIES") {
            return cities();
        }
        if name("COLLECTION_PLAYER_CAPITAL_CITY") {
            return self.cities.iter().filter(|c| c.capital).map(|c| Subj::City(c.name)).collect();
        }
        if name("COLLECTION_OWNER_CITY") {
            return city_of(owner).map(|n| vec![Subj::City(n)]).unwrap_or_default();
        }
        if name("COLLECTION_PLAYER_DISTRICTS") || name("COLLECTION_ALL_DISTRICTS") {
            return self.cities.iter().flat_map(|c| c.districts.iter().filter(|d| d.complete).map(move |d| Subj::District(c.name, d.x, d.y))).collect();
        }
        if name("COLLECTION_CITY_DISTRICTS") {
            let Some(n) = city_of(owner) else { return vec![] };
            return self.city_by_name(n).map(|c| c.districts.iter().filter(|d| d.complete).map(|d| Subj::District(n, d.x, d.y)).collect()).unwrap_or_default();
        }
        if name("COLLECTION_ALL_PLAYERS") || name("COLLECTION_MAJOR_PLAYERS") {
            return vec![Subj::Player];
        }
        if name("COLLECTION_PLAYER_UNITS") || name("COLLECTION_PLAYER_TRAINED_UNITS") || name("COLLECTION_ALL_UNITS") {
            return ECON_UNITS.iter().map(|u| Subj::Unit(ss(u), None)).collect();
        }
        if name("COLLECTION_CITY_TRAINED_UNITS") {
            return city_of(owner).map(|n| ECON_UNITS.iter().map(|u| Subj::Unit(ss(u), Some(n))).collect()).unwrap_or_default();
        }
        vec![]
    }

    // ---------------------------------------------------------------- requirements
    pub fn reqset(&mut self, rs: Sym, s: &Subj) -> Option<bool> {
        let set = self.r.requirement_set(rs);
        let mut results = Vec::with_capacity(set.0.len());
        for q in &set.0 {
            let v = self.requirement(q.rtype, &q.args, s)?;
            results.push(v != q.inverse);
        }
        Some(if set.1 { results.iter().any(|&x| x) } else { results.iter().all(|&x| x) })
    }

    pub fn city(&self, s: &Subj) -> Option<&City> {
        city_of(s).and_then(|n| self.city_by_name(n))
    }

    fn district(&self, s: &Subj) -> Option<&District> {
        if let Subj::District(n, x, y) = s {
            return self.city_by_name(*n).and_then(|c| c.districts.iter().find(|d| d.x == *x && d.y == *y));
        }
        None
    }

    pub fn plot(&self, s: &Subj) -> Option<Plot> {
        let xy = self.xy_of(s)?;
        let mut p = self.plots.get(&xy)?.clone();
        if let Subj::City(_) = s {
            p.district_type = Some(s!("DISTRICT_CITY_CENTER"));
        }
        Some(p)
    }

    fn requirement(&mut self, rtype: Sym, a: &Args, s: &Subj) -> Option<bool> {
        let rt = rtype;
        let r = self.r;
        let pl = &self.env.p;
        macro_rules! city {
            () => {
                match self.city(s) {
                    Some(c) => c,
                    None => return None,
                }
            };
        }
        let v: Option<bool> = if (rt == s!("REQUIREMENT_PLAYER_HAS_TECHNOLOGY")) {
            Some(a.get("TechnologyType").map(|t| self.env.techs.contains(&t)).unwrap_or(false))
        } else if (rt == s!("REQUIREMENT_UNIT_TYPE_MATCHES")) {
            if let Subj::Unit(u, _) = s { Some(Some(*u) == a.get("UnitType")) } else { None }
        } else if (rt == s!("REQUIREMENT_UNIT_PROMOTION_CLASS_MATCHES")) {
            if let Subj::Unit(u, _) = s { Some(r.lookup_sym("Units", "UnitType", *u, "PromotionClass") == a.get("UnitPromotionClass")) } else { None }
        } else if (rt == s!("REQUIREMENT_UNIT_DOMAIN_MATCHES")) {
            if let Subj::Unit(u, _) = s { Some(r.lookup_sym("Units", "UnitType", *u, "Domain") == a.get("UnitDomain")) } else { None }
        } else if (rt == s!("REQUIREMENT_UNIT_TAG_MATCHES")) {
            if let Subj::Unit(u, _) = s { Some(a.get("Tag").map(|t| r.tags_of(Some(*u)).contains(&t)).unwrap_or(false)) } else { None }
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_CIVIC")) {
            Some(a.get("CivicType").map(|t| self.env.civics.contains(&t)).unwrap_or(false))
        } else if (rt == s!("REQUIREMENT_PLAYER_LEADER_TYPE_MATCHES")) {
            let l = if let Subj::Cs(i) = s { Some(pl.envoys[*i].leader) } else { pl.leader };
            Some(l == a.get("LeaderType"))
        } else if (rt == s!("REQUIREMENT_PLAYER_CIVILIZATION_TYPE_MATCHES")) || (rt == s!("REQUIREMENT_PLAYER_TYPE_MATCHES")) {
            let c = if let Subj::Cs(i) = s { Some(pl.envoys[*i].civilization) } else { pl.civilization };
            Some(c == a.get("CivilizationType"))
        } else if (rt == s!("REQUIREMENT_PLAYER_IS_AT_PEACE")) || (rt == s!("REQUIREMENT_PLAYER_IS_SUZERAIN_BONUS_ENABLED"))
            || (rt == s!("REQUIREMENT_CITY_IS_ORIGINAL_OWNER")) || (rt == s!("REQUIREMENT_MAP_HAS_FEATURE")) || (rt == s!("REQUIREMENT_CITY_HAS_FULL_LOYALTY"))
            || (rt == s!("REQUIREMENT_PLAYER_ALWAYS_ALLOWED_COMMEMORATION_QUEST")) || (rt == s!("REQUIREMENT_PLAYER_CAN_EVER_EARN_GREAT_PERSON_CLASS"))
            || (rt == s!("REQUIREMENT_PLAYER_OWNS_OBJECT")) || (rt == s!("REQUIREMENT_CITY_FOLLOWS_PANTHEON")) {
            Some(true)
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_GIVEN_INFLUENCE_TOKENS")) {
            let cs = if let Subj::Cs(i) = s { Some(*i) } else if let Some(Subj::Cs(i)) = &self.owner { Some(*i) } else { None };
            cs.map(|i| pl.envoys[i].count >= a.f("MinimumTokens") as i64)
        } else if (rt == s!("REQUIREMENT_PLAYER_IS_SUZERAIN")) {
            let cs = if let Subj::Cs(i) = s { Some(*i) } else if let Some(Subj::Cs(i)) = &self.owner { Some(*i) } else { None };
            cs.map(|i| pl.envoys[i].suzerain)
        } else if (rt == s!("REQUIREMENT_PLAYER_ERA_AT_LEAST")) {
            match (r.era_index(self.env.game_era), a.get("EraType").and_then(|e| r.era_index(e))) {
                (Some(cur), Some(want)) => Some(cur >= want),
                _ => None,
            }
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_RESOURCE_VISIBILITY")) {
            Some(a.get("ResourceType").map(|res| r.resource_revealed(res, &self.env.techs)).unwrap_or(true))
        } else if (rt == s!("REQUIREMENT_REQUIREMENTSET_IS_MET")) {
            let rs = a.get("RequirementSetId")?;
            self.reqset(rs, s)
        } else if (rt == s!("REQUIREMENT_DISTRICT_TYPE_MATCHES")) {
            if let Some(d) = self.district(s) {
                Some(Some(d.dtype) == a.get("DistrictType"))
            } else {
                self.plot(s).map(|p| p.district_type == a.get("DistrictType"))
            }
        } else if (rt == s!("REQUIREMENT_CITY_HAS_GOVERNOR_WITH_X_TITLES")) {
            let c = city!();
            Some(governor_titles(pl, c, a.is("Established", &["1"])) >= a.f("Amount") as usize)
        } else if (rt == s!("REQUIREMENT_CITY_HAS_GOVERNOR")) {
            let c = city!();
            Some(governor_titles(pl, c, true) > 0)
        } else if (rt == s!("REQUIREMENT_CITY_HAS_X_POPULATION")) {
            let c = city!();
            Some(c.population as f64 >= a.f("Amount"))
        } else if (rt == s!("REQUIREMENT_CITY_IS_CAPITAL")) {
            let c = city!();
            Some(c.capital)
        } else if (rt == s!("REQUIREMENT_CITY_FOLLOWS_RELIGION")) {
            None // no religion in the plan
        } else if (rt == s!("REQUIREMENT_PLOT_IS_FRESH_WATER")) {
            if let (Subj::City(_), Some(c)) = (s, self.city(s)) {
                Some(crate::growth::has_fresh_water(c))
            } else {
                let p = self.plot(s)?;
                Some(p.river || crate::geo::neighbour_xy(p.x, p.y).iter().any(|xy| self.plots.get(xy).map(|q| q.lake || q.feature == Some(s!("FEATURE_OASIS"))).unwrap_or(false)))
            }
        } else if (rt == s!("REQUIREMENT_PLOT_ADJACENT_TO_COAST")) {
            let p = self.plot(s)?;
            Some(crate::geo::neighbour_xy(p.x, p.y).iter().any(|xy| self.plots.get(xy).map(|q| q.terrain == s!("TERRAIN_COAST") && !q.lake).unwrap_or(false)))
        } else if (rt == s!("REQUIREMENT_PLOT_IS_COAST")) {
            self.plot(s).map(|p| p.terrain == s!("TERRAIN_COAST"))
        } else if (rt == s!("REQUIREMENT_PLOT_NEAR_CAPITAL")) {
            None
        } else if (rt == s!("REQUIREMENT_GAME_ERA_IS")) {
            match (r.era_index(self.env.game_era), a.get("EraType").and_then(|e| r.era_index(e))) {
                (Some(cur), Some(want)) => Some(if self.permanent { cur >= want } else { cur == want }),
                _ => None,
            }
        } else if (rt == s!("REQUIREMENT_GAME_ERA_ATLEAST_EXPANSION")) {
            match (r.era_index(self.env.game_era), a.get("EraType").and_then(|e| r.era_index(e))) {
                (Some(cur), Some(want)) => Some(cur >= want),
                _ => None,
            }
        } else if (rt == s!("REQUIREMENT_PLAYER_FOUNDED_OUR_RELIGION")) || (rt == s!("REQUIREMENT_PLAYER_IS_RELIGION_FOUNDER"))
            || (rt == s!("REQUIREMENT_PLAYER_FOUNDED_RELIGION_WITH_BELIEF")) {
            Some(false) // the plan founds no religion
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_BUILDING")) {
            Some(a.get("BuildingType").map(|b| self.cities.iter().any(|c| c.buildings.contains(&b))).unwrap_or(false))
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_DISTRICT")) {
            Some(a.get("DistrictType").map(|d| self.cities.iter().any(|c| c.districts.iter().any(|x| x.dtype == d && x.complete))).unwrap_or(false))
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_IMPROVEMENT")) {
            let i = a.get("ImprovementType");
            Some(self.cities.iter().any(|c| c.owned.iter().any(|p| p.improvement == i)))
        } else if (rt == s!("REQUIREMENT_PLOT_ADJACENT_BUILDING_TYPE_MATCHES")) {
            let p = self.plot(s)?;
            let b = a.get("BuildingType");
            let mut spots: Vec<XY> = self.plots.iter().filter(|(_, q)| q.wonder.is_some() && q.wonder == b && q.wonder_complete == Some(true)).map(|(xy, _)| *xy).collect();
            if spots.is_empty() {
                spots = self.cities.iter().filter(|c| b.map(|b| c.buildings.contains(&b)).unwrap_or(false)).map(|c| c.center).collect();
            }
            let lo = a.get("MinRange").filter(|x| !x.is_empty()).map(|x| x.with(|s| s.parse::<i32>().unwrap_or(0))).unwrap_or(0);
            let hi = a.get("MaxRange").filter(|x| !x.is_empty()).map(|x| x.with(|s| s.parse::<i32>().unwrap_or(1))).unwrap_or(1);
            Some(spots.iter().any(|&xy| { let d = crate::geo::hex_distance((p.x, p.y), xy); lo <= d && d <= hi }))
        } else if (rt == s!("REQUIREMENT_PLAYER_IS_MINOR")) {
            Some(matches!(s, Subj::Cs(_)))
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_PANTHEON")) {
            Some(pl.pantheon.is_some())
        } else if rt == s!("REQUIREMENT_PLAYER_HAS_GOLDEN_AGE") {
            Some(pl.age == Some(s!("GOLDEN")) || pl.age == Some(s!("HEROIC")))
        } else if rt == s!("REQUIREMENT_PLAYER_HAS_DARK_AGE") {
            Some(pl.age == Some(s!("DARK")))
        } else if rt == s!("REQUIREMENT_PLAYER_HAS_NORMAL_AGE") {
            Some(pl.age == Some(s!("NORMAL")) || pl.age.is_none())
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_FEATURE")) {
            let f = a.get("FeatureType");
            Some(self.cities.iter().any(|c| c.owned.iter().any(|p| p.feature == f)))
        } else if (rt == s!("REQUIREMENT_CITY_HAS_FEATURE")) {
            let c = city!();
            let f = a.get("FeatureType");
            Some(c.owned.iter().any(|p| p.feature == f))
        } else if (rt == s!("REQUIREMENT_PLOT_PROPERTY_MATCHES")) {
            let name = a.get("PropertyName");
            if name == Some(s!("AMANI_ESTABLISHED_CS")) {
                Some(false) // no ambassador to city-states in the plan
            } else if name == Some(s!("TRADER_TO_AMANI_CS")) {
                self.city(s).map(|_| false)
            } else {
                None
            }
        } else if (rt == s!("REQUIREMENT_PLAYER_HAS_AT_LEAST_NUMBER_CITIES")) {
            Some(self.cities.len() as f64 >= a.f("Amount"))
        } else if yields::is_plot_requirement(rt) {
            let p = self.plot(s)?;
            let c = self.city(s);
            // the appeal modifiers only for what reads appeal (they cost a pass over the city's effects)
            let amods = if rt == s!("REQUIREMENT_PLOT_IS_APPEAL_BETWEEN") || rt == s!("REQUIREMENT_REQUIREMENTSET_IS_MET") {
                c.map(|c| yields::appeal_modifiers(self, c.name))
            } else {
                None
            };
            let ctx = yields::Ctx { techs: &self.env.techs, civics: &self.env.civics, plots: Some(&self.plots), city: c, appeal_mods: amods.as_ref() };
            Some(yields::plot_requirement(r, rt, &p, a, &ctx))
        } else if yields::is_city_requirement(rt) {
            let c = city!();
            Some(yields::city_requirement(r, rt, c, a))
        } else {
            None
        };
        v
    }

    // ---------------------------------------------------------------- queries
    pub fn city_effects(&self, effect: Sym, city: Sym) -> Vec<&Eff> {
        self.effects.get(effect).map(|v| v.iter().filter(|e| e.subj == Subj::City(city)).collect()).unwrap_or_default()
    }
    pub fn player_effects(&self, effect: Sym) -> Vec<&Eff> {
        self.effects.get(effect).map(|v| v.iter().filter(|e| e.subj == Subj::Player).collect()).unwrap_or_default()
    }
    pub fn all_effects(&self, effect: Sym) -> Vec<&Eff> {
        self.effects.get(effect).map(|v| v.iter().collect()).unwrap_or_default()
    }
    /// [(district, args)] of an effect on a city's districts.
    pub fn district_effects(&self, effect: Sym, city: Sym) -> Vec<(Option<&District>, &Eff)> {
        let mut out = vec![];
        for e in self.effects.get(effect).map(|v| v.as_slice()).unwrap_or(&[]) {
            if let Subj::District(n, _, _) = &e.subj {
                if *n == city {
                    out.push((self.district(&e.subj), e));
                }
            }
        }
        out
    }
    pub fn owned_plot_effects(&self, effect: Sym, city: Sym) -> Vec<&Eff> {
        self.effects.get(effect).map(|v| v.iter().filter(|e| matches!(&e.subj, Subj::Plot(n, _, _) if *n == city)).collect()).unwrap_or_default()
    }
    pub fn unit_effects(&self, effect: Sym, unit: Sym, city: Option<Sym>) -> Vec<&Eff> {
        self.effects.get(effect).map(|v| v.iter().filter(|e| matches!(&e.subj, Subj::Unit(u, c) if *u == unit && (c.is_none() || *c == city))).collect()).unwrap_or_default()
    }

    /// Effects of one source's modifiers resolved on their own (one-time grants): [(effect, subject, args)].
    pub fn grants_of(&mut self, table: &str, column: &str, key: Sym, owner: Subj) -> Vec<(Sym, Subj, Args)> {
        let saved = std::mem::take(&mut self.effects);
        let saved_plots = std::mem::take(&mut self.plot_effects);
        for m in self.r.list_sym(table, column, key, "ModifierId") {
            self.religion = None;
            self.apply(m, owner.clone(), key, 0);
        }
        let got = std::mem::replace(&mut self.effects, saved);
        self.plot_effects = saved_plots;
        let mut out = vec![];
        for (eff, rows) in got.list {
            for e in rows {
                out.push((eff, e.subj, e.args));
            }
        }
        out
    }
}

/// Whether a modifier can produce an effect the simulator reads (itself, or what it attaches).
fn relevant(r: &Rules, mid: Sym, depth: u32) -> bool {
    thread_local!(static MEMO: std::cell::RefCell<HashMap<Sym, bool>> = Default::default());
    if let Some(v) = MEMO.with(|m| m.borrow().get(&mid).copied()) {
        return v;
    }
    let v = match r.modifier(mid) {
        None => false,
        Some(m) if m.effect == Some(s!("EFFECT_ATTACH_MODIFIER")) => {
            depth < 5 && m.args.get("ModifierId").filter(|x| !x.is_empty()).map(|inner| relevant(r, inner, depth + 1)).unwrap_or(false)
        }
        Some(m) => m.effect.map(crate::consumed::is_consumed).unwrap_or(false),
    };
    MEMO.with(|m| m.borrow_mut().insert(mid, v));
    v
}

pub fn city_of(s: &Subj) -> Option<Sym> {
    match s {
        Subj::City(n) | Subj::District(n, _, _) | Subj::Plot(n, _, _) => Some(*n),
        Subj::Unit(_, c) => *c,
        _ => None,
    }
}

pub const ECON_UNITS: [&str; 5] = ["UNIT_BUILDER", "UNIT_SETTLER", "UNIT_TRADER", "UNIT_MILITARY_ENGINEER", "UNIT_SCOUT"];

/// Is governor record g serving in this city of ours? (envs.governor_here)
pub fn governor_here(g: &Governor, city: &City, pl: &Player) -> bool {
    g.city_id == Some(city.id) && (g.city_owner_id.is_none() || g.city_owner_id == pl.player_id || pl.player_id.is_none())
}

/// Titles of the governor in the city (one per promotion, the base one included).
pub fn governor_titles(pl: &Player, city: &City, established: bool) -> usize {
    for g in pl.governors.iter().flatten() {
        if governor_here(g, city, pl) && (g.established || !established) {
            return g.promotions.len();
        }
    }
    0
}

/// Cache of engines by the state they were built for (modifiers.engine_for).
pub static LOOKUPS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub struct EngineCache<'a> {
    map: HashMap<u64, std::rc::Rc<Engine<'a>>>,
}

impl<'a> EngineCache<'a> {
    pub fn new() -> Self {
        EngineCache { map: HashMap::new() }
    }
    pub fn get(&mut self, r: &'a Rules, env: &Env, cities: &[City]) -> std::rc::Rc<Engine<'a>> {
        LOOKUPS.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let _g = crate::prof::start(12);
        let key = engine_key(env, cities, pop_thresholds(r));
        drop(_g);
        if let Some(e) = self.map.get(&key) {
            return e.clone();
        }
        if self.map.len() > engine_cache_cap() {
            self.map.clear();
        }
        let e = std::rc::Rc::new(Engine::new(r, env, cities));
        self.map.insert(key, e.clone());
        e
    }
}

/// The populations the modifiers' requirements test (REQUIREMENT_CITY_HAS_X_POPULATION): an engine only depends
/// on which of them a city has reached.
fn pop_thresholds(r: &Rules) -> &'static [i32] {
    static T: std::sync::OnceLock<Vec<i32>> = std::sync::OnceLock::new();
    T.get_or_init(|| {
        let rq = r.tab("Requirements");
        let ra = r.tab("RequirementArguments");
        let mut v: Vec<i32> = vec![];
        for i in r.scan("Requirements", "Requirements") {
            if rq.v(i, "RequirementType").sym() != Some(s!("REQUIREMENT_CITY_HAS_X_POPULATION")) {
                continue;
            }
            let id = rq.v(i, "RequirementId").sym().unwrap();
            for &j in ra.find("RequirementId", id) {
                if ra.v(j, "Name").sym() == Some(s!("Amount")) {
                    v.push(ra.v(j, "Value").f().ceil() as i32);
                }
            }
        }
        v.sort();
        v.dedup();
        v
    })
}

/// Everything an engine is built from: the whole Env but the units trained (no modifier reads them), and every
/// city but what only the citizens' assignment and the production queue use (worked, near, pop_mark, current);
/// of the population only which of the tested sizes it has reached. (Counting techs instead of naming them let two
/// states with as many techs share an engine.)
fn engine_key(env: &Env, cities: &[City], pops: &[i32]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = Fx(0);
    let p = &env.p;
    (p.civilization, p.leader, &p.policies, p.government, p.pantheon, p.player_id, p.age, &p.dedications).hash(&mut h);
    match &p.governors {
        None => 0u8.hash(&mut h),
        Some(gs) => {
            for g in gs {
                (g.gtype, &g.promotions, g.city_id, g.city_owner_id, g.established, g.ready).hash(&mut h);
            }
        }
    }
    for e in &p.envoys {
        (e.player_id, e.civilization, e.leader, e.count, e.suzerain, &e.resources).hash(&mut h);
    }
    for g in &p.great_people_used {
        (g.individual, g.cls, g.turn, g.city).hash(&mut h);
    }
    p.boosted.hash(&mut h);
    env.techs.hash(&mut h);
    env.civics.hash(&mut h);
    (env.game_era, env.prophet_on_offer, env.districts_completed).hash(&mut h);
    for c in cities {
        let reached = pops.iter().filter(|&&x| c.population >= x).count();
        (c.name, c.id, c.capital, c.center, reached, &c.buildings, &c.great_works).hash(&mut h);
        if let Some(r) = &c.religion {
            (r.majority, &r.followers).hash(&mut h);
        }
        c.owned.len().hash(&mut h);
        for q in &c.owned {
            hash_plot(q, &mut h);
        }
        for d in &c.districts {
            (d.dtype, d.x, d.y, d.complete).hash(&mut h);
            hash_plot(&d.plot, &mut h);
            for q in &d.neighbours {
                hash_plot(q, &mut h);
            }
        }
    }
    h.finish()
}

fn hash_plot(q: &Plot, h: &mut Fx) {
    use std::hash::Hash;
    (q.x, q.y, q.terrain, q.feature, q.resource, q.improvement, q.river, q.lake, q.pillaged, q.district, q.wonder).hash(h);
    for (k, v) in &q.yield_changes {
        (k, v.to_bits()).hash(h);
    }
    (q.owner, q.river_edges, q.route, q.district_type, q.district_complete, q.wonder_complete).hash(h);
}

/// FxHash (rustc's): fast, fine for cache keys.
struct Fx(u64);
impl std::hash::Hasher for Fx {
    fn finish(&self) -> u64 {
        self.0
    }
    fn write(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.write_u64(b as u64);
        }
    }
    fn write_u8(&mut self, i: u8) {
        self.write_u64(i as u64)
    }
    fn write_u32(&mut self, i: u32) {
        self.write_u64(i as u64)
    }
    fn write_i32(&mut self, i: i32) {
        self.write_u64(i as u32 as u64)
    }
    fn write_i64(&mut self, i: i64) {
        self.write_u64(i as u64)
    }
    fn write_usize(&mut self, i: usize) {
        self.write_u64(i as u64)
    }
    fn write_u64(&mut self, i: u64) {
        self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
    }
}

pub fn sorted_set(v: &BTreeSet<Sym>) -> Vec<Sym> {
    v.iter().copied().collect()
}

#[derive(Default, Clone, Debug)]
pub struct Effects {
    pub list: Vec<(Sym, Vec<Eff>)>,
    idx: HashMap<Sym, usize>,
}

impl Effects {
    pub fn push(&mut self, eff: Sym, e: Eff) {
        if let Some(&i) = self.idx.get(&eff) {
            self.list[i].1.push(e);
        } else {
            self.idx.insert(eff, self.list.len());
            self.list.push((eff, vec![e]));
        }
    }
    pub fn get(&self, eff: Sym) -> Option<&Vec<Eff>> {
        assert!(crate::consumed::is_consumed(eff), "effect {eff} is read but missing in consumed.rs");
        self.idx.get(&eff).map(|&i| &self.list[i].1)
    }
}

fn engine_cache_cap() -> usize {
    thread_local!(static CAP: usize = std::env::var("CIV6OPT_ENGINE_CACHE").ok().and_then(|x| x.parse().ok()).unwrap_or(256));
    CAP.with(|c| *c)
}
