//! The planner's state (civ6sim/plan2.py State, sim.py PlayerAcc / CityAcc) and the export-format city / plot
//! records it carries, loaded from the game data's "start" (gamedata.rs start_state).
use crate::sym::{sym, Sym};
use crate::json::Value;
use std::collections::BTreeSet;

/// A map that keeps insertion order like a Python dict (small; linear scans).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OMap<K: PartialEq + Clone, V: Clone>(pub Vec<(K, V)>);

impl<K: PartialEq + Clone, V: Clone> OMap<K, V> {
    pub fn new() -> Self {
        OMap(Vec::new())
    }
    pub fn get(&self, k: &K) -> Option<&V> {
        self.0.iter().find(|(a, _)| a == k).map(|(_, v)| v)
    }
    pub fn get_mut(&mut self, k: &K) -> Option<&mut V> {
        self.0.iter_mut().find(|(a, _)| a == k).map(|(_, v)| v)
    }
    pub fn insert(&mut self, k: K, v: V) {
        if let Some(x) = self.get_mut(&k) {
            *x = v;
        } else {
            self.0.push((k, v));
        }
    }
    pub fn entry_or(&mut self, k: K, default: V) -> &mut V {
        if let Some(i) = self.0.iter().position(|(a, _)| *a == k) {
            &mut self.0[i].1
        } else {
            self.0.push((k, default));
            &mut self.0.last_mut().unwrap().1
        }
    }
    pub fn remove(&mut self, k: &K) -> Option<V> {
        let i = self.0.iter().position(|(a, _)| a == k)?;
        Some(self.0.remove(i).1)
    }
    pub fn contains(&self, k: &K) -> bool {
        self.0.iter().any(|(a, _)| a == k)
    }
    pub fn iter(&self) -> impl Iterator<Item = &(K, V)> {
        self.0.iter()
    }
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.0.iter().map(|(k, _)| k)
    }
}

pub type XY = (i32, i32);

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Plot {
    pub x: i32,
    pub y: i32,
    pub terrain: Sym,
    pub feature: Option<Sym>,
    pub resource: Option<Sym>,
    pub improvement: Option<Sym>,
    pub river: bool,
    pub lake: bool,
    pub pillaged: bool,
    /// the export's district id on the plot (a district stands here); None on map plots
    pub district: Option<i64>,
    pub wonder: Option<Sym>,
    pub yield_changes: Vec<(Sym, f64)>,
    pub owner: Option<i64>,
    pub river_edges: u8,
    pub route: Option<i64>,
    // set where plots are viewed with their districts (plots_by_xy_of, near_fn, neighbours_with_districts)
    pub district_type: Option<Sym>,
    pub district_complete: Option<bool>,
    pub wonder_complete: Option<bool>,
    /// citizens on a worked plot
    pub workers: i32,
}

impl Plot {
    pub fn xy(&self) -> XY {
        (self.x, self.y)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct District {
    pub dtype: Sym,
    pub x: i32,
    pub y: i32,
    pub complete: bool,
    pub plot: Plot,
    pub neighbours: Vec<Plot>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CityReligion {
    pub majority: Option<Sym>,
    pub followers: Vec<(Sym, i64)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct City {
    pub name: Sym,
    pub id: i64,
    pub capital: bool,
    pub center: XY,
    pub population: i32,
    pub religion: Option<CityReligion>,
    pub worked: Vec<Plot>,
    pub owned: Vec<Plot>,
    pub near: Vec<Plot>,
    pub current: Option<Sym>,
    pub buildings: Vec<Sym>,
    pub great_works: Vec<(Sym, Sym)>,
    pub districts: Vec<District>,
    /// "_pop": the population the citizen focus was last decided for
    pub pop_mark: Option<i32>,
}

impl City {
    pub fn owned_at(&self, xy: XY) -> Option<&Plot> {
        self.owned.iter().find(|p| p.xy() == xy)
    }
    pub fn owned_at_mut(&mut self, xy: XY) -> Option<&mut Plot> {
        self.owned.iter_mut().find(|p| p.xy() == xy)
    }
    pub fn has_complete_district(&self, d: Sym) -> bool {
        self.districts.iter().any(|x| x.dtype == d && x.complete)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Governor {
    pub gtype: Sym,
    pub promotions: Vec<Sym>,
    pub city_id: Option<i64>,
    pub city_owner_id: Option<i64>,
    pub established: bool,
    pub ready: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Envoy {
    pub player_id: i64,
    pub civilization: Sym,
    pub leader: Sym,
    pub count: i64,
    pub suzerain: bool,
    /// the player is at war with the city-state (its envoys give no bonus)
    pub at_war: bool,
    pub resources: Vec<Sym>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct GpUse {
    pub individual: Sym,
    pub cls: Sym,
    pub turn: i64,
    pub city: Option<Sym>,
}

/// A city in the route endpoint format (ours, or another player's from the recorded game).
#[derive(Clone, Debug, PartialEq)]
pub struct Endpoint {
    pub name: Sym,
    pub id: i64,
    pub player_id: Option<i64>,
    pub civilization: Option<Sym>,
    pub leader: Option<Sym>,
    pub x: i32,
    pub y: i32,
    pub districts: Vec<Sym>,
    pub tech_count: i64,
    pub civic_count: i64,
    pub plots: Vec<(Sym, Option<Sym>, Option<Sym>)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Route {
    pub origin: Endpoint,
    pub dest: Endpoint,
}

#[derive(Clone, Debug)]
pub struct Player {
    pub civilization: Option<Sym>,
    pub leader: Option<Sym>,
    pub policies: Vec<Sym>,
    pub government: Option<Sym>,
    pub pantheon: Option<Sym>,
    pub governors: Option<Vec<Governor>>,
    pub envoys: Vec<Envoy>,
    pub player_id: Option<i64>,
    pub great_people_used: Vec<GpUse>,
    pub boosted: BTreeSet<Sym>,
    /// the age of the current era ("GOLDEN", "NORMAL", "DARK", "HEROIC"; from the record) and its dedications
    pub age: Option<Sym>,
    pub dedications: Vec<Sym>,
}

/// player_env(): the player plus what the modifiers read (techs, civics, era, envoys of this turn).
#[derive(Clone, Debug)]
pub struct Env {
    pub p: Player,
    pub techs: BTreeSet<Sym>,
    pub civics: BTreeSet<Sym>,
    pub units_trained: OMap<Sym, i64>,
    pub game_era: Sym,
    pub prophet_on_offer: bool,
    /// set for item costs: completed specialty districts snapshot (sim.step's _districts_completed)
    pub districts_completed: Option<i64>,
    /// luxury types the player has from outside its land this turn (traded or given by allies; --team)
    pub extra_luxuries: i64,
    /// replay: the luxury amenities each city gets, as the save holds them (they replace the model's own share)
    pub luxuries_given: Option<Vec<(Sym, f64)>>,
    /// allied players (Scenario.allies)
    pub allies: Vec<i64>,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct CityAcc {
    pub population: i32,
    pub food_store: f64,
    pub progress: OMap<Sym, f64>,
    pub overflow: f64,
    pub locked_cost: OMap<Sym, f64>,
    /// the item a chop finished during this turn (it completes at the step; the turn's production goes to the next)
    pub chopped: Option<Sym>,
    /// production waiting for the next item, which applies its own percent to it
    pub raw: f64,
}

#[derive(Clone, Debug, Default)]
pub struct PlayerAcc {
    pub gold: f64,
    pub tech_progress: OMap<Sym, f64>,
    pub civic_progress: OMap<Sym, f64>,
    pub techs: BTreeSet<Sym>,
    pub civics: BTreeSet<Sym>,
    pub units_trained: OMap<Sym, i64>,
    pub faith: f64,
    pub tech_overflow: f64,
    pub civic_overflow: f64,
    pub cities: OMap<Sym, CityAcc>,
    pub districts_snapshot: Option<i64>,
}

// ---------------------------------------------------------------- decisions
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Ent {
    S(Sym),
    I(i64),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Key {
    pub kind: Sym,
    pub ent: Ent,
    pub n: i64,
}

/// A choice: a type name, a plot, or one of the tuple forms of plan2.py (builder actions, purchases, governor
/// picks). Floats (prices) are kept as bits so choices compare exactly.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Choice {
    S(Sym),
    Xy(i32, i32),
    /// ("improve" | "plant" | "harvest" | "rush", city, (x, y), what)
    Act(Sym, Sym, i32, i32, Sym),
    Wait,
    /// ("none",): no purchase / no governor
    Nothing,
    /// ("item", city, item, price, currency)
    BuyItem(Sym, Sym, u64, Sym),
    /// ("tile", city, gold, (x, y))
    BuyTile(Sym, u64, i32, i32),
    /// ("appoint", governor, city) / ("promote", governor, promotion)
    Appoint(Sym, Sym),
    Promote(Sym, Sym),
    /// ("route", origin, destination): a trader's route, from the city it makes its home
    Route(Sym, Sym),
}

impl Choice {
    pub fn sym(&self) -> Option<Sym> {
        if let Choice::S(s) = self { Some(*s) } else { None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Builder {
    pub charges: i64,
    pub at: XY,
    pub ready: i64,
    pub id: Option<i64>,
    pub pending: Option<Choice>,
    /// set when the builder chose to wait: what it waits to change (Planner::idle_mark)
    pub idle: Option<IdleMark>,
}

pub type IdleMark = ([usize; 7], i64, Vec<Option<Sym>>);

#[derive(Clone, Debug, PartialEq)]
pub struct Settler {
    pub from: Sym,
    pub target: Option<XY>,
    pub name: Option<Sym>,
    pub arrive: i64,
    pub id: i64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Trader {
    pub origin: Sym,
    pub dest: Option<Sym>,
    pub info: Option<Endpoint>,
    pub until: Option<i64>,
    /// the turn the route starts (after the trader walked to a new home city)
    pub start: i64,
    /// replay: the record lost the trader when the route ends
    pub lost: bool,
}

#[derive(Clone, Debug)]
pub struct State {
    pub turn: i64,
    pub acc: PlayerAcc,
    pub cities: Vec<City>,
    pub player: Player,
    pub builders: Vec<Builder>,
    pub settlers: Vec<Settler>,
    pub traders: Vec<Trader>,
    pub units: OMap<Sym, i64>,
    pub research: Option<Sym>,
    pub civic: Option<Sym>,
    pub policy_dirty: bool,
    /// the game era the dedications were chosen for
    pub era_mark: Option<Sym>,
    pub known_govs: BTreeSet<Sym>,
    /// governments the player had before (their legacy policy cards are available)
    pub past_govs: BTreeSet<Sym>,
    pub border: OMap<Sym, (f64, i64)>,
    pub focus: OMap<Sym, Sym>,
    pub free_building_given: BTreeSet<Sym>,
    pub gov_plaza: bool,
    pub score: f64,
    pub last_output: [f64; 6],
    /// each city's part of last_output
    pub city_output: Vec<(Sym, [f64; 6])>,
    pub log: Vec<(Key, i64, Option<Choice>)>,
    pub counters: OMap<(Sym, Ent), i64>,
    pub builder_counter: i64,
    pub settler_counter: i64,
    pub bought: Vec<(Sym, Sym)>,
    pub tiles_bought: i64,
    /// the last turn whose recorded gifts (Scenario: gold, faith, units) the plan has received
    pub gifts_through: i64,
    /// improvements built this turn (city, plot, improvement): their yields count from the next turn (the records)
    pub improving: Vec<(Sym, XY, Sym)>,
    pub gp: OMap<Sym, f64>,
    pub gp_ours: Vec<Sym>,
    pub gp_pending: Vec<GpUse>,
    /// requirement / reward terms recorded at their own turns (crate::milestones::Milestones::record)
    pub req_at: Vec<(u32, f64)>,
}

impl State {
    pub fn city(&self, name: Sym) -> Option<&City> {
        self.cities.iter().find(|c| c.name == name)
    }
    pub fn city_mut(&mut self, name: Sym) -> Option<&mut City> {
        self.cities.iter_mut().find(|c| c.name == name)
    }
    pub fn cacc(&self, name: Sym) -> &CityAcc {
        self.acc.cities.get(&name).unwrap()
    }
    pub fn cacc_mut(&mut self, name: Sym) -> &mut CityAcc {
        self.acc.cities.get_mut(&name).unwrap()
    }
}

// ---------------------------------------------------------------- JSON loading
pub fn osym(v: &Value) -> Option<Sym> {
    v.as_str().map(sym)
}
fn obool(v: &Value) -> bool {
    match v {
        Value::Bool(b) => *b,
        Value::Num(n) => *n != 0.0,
        _ => false,
    }
}
pub fn oi64(v: &Value) -> Option<i64> {
    v.as_i64().or_else(|| v.as_f64().map(|f| f as i64))
}

pub fn plot_from(v: &Value) -> Plot {
    Plot {
        x: v["x"].as_i64().unwrap() as i32,
        y: v["y"].as_i64().unwrap() as i32,
        terrain: osym(&v["terrain"]).unwrap_or(Sym(0)),
        feature: osym(&v["feature"]),
        resource: osym(&v["resource"]),
        improvement: osym(&v["improvement"]),
        river: obool(&v["river"]),
        lake: obool(&v["lake"]),
        pillaged: obool(&v["pillaged"]),
        district: oi64(&v["district"]),
        wonder: osym(&v["wonder"]),
        yield_changes: v["yieldChanges"].as_object().map(|o| o.iter().map(|(k, x)| (sym(k), x.as_f64().unwrap_or(0.0))).collect()).unwrap_or_default(),
        owner: oi64(&v["owner"]),
        river_edges: v["riverEdges"].as_u64().unwrap_or(0) as u8,
        route: oi64(&v["route"]),
        district_type: osym(&v["districtType"]),
        district_complete: v["districtComplete"].as_bool(),
        wonder_complete: v["wonderComplete"].as_bool(),
        workers: v["workers"].as_i64().unwrap_or(1) as i32,
    }
}

fn district_from(v: &Value) -> District {
    District {
        dtype: osym(&v["type"]).unwrap(),
        x: v["x"].as_i64().unwrap() as i32,
        y: v["y"].as_i64().unwrap() as i32,
        complete: obool(&v["complete"]),
        plot: plot_from(&v["plot"]),
        neighbours: v["neighbours"].as_array().map(|a| a.iter().map(plot_from).collect()).unwrap_or_default(),
    }
}

pub fn city_from(v: &Value) -> City {
    let arr = |k: &str| v[k].as_array().map(|a| a.iter().map(plot_from).collect()).unwrap_or_default();
    City {
        name: osym(&v["name"]).unwrap(),
        id: oi64(&v["id"]).unwrap_or(0),
        capital: obool(&v["capital"]),
        center: (v["center"]["x"].as_i64().unwrap() as i32, v["center"]["y"].as_i64().unwrap() as i32),
        population: v["population"].as_i64().unwrap_or(1) as i32,
        religion: if v["religion"].is_object() {
            Some(CityReligion {
                majority: osym(&v["religion"]["majority"]),
                followers: v["religion"]["followers"].as_object().map(|o| o.iter().map(|(k, x)| (sym(k), x.as_i64().unwrap_or(0))).collect()).unwrap_or_default(),
            })
        } else {
            None
        },
        worked: arr("worked"),
        owned: arr("owned"),
        near: arr("near"),
        current: osym(&v["production"]["current"]),
        buildings: v["buildings"].as_array().map(|a| a.iter().filter_map(osym).collect()).unwrap_or_default(),
        great_works: v["greatWorks"].as_array().map(|a| a.iter().filter_map(|x| {
            let a = x.as_array()?;
            Some((osym(&a[0])?, osym(&a[1])?))
        }).collect()).unwrap_or_default(),
        districts: v["districts"].as_array().map(|a| a.iter().map(district_from).collect()).unwrap_or_default(),
        pop_mark: v["_pop"].as_i64().map(|x| x as i32),
    }
}

pub fn governor_from(v: &Value) -> Governor {
    Governor {
        gtype: osym(&v["type"]).unwrap(),
        promotions: v["promotions"].as_array().map(|a| a.iter().filter_map(osym).collect()).unwrap_or_default(),
        city_id: oi64(&v["cityId"]),
        city_owner_id: oi64(&v["cityOwnerId"]),
        established: obool(&v["established"]),
        ready: oi64(&v["ready"]).unwrap_or(0),
    }
}

pub fn envoy_from(v: &Value) -> Envoy {
    Envoy {
        player_id: oi64(&v["playerId"]).unwrap_or(-1),
        civilization: osym(&v["civilization"]).unwrap_or(Sym(0)),
        leader: osym(&v["leader"]).unwrap_or(Sym(0)),
        count: oi64(&v["count"]).unwrap_or(0),
        suzerain: obool(&v["suzerain"]),
        at_war: obool(&v["atWar"]),
        resources: v["resources"].as_array().map(|a| a.iter().filter_map(osym).collect()).unwrap_or_default(),
    }
}

pub fn endpoint_from(v: &Value) -> Endpoint {
    Endpoint {
        name: osym(&v["name"]).unwrap_or(Sym(0)),
        id: oi64(&v["id"]).unwrap_or(0),
        player_id: oi64(&v["playerId"]),
        civilization: osym(&v["civilization"]),
        leader: osym(&v["leader"]),
        x: v["x"].as_i64().unwrap_or(0) as i32,
        y: v["y"].as_i64().unwrap_or(0) as i32,
        districts: v["districts"].as_array().map(|a| a.iter().filter_map(osym).collect()).unwrap_or_default(),
        tech_count: oi64(&v["techCount"]).unwrap_or(0),
        civic_count: oi64(&v["civicCount"]).unwrap_or(0),
        plots: v["plots"].as_array().map(|a| a.iter().map(|p| {
            let p = p.as_array().unwrap();
            (osym(&p[0]).unwrap_or(Sym(0)), osym(&p[1]), osym(&p[2]))
        }).collect()).unwrap_or_default(),
    }
}

pub fn omap_f(v: &Value) -> OMap<Sym, f64> {
    OMap(v.as_object().map(|o| o.iter().map(|(k, x)| (sym(k), x.as_f64().unwrap_or(0.0))).collect()).unwrap_or_default())
}
pub fn omap_i(v: &Value) -> OMap<Sym, i64> {
    OMap(v.as_object().map(|o| o.iter().map(|(k, x)| (sym(k), oi64(x).unwrap_or(0))).collect()).unwrap_or_default())
}
pub fn symset(v: &Value) -> BTreeSet<Sym> {
    v.as_array().map(|a| a.iter().filter_map(osym).collect()).unwrap_or_default()
}

/// The start state (gamedata.rs start_state). Decision logs and counters start empty.
pub fn state_from(v: &Value) -> State {
    let a = &v["acc"];
    let acc = PlayerAcc {
        gold: a["gold"].as_f64().unwrap_or(0.0),
        tech_progress: omap_f(&a["tech_progress"]),
        civic_progress: omap_f(&a["civic_progress"]),
        techs: symset(&a["techs"]),
        civics: symset(&a["civics"]),
        units_trained: omap_i(&a["units_trained"]),
        faith: a["faith"].as_f64().unwrap_or(0.0),
        tech_overflow: a["tech_overflow"].as_f64().unwrap_or(0.0),
        civic_overflow: a["civic_overflow"].as_f64().unwrap_or(0.0),
        cities: OMap(a["cities"].as_object().unwrap().iter().map(|(k, c)| (sym(k), CityAcc {
            population: c["population"].as_i64().unwrap() as i32,
            food_store: c["food_store"].as_f64().unwrap_or(0.0),
            progress: omap_f(&c["progress"]),
            overflow: c["overflow"].as_f64().unwrap_or(0.0),
            locked_cost: omap_f(&c["locked_cost"]),
            ..Default::default()
        })).collect()),
        districts_snapshot: oi64(&a["districts_snapshot"]),
    };
    let p = &v["player"];
    let player = Player {
        civilization: osym(&p["civilization"]),
        leader: osym(&p["leader"]),
        policies: p["policies"].as_array().map(|x| x.iter().filter_map(osym).collect()).unwrap_or_default(),
        government: osym(&p["government"]),
        pantheon: osym(&p["pantheon"]),
        governors: p["governors"].as_array().map(|x| x.iter().map(governor_from).collect()),
        envoys: p["envoys"].as_array().map(|x| x.iter().map(envoy_from).collect()).unwrap_or_default(),
        player_id: oi64(&p["playerId"]),
        great_people_used: vec![],
        boosted: BTreeSet::new(),
        age: osym(&p["age"]),
        dedications: p["dedications"].as_array().map(|x| x.iter().filter_map(osym).collect()).unwrap_or_default(),
    };
    State {
        turn: v["turn"].as_i64().unwrap(),
        acc,
        cities: v["cities"].as_array().unwrap().iter().map(city_from).collect(),
        player,
        builders: v["builders"].as_array().unwrap().iter().map(|b| Builder {
            charges: oi64(&b["charges"]).unwrap_or(3),
            at: (b["at"][0].as_i64().unwrap() as i32, b["at"][1].as_i64().unwrap() as i32),
            ready: oi64(&b["ready"]).unwrap_or(0),
            id: None,
            pending: None,
            idle: None,
        }).collect(),
        settlers: vec![],
        traders: vec![],
        units: omap_i(&v["units"]),
        research: osym(&v["research"]),
        civic: osym(&v["civic"]),
        policy_dirty: obool(&v["policy_dirty"]),
        era_mark: None,
        known_govs: symset(&v["known_govs"]),
        past_govs: BTreeSet::new(),
        border: OMap(v["border"].as_object().unwrap().iter().map(|(k, b)| (sym(k), (b[0].as_f64().unwrap_or(0.0), oi64(&b[1]).unwrap_or(0)))).collect()),
        focus: OMap(v["focus"].as_object().unwrap().iter().map(|(k, f)| (sym(k), osym(f).unwrap())).collect()),
        free_building_given: symset(&v["free_building_given"]),
        gov_plaza: obool(&v["gov_plaza"]),
        score: 0.0,
        last_output: [0.0; 6],
        city_output: vec![],
        log: vec![],
        counters: OMap::new(),
        builder_counter: 0,
        settler_counter: 0,
        bought: vec![],
        tiles_bought: 0,
        gifts_through: v["turn"].as_i64().unwrap() - 1,
        gp: OMap::new(),
        gp_ours: vec![],
        gp_pending: vec![],
        improving: vec![],
        req_at: vec![],
    }
}

/// The state at a recorded turn, from that turn's save (the export row; plan2.FullPlanner.initial_state): what the
/// save does not hold starts empty (production and research overflow, border culture, locked district costs, routes
/// and great people, which Planner::state_at adds).
pub fn state_from_row(d: &Value) -> State {
    let p = &d["player"];
    let cities: Vec<City> = d["cities"].as_array().unwrap().iter().map(|c| {
        let mut x = city_from(c);
        x.current = None;
        x
    }).collect();
    let acc = PlayerAcc {
        gold: p["gold"].as_f64().unwrap_or(0.0),
        tech_progress: omap_f(&p["techProgress"]),
        civic_progress: omap_f(&p["civicProgress"]),
        techs: symset(&p["techs"]),
        civics: symset(&p["civics"]),
        units_trained: omap_i(&p["unitsTrained"]),
        faith: p["faithStock"].as_f64().or(p["faith"].as_f64()).unwrap_or(0.0),
        tech_overflow: 0.0,
        civic_overflow: 0.0,
        cities: OMap(d["cities"].as_array().unwrap().iter().map(|c| (osym(&c["name"]).unwrap(), CityAcc {
            population: c["population"].as_i64().unwrap_or(1) as i32,
            food_store: c["foodStore"].as_f64().unwrap_or(0.0),
            progress: omap_f(&c["production"]["progress"]),
            // production carried over from a finished item (whole units in the save; absent in older exports)
            overflow: c["production"]["heldOverflow"].as_f64().unwrap_or(0.0),
            locked_cost: OMap::new(),
            ..Default::default()
        })).collect()),
        districts_snapshot: None,
    };
    let player = Player {
        civilization: osym(&p["civilization"]),
        leader: osym(&p["leader"]),
        policies: p["policies"].as_array().map(|x| x.iter().filter_map(osym).collect()).unwrap_or_default(),
        government: osym(&p["government"]).or(Some(crate::sym::sym("GOVERNMENT_CHIEFDOM"))),
        pantheon: osym(&p["pantheon"]),
        governors: p["governors"].as_array().map(|x| x.iter().map(governor_from).collect()),
        envoys: p["envoys"].as_array().map(|x| x.iter().map(envoy_from).collect()).unwrap_or_default(),
        player_id: oi64(&p["playerId"]),
        // recruited great people: used at once (their lasting effects come from here)
        great_people_used: p["greatPeopleRecruited"].as_array().map(|a| a.iter().filter_map(|g| Some(GpUse {
            individual: osym(&g["individual"])?, cls: osym(&g["cls"]).unwrap_or(Sym(0)), turn: oi64(&g["turn"]).unwrap_or(0), city: None,
        })).collect()).unwrap_or_default(),
        boosted: symset(&p["techsBoosted"]).into_iter().chain(symset(&p["civicsInspired"])).collect(),
        age: osym(&p["age"]),
        dedications: p["dedications"].as_array().map(|x| x.iter().filter_map(osym).collect()).unwrap_or_default(),
    };
    let mut units = OMap::new();
    let mut builders = vec![];
    let turn = d["turn"].as_i64().unwrap();
    for u in d["units"].as_array().map(|a| a.as_slice()).unwrap_or(&[]) {
        let Some(t) = osym(&u["type"]) else { continue };
        if t.as_string() == "UNIT_BUILDER" {
            builders.push(Builder { charges: oi64(&u["charges"]).unwrap_or(3), at: (oi64(&u["x"]).unwrap_or(0) as i32, oi64(&u["y"]).unwrap_or(0) as i32),
                                    ready: turn, id: None, pending: None, idle: None });
        } else if !["UNIT_SETTLER", "UNIT_TRADER"].contains(&t.as_string().as_str()) {
            *units.entry_or(t, 0) += 1;
        }
    }
    let names: Vec<Sym> = cities.iter().map(|c| c.name).collect();
    State {
        turn,
        acc,
        border: OMap(cities.iter().map(|c| (c.name, (0.0, c.owned.len() as i64 - 7))).collect()),
        focus: OMap(names.iter().map(|n| (*n, crate::sym::sym("balanced"))).collect()),
        // Rome's free building (Planner::rome_free_buildings): given once Early Empire is done
        free_building_given: if p["civics"].as_array().map(|a| a.iter().any(|x| x.as_str() == Some("CIVIC_EARLY_EMPIRE"))).unwrap_or(false) {
            names.iter().copied().collect()
        } else {
            BTreeSet::new()
        },
        gov_plaza: cities.iter().any(|c| c.districts.iter().any(|x| x.dtype.as_string() == "DISTRICT_GOVERNMENT")),
        cities,
        player,
        builders,
        settlers: vec![],
        traders: vec![],
        units,
        research: None,
        civic: None,
        policy_dirty: false,
        era_mark: None,
        known_govs: BTreeSet::new(),
        past_govs: BTreeSet::new(),
        score: 0.0,
        last_output: [0.0; 6],
        city_output: vec![],
        log: vec![],
        counters: OMap::new(),
        builder_counter: 0,
        settler_counter: 0,
        bought: vec![],
        tiles_bought: 0,
        gifts_through: turn - 1,
        gp: OMap(p["greatPersonPoints"].as_object().map(|o| o.iter().map(|(k, v)| (crate::sym::sym(k), v["banked"].as_f64().unwrap_or(0.0))).collect()).unwrap_or_default()),
        gp_ours: p["greatPeopleRecruited"].as_array().map(|a| a.iter().filter_map(|g| osym(&g["individual"])).collect()).unwrap_or_default(),
        gp_pending: vec![],
        improving: vec![],
        req_at: vec![],
    }
}
