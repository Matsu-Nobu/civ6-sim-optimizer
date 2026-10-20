//! Exogenous parts of the recorded game, by turn (plan2.Scenario), and the planner settings.
use crate::model::*;
use crate::sym::{sym, Sym};
use crate::json::Value;
use std::collections::{BTreeSet, HashMap};

pub struct TurnRow {
    pub game_era: Option<Sym>,
    pub envoys: Vec<Envoy>,
    pub world_cities: Vec<Endpoint>,
    pub wonders_taken: BTreeSet<Sym>,
    pub prophet_on_offer: bool,
    /// the player's age of the era ("GOLDEN", ...) as recorded
    pub age: Option<Sym>,
}

pub struct Scenario {
    pub rows: HashMap<i64, TurnRow>,
    pub last_turn: i64,
    pub exo_boosts: HashMap<i64, Vec<Sym>>,
    pub faith_gift: HashMap<i64, f64>,
    pub gold_gift: HashMap<i64, f64>,
    /// gold the recorded player gave away (to allies) on each turn: taken from a plan's gold too (--gold-out)
    pub gold_out: HashMap<i64, f64>,
    /// luxury types from outside the player's land on each turn (inferred from the record's happiness; --team)
    pub extra_luxuries: HashMap<i64, i64>,
    pub scouts_by_turn: HashMap<i64, i64>,
    pub unit_gifts: HashMap<i64, Vec<Sym>>,
    /// players whose cities count as allied for trade routes (the research alliance's route science; --allies, or
    /// the major players the record's routes went to)
    pub allies: Vec<i64>,
    /// class -> [(individual, first turn, turn taken by others)]
    pub gp_timeline: Vec<(Sym, Vec<(Sym, i64, Option<i64>)>)>,
}

const DEFAULT_ERA_TURNS: [(i64, &str); 5] = [(1, "ERA_ANCIENT"), (31, "ERA_CLASSICAL"), (61, "ERA_MEDIEVAL"), (89, "ERA_RENAISSANCE"), (119, "ERA_INDUSTRIAL")];

fn turn_map<T>(v: &Value, f: impl Fn(&Value) -> T) -> HashMap<i64, T> {
    v.as_object().map(|o| o.iter().map(|(k, x)| (k.parse::<i64>().unwrap(), f(x))).collect()).unwrap_or_default()
}

impl Scenario {
    /// Scouts the record had built (not received as gifts) by the end of turn t: the exploration a plan must match
    /// before the recorded rewards of turn t come.
    pub fn scouts_built_by(&self, t: i64) -> i64 {
        let gifted = self.unit_gifts.iter().filter(|(tt, _)| **tt <= t).flat_map(|(_, us)| us.iter()).filter(|u| **u == crate::sym::sym("UNIT_SCOUT")).count() as i64;
        self.scouts_by_turn.get(&t).copied().unwrap_or(0) - gifted
    }
    /// Scouts received as the record's gifts of the turns up to t.
    pub fn scouts_gifted_through(&self, t: i64) -> i64 {
        self.unit_gifts.iter().filter(|(tt, _)| **tt <= t).flat_map(|(_, us)| us.iter()).filter(|u| **u == crate::sym::sym("UNIT_SCOUT")).count() as i64
    }
    pub fn load(rows: &Value, sc: &Value) -> Scenario {
        let mut out = HashMap::new();
        let mut last = 0;
        for d in rows.as_array().unwrap() {
            let t = d["turn"].as_i64().unwrap();
            last = t;
            let me = d["player"]["playerId"].as_i64();
            out.insert(t, TurnRow {
                game_era: osym(&d["gameEra"]),
                envoys: d["player"]["envoys"].as_array().map(|a| a.iter().map(envoy_from).collect()).unwrap_or_default(),
                world_cities: d["worldCities"].as_array().map(|a| a.iter().map(endpoint_from).collect()).unwrap_or_default(),
                wonders_taken: d["worldWonders"].as_array().map(|a| a.iter().filter(|w| w["playerId"].as_i64() != me).filter_map(|w| osym(&w["building"])).collect()).unwrap_or_default(),
                prophet_on_offer: d["player"]["prophetOnOffer"].as_bool().unwrap_or(true),
                age: osym(&d["player"]["age"]),
            });
        }
        // the recorded player's own recruits: gone at the next turn for a plan that did not take them (someone
        // else would have), so the ones after them come on offer as in the record
        let mut own: Vec<(Sym, i64)> = vec![];
        for d in rows.as_array().unwrap() {
            for g in d["player"]["greatPeopleRecruited"].as_array().cloned().unwrap_or_default() {
                if let (Some(i), Some(t)) = (osym(&g["individual"]), g["turn"].as_i64()) {
                    if !own.iter().any(|x| x.0 == i) {
                        own.push((i, t));
                    }
                }
            }
        }
        let mut sc_out = Scenario {
            rows: out,
            last_turn: last,
            exo_boosts: turn_map(&sc["exo_boosts"], |x| x.as_array().unwrap().iter().filter_map(osym).collect()),
            faith_gift: turn_map(&sc["faith_gift"], |x| x.as_f64().unwrap_or(0.0)),
            gold_gift: turn_map(&sc["gold_gift"], |x| x.as_f64().unwrap_or(0.0)),
            gold_out: HashMap::new(),
            extra_luxuries: HashMap::new(),
            allies: vec![],
            scouts_by_turn: turn_map(&sc["scouts_by_turn"], |x| x.as_i64().unwrap_or(0)),
            unit_gifts: turn_map(&sc["unit_gifts"], |x| x.as_array().unwrap().iter().filter_map(osym).collect()),
            gp_timeline: sc["gp_timeline"].as_object().map(|o| o.iter().map(|(cls, v)| (sym(cls), v.as_array().unwrap().iter().map(|e| {
                let e = e.as_array().unwrap();
                (osym(&e[0]).unwrap(), e[1].as_i64().unwrap_or(0), e[2].as_i64())
            }).collect())).collect()).unwrap_or_default(),
        };
        for (_, list) in sc_out.gp_timeline.iter_mut() {
            for e in list.iter_mut() {
                if e.2.is_none() {
                    if let Some((_, t)) = own.iter().find(|x| x.0 == e.0) {
                        e.2 = Some(t + 1);
                    }
                }
            }
        }
        sc_out
    }
    fn row(&self, turn: i64) -> &TurnRow {
        self.rows.get(&turn).unwrap_or_else(|| &self.rows[&self.last_turn])
    }
    pub fn game_era(&self, turn: i64) -> Sym {
        if let Some(d) = self.rows.get(&turn) {
            if let Some(e) = d.game_era {
                return e;
            }
        }
        let mut era = "ERA_ANCIENT";
        for (t, e) in DEFAULT_ERA_TURNS {
            if turn >= t {
                era = e;
            }
        }
        sym(era)
    }
    /// The player's age: as recorded; past the record, a golden age after the ancient era (all the user's games so
    /// far; era score is not modelled yet).
    pub fn age(&self, turn: i64) -> Sym {
        if let Some(a) = self.rows.get(&turn).and_then(|d| d.age) {
            return a;
        }
        if self.game_era(turn) == s!("ERA_ANCIENT") { s!("NORMAL") } else { s!("GOLDEN") }
    }
    pub fn envoys(&self, turn: i64) -> &Vec<Envoy> {
        &self.row(turn).envoys
    }
    /// Other players' city centers on the record by `turn` (once seen, kept: a site there was not the player's to
    /// settle, even after the record's player took that city)
    pub fn foreign_centers(&self, turn: i64, me: Option<i64>) -> Vec<XY> {
        let mut out: Vec<XY> = vec![];
        for (t, row) in &self.rows {
            if *t > turn {
                continue;
            }
            for c in &row.world_cities {
                if c.player_id != me && !out.contains(&(c.x, c.y)) {
                    out.push((c.x, c.y));
                }
            }
        }
        out
    }
    pub fn world_cities(&self, turn: i64) -> &Vec<Endpoint> {
        &self.row(turn).world_cities
    }
    pub fn wonders_taken(&self, turn: i64) -> &BTreeSet<Sym> {
        &self.row(turn).wonders_taken
    }
    pub fn prophet_on_offer(&self, turn: i64) -> bool {
        self.row(turn).prophet_on_offer
    }
    pub fn gp_list(&self, cls: Sym) -> &[(Sym, i64, Option<i64>)] {
        self.gp_timeline.iter().find(|(c, _)| *c == cls).map(|(_, v)| v.as_slice()).unwrap_or(&[])
    }
    pub fn gp_offer(&self, cls: Sym, turn: i64, ours: &[Sym]) -> Option<Sym> {
        for (ind, _, taken) in self.gp_list(cls) {
            if ours.contains(ind) || taken.map(|t| t <= turn).unwrap_or(false) {
                continue;
            }
            return Some(*ind);
        }
        None
    }
}

/// A fixed / genome key on the Python side: [kind, entity, n] with entity a string or int.
pub fn key_from(v: &Value) -> Key {
    let a = v.as_array().unwrap();
    Key {
        kind: sym(a[0].as_str().unwrap()),
        ent: match &a[1] {
            Value::Str(s) => Ent::S(sym(s)),
            Value::Num(n) => Ent::I(*n as i64),
            _ => Ent::S(Sym(0)),
        },
        n: a[2].as_i64().unwrap(),
    }
}

/// A choice as JSON (jsonable on the Python side): a string, [x, y], or a list (tuple choices).
pub fn choice_from(v: &Value) -> Option<Choice> {
    match v {
        Value::Str(s) => Some(Choice::S(sym(s))),
        Value::Arr(a) => {
            if a.len() == 2 && a[0].is_number() && a[1].is_number() {
                return Some(Choice::Xy(a[0].as_i64()? as i32, a[1].as_i64()? as i32));
            }
            let tag = a.first()?.as_str()?;
            let s = |i: usize| a.get(i).and_then(|x| x.as_str()).map(sym);
            let xy = |i: usize| a.get(i).and_then(|x| x.as_array()).map(|p| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32));
            match tag {
                "improve" | "plant" | "harvest" | "rush" => {
                    let (x, y) = xy(2)?;
                    Some(Choice::Act(sym(tag), s(1)?, x, y, s(3)?))
                }
                "wait" => Some(Choice::Wait),
                "none" => Some(Choice::Nothing),
                "item" => Some(Choice::BuyItem(s(1)?, s(2)?, a[3].as_f64()?.to_bits(), s(4).unwrap_or(sym("YIELD_GOLD")))),
                "tile" => {
                    let (x, y) = xy(3)?;
                    Some(Choice::BuyTile(s(1)?, a[2].as_f64()?.to_bits(), x, y))
                }
                "appoint" => Some(Choice::Appoint(s(1)?, s(2)?)),
                "promote" => Some(Choice::Promote(s(1)?, s(2)?)),
                "route" => Some(Choice::Route(s(1)?, s(2)?)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// A choice back to the Python JSON form.
pub fn choice_json(c: &Choice) -> Value {
    let st = |s: &Sym| Value::from(s.as_string());
    let xy = |x: &i32, y: &i32| Value::Arr(vec![Value::from(*x), Value::from(*y)]);
    match c {
        Choice::S(s) => st(s),
        Choice::Xy(x, y) => xy(x, y),
        Choice::Act(k, c, x, y, w) => Value::Arr(vec![st(k), st(c), xy(x, y), st(w)]),
        Choice::Wait => Value::Arr(vec![Value::from("wait")]),
        Choice::Nothing => Value::Arr(vec![Value::from("none")]),
        Choice::BuyItem(c, i, p, cur) => {
            let mut v = vec![Value::from("item"), st(c), st(i), Value::from(f64::from_bits(*p))];
            if cur.as_string() != "YIELD_GOLD" {
                v.push(st(cur));
            }
            Value::Arr(v)
        }
        Choice::BuyTile(c, g, x, y) => Value::Arr(vec![Value::from("tile"), st(c), Value::from(f64::from_bits(*g)), xy(x, y)]),
        Choice::Appoint(g, c) => Value::Arr(vec![Value::from("appoint"), st(g), st(c)]),
        Choice::Promote(g, p) => Value::Arr(vec![Value::from("promote"), st(g), st(p)]),
        Choice::Route(o, d) => Value::Arr(vec![Value::from("route"), st(o), st(d)]),
    }
}

pub fn key_json(k: &Key) -> Value {
    let ent = match &k.ent {
        Ent::S(s) => Value::from(s.as_string()),
        Ent::I(i) => Value::from(*i),
    };
    Value::Arr(vec![Value::from(k.kind.as_string()), ent, Value::from(k.n)])
}

pub struct Settings {
    /// offer units that do nothing for the objective (military) to a plan too (--allow-military)
    pub allow_idle_units: bool,
    /// no harvests (chops, resource harvests) offered before this turn (--no-harvest-before)
    pub no_harvest_before: i64,
    pub horizon: i64,
    pub max_cities: usize,
    pub settle_candidates: usize,
    pub settle_radius: i32,
    pub builder_candidates: Option<usize>,
    /// (kind, entity or "CAPITAL", n) -> choice
    pub fixed: Vec<(Key, Choice)>,
    pub extra_sites: Vec<XY>,
    pub site_names: Vec<(XY, Sym)>,
    pub extra_builder: Vec<Choice>,
    pub borders: Vec<((XY, i64), XY)>,
    /// Replaying the recorded game (plan2.Settings.recorded_tiles): city centre -> [(turn, plot)] the recorded city
    /// got, each given on its turn; None: borders and tile purchases as modelled.
    pub recorded_tiles: Option<HashMap<XY, Vec<(i64, XY)>>>,
}

/// settings.recorded_tiles of a problem file
pub fn recorded_tiles_from(v: &Value) -> HashMap<XY, Vec<(i64, XY)>> {
    let xy = |p: &Value| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32);
    v.as_array().map(|a| a.iter().map(|e| (xy(&e[0]), e[1].as_array().unwrap().iter().map(|x| (x[0].as_i64().unwrap(), xy(&x[1]))).collect())).collect())
        .unwrap_or_default()
}

impl Settings {
    pub fn load(v: &Value) -> Settings {
        Settings {
            allow_idle_units: false,
            no_harvest_before: 0,
            horizon: v["horizon"].as_i64().unwrap_or(60),
            max_cities: v["max_cities"].as_u64().unwrap_or(8) as usize,
            settle_candidates: v["settle_candidates"].as_u64().unwrap_or(5) as usize,
            settle_radius: v["settle_radius"].as_i64().unwrap_or(8) as i32,
            builder_candidates: v["builder_candidates"].as_u64().map(|x| x as usize),
            fixed: v["fixed"].as_array().map(|a| a.iter().filter_map(|e| Some((key_from(&e[0]), choice_from(&e[1])?))).collect()).unwrap_or_default(),
            extra_sites: v["extra_sites"].as_array().map(|a| a.iter().map(|p| (p[0].as_i64().unwrap() as i32, p[1].as_i64().unwrap() as i32)).collect()).unwrap_or_default(),
            site_names: v["site_names"].as_array().map(|a| a.iter().map(|e| ((e[0][0].as_i64().unwrap() as i32, e[0][1].as_i64().unwrap() as i32), sym(e[1].as_str().unwrap()))).collect()).unwrap_or_default(),
            extra_builder: v["extra_builder"].as_array().map(|a| a.iter().filter_map(choice_from).collect()).unwrap_or_default(),
            borders: vec![],
            recorded_tiles: None,
        }
    }
}
