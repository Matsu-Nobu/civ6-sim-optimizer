//! The game data file (civ6opt build): one JSON file with everything the simulator and the searches read for one
//! recorded game. Its parts:
//!
//!   rules     every table of the rules database (Cache/DebugGameplay.sqlite): columns, rows in rowid order, and per
//!             column the rows matching each value in the order SQLite returns them for "select ... where col=?"
//!             (an index may order them differently from rowid); "__scans__": the row order of some whole-table reads
//!   map       the exported map
//!   rows      the recorded game's export rows of the player
//!   start     the start state, from the row of the start turn
//!   settings  the planner settings, the recorded cities' sites and tiles
//!   scenario  the parts of the recorded game the planner does not decide (exogenous boosts, tribal village gifts, ...)
//!   establish_turns  the turns each governor takes to establish
//!
//! The recorded game as plans ("recorded_genome", "recorded_genome_tiles") and its builder actions
//! ("settings"/"extra_builder") are added by the build command, which replays the record with the simulator.
use crate::json::Value;
use rusqlite::types::ValueRef;
use rusqlite::Connection;
use std::collections::{BTreeSet, HashMap};

/// Boost triggers by events the planner does not play; the record says when they fired.
const EXOGENOUS_TRIGGERS: [&str; 7] = ["BOOST_TRIGGER_MEET_CIV", "BOOST_TRIGGER_MEET_X_CITY_STATES", "BOOST_TRIGGER_FIND_NATURAL_WONDER",
    "BOOST_TRIGGER_KILL_WITH", "BOOST_TRIGGER_CLEAR_CAMP", "BOOST_TRIGGER_NUM_BARBS_KILLED", "BOOST_TRIGGER_RECEIVE_DOW"];

/// Whole-table reads whose row order the planner follows: name -> (query, table, key column).
/// The first selected column is the table's key, used to map the query's rows back to the table's rows.
const SCANS: [(&str, &str, &str, &str); 13] = [
    ("Technologies", "select TechnologyType from Technologies", "Technologies", "TechnologyType"),
    ("Civics", "select CivicType from Civics", "Civics", "CivicType"),
    ("GameModifiers", "select ModifierId from GameModifiers", "GameModifiers", "ModifierId"),
    ("Pantheons", "select BeliefType from Beliefs where BeliefClassType='BELIEF_CLASS_PANTHEON'", "Beliefs", "BeliefType"),
    ("Governments", "select GovernmentType, PrereqCivic from Governments", "Governments", "GovernmentType"),
    ("Policies", "select PolicyType, PrereqCivic, PrereqTech, GovernmentSlotType from Policies where PolicyType not like 'POLICY_GOV_%'",
     "Policies", "PolicyType"),
    ("Units", "select UnitType, PrereqTech, PrereqCivic, TraitType, Domain, CanTrain, PromotionClass, \
               ReligiousStrength > 0 or SpreadCharges > 0 or FoundReligion or EnabledByReligion from Units", "Units", "UnitType"),
    ("Buildings", "select BuildingType, PrereqTech, PrereqCivic, PrereqDistrict, TraitType, IsWonder from Buildings where InternalOnly=0",
     "Buildings", "BuildingType"),
    ("Districts", "select DistrictType, PrereqTech, PrereqCivic, TraitType, RequiresPopulation, OnePerCity, MaxPerPlayer \
                   from Districts where InternalOnly=0 and CityCenter=0", "Districts", "DistrictType"),
    ("Projects", "select ProjectType, PrereqTech, PrereqCivic, PrereqDistrict, MaxPlayerInstances, SpaceRace, WMD, UnlocksFromEffect \
                  from Projects", "Projects", "ProjectType"),
    ("Improvements", "select ImprovementType, PrereqTech, PrereqCivic, TraitType, SameAdjacentValid, RequiresRiver, MinimumAppeal, Coast, \
                      ValidAdjacentTerrainAmount, RequiresAdjacentBonusOrLuxury, RequiresAdjacentLuxury, NoAdjacentSpecialtyDistrict, \
                      AdjacentToLand, OnePerCity, Domain from Improvements where Buildable=1", "Improvements", "ImprovementType"),
    ("Governors", "select GovernorType, AssignCityState, TraitType from Governors", "Governors", "GovernorType"),
    ("PlantableFeatures", "select FeatureType, AddCivic from Features where AddCivic is not null", "Features", "FeatureType"),
];

/// Reads the database without changing it.
pub fn open(path: &str) -> Connection {
    Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap_or_else(|e| panic!("{path}: {e}"))
}

fn json_of(v: ValueRef) -> Value {
    match v {
        ValueRef::Null => Value::Null,
        ValueRef::Integer(i) => Value::Num(i as f64),
        ValueRef::Real(x) => Value::Num(x),
        ValueRef::Text(t) => Value::Str(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(_) => panic!("a blob in the rules database"),
    }
}

/// The text of a value as an object key, as Python's str() writes it (2.0 -> "2.0").
fn key_text(v: ValueRef) -> Option<String> {
    match v {
        ValueRef::Null => None,
        ValueRef::Integer(i) => Some(i.to_string()),
        ValueRef::Real(x) if x.fract() == 0.0 && x.abs() < 1e16 => Some(format!("{x:.1}")),
        ValueRef::Real(x) => Some(x.to_string()),
        ValueRef::Text(t) => Some(String::from_utf8_lossy(t).into_owned()),
        ValueRef::Blob(_) => panic!("a blob in the rules database"),
    }
}

/// Every table of the rules database, as the simulator's Rules::load reads it.
pub fn export_rules(db: &Connection) -> Value {
    let names: Vec<String> = db.prepare("select name from sqlite_master where type='table' and name not like 'sqlite_%'").unwrap()
        .query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
    let mut out: Vec<(String, Value)> = vec![];
    let mut keyed: HashMap<String, (Vec<String>, Vec<Vec<Value>>)> = HashMap::new();
    for t in &names {
        let cols: Vec<String> = db.prepare(&format!("pragma table_info(\"{t}\")")).unwrap()
            .query_map([], |r| r.get(1)).unwrap().map(|x| x.unwrap()).collect();
        // rowid and row from the same query: "select rowid" alone may come back in an index's order
        let mut ids = vec![];
        let mut rows: Vec<Vec<Value>> = vec![];
        let mut q = db.prepare(&format!("select rowid, * from \"{t}\" order by rowid")).unwrap();
        let mut rs = q.query([]).unwrap();
        while let Some(r) = rs.next().unwrap() {
            ids.push(r.get::<_, i64>(0).unwrap());
            rows.push((1..=cols.len()).map(|i| json_of(r.get_ref(i).unwrap())).collect());
        }
        let pos: HashMap<i64, usize> = ids.iter().enumerate().map(|(i, id)| (*id, i)).collect();
        let mut order = vec![];
        for c in &cols {
            let mut distinct: Vec<Option<String>> = vec![];
            let mut q = db.prepare(&format!("select distinct \"{c}\" from \"{t}\"")).unwrap();
            let mut rs = q.query([]).unwrap();
            while let Some(r) = rs.next().unwrap() {
                distinct.push(key_text(r.get_ref(0).unwrap()));
            }
            // a unique column: lookups find one row, there is no order to keep
            if distinct.len() == rows.len() && rows.len() > 64 {
                continue;
            }
            let mut by = vec![];
            let mut q = db.prepare(&format!("select rowid from \"{t}\" where \"{c}\"=?")).unwrap();
            let mut vq = db.prepare(&format!("select distinct \"{c}\" from \"{t}\" where \"{c}\" is not null")).unwrap();
            let mut vs = vq.query([]).unwrap();
            while let Some(v) = vs.next().unwrap() {
                let key = key_text(v.get_ref(0).unwrap()).unwrap();
                let val: rusqlite::types::Value = v.get(0).unwrap();
                let hits: Vec<Value> = q.query_map([val], |r| r.get::<_, i64>(0)).unwrap()
                    .map(|id| Value::from(pos[&id.unwrap()])).collect();
                by.push((key, Value::Arr(hits)));
            }
            order.push((c.clone(), Value::Obj(by)));
        }
        out.push((t.clone(), Value::obj(vec![
            ("cols", Value::Arr(cols.iter().map(|c| Value::from(c.as_str())).collect())),
            ("rows", Value::Arr(rows.iter().map(|r| Value::Arr(r.clone())).collect())),
            ("order", Value::Obj(order)),
        ])));
        keyed.insert(t.clone(), (cols, rows));
    }
    let mut scans = vec![];
    for (name, sql, table, key) in SCANS {
        let Some((cols, rows)) = keyed.get(table) else { continue };
        let kc = cols.iter().position(|c| c == key).unwrap();
        let mut pos: HashMap<String, Vec<usize>> = HashMap::new();
        for (i, r) in rows.iter().enumerate() {
            pos.entry(r[kc].to_string()).or_default().push(i);
        }
        let mut seen: HashMap<String, usize> = HashMap::new();
        let mut idx = vec![];
        let mut q = db.prepare(sql).unwrap();
        let mut rs = q.query([]).unwrap();
        while let Some(r) = rs.next().unwrap() {
            let v = json_of(r.get_ref(0).unwrap()).to_string();
            let k = seen.entry(v.clone()).or_insert(0);
            idx.push(Value::from(pos[&v][*k]));
            *k += 1;
        }
        scans.push((name.to_string(), Value::Arr(idx)));
    }
    out.push(("__scans__".to_string(), Value::Obj(scans)));
    Value::Obj(out)
}

fn huts(p: &Value) -> f64 {
    p["goodyHutsReceived"].as_object().map(|o| o.iter().filter_map(|(_, v)| v.as_f64()).sum()).unwrap_or(0.0)
}

fn units_trained(p: &Value, u: &str) -> f64 {
    p["unitsTrained"][u].as_f64().unwrap_or(0.0)
}

/// How many of `unit` the cities finished between saves a and b (no more progress than before, or switched away from
/// it leaving no progress on it: one switched away from with its progress kept was not finished, 10/21 Antium T28).
fn produced_between(a: &Value, b: &Value, unit: &str) -> f64 {
    let after: HashMap<&str, &Value> = b["cities"].as_array().unwrap().iter().map(|c| (c["name"].as_str().unwrap(), c)).collect();
    let mut n = 0.0;
    for c in a["cities"].as_array().unwrap() {
        let Some(c2) = after.get(c["name"].as_str().unwrap()) else { continue };
        let prog = |x: &Value| x["production"]["progress"][unit].as_f64().unwrap_or(0.0);
        let switched_away = c2["production"]["current"].as_str() != Some(unit) && prog(c2) == 0.0;
        // still on it with no more progress than before: one finished and the next begun (10/21 Rome T36: 0 and 24
        // carried over, then 0)
        let again = c2["production"]["current"].as_str() == Some(unit) && prog(c2) <= prog(c);
        if c["production"]["current"].as_str() == Some(unit) && (switched_away || again) {
            n += 1.0;
        }
    }
    n
}

fn boost_class(db: &Connection, item: &str) -> Option<String> {
    let col = if item.starts_with("TECH_") { "TechnologyType" } else { "CivicType" };
    db.query_row(&format!("select BoostClass from Boosts where {col}=?"), [item], |r| r.get(0)).ok()
}

fn param(db: &Connection, name: &str) -> f64 {
    let v: rusqlite::types::Value = db.query_row("select Value from GlobalParameters where Name=?", [name], |r| r.get(0))
        .unwrap_or_else(|e| panic!("GlobalParameters {name}: {e}"));
    match v {
        rusqlite::types::Value::Integer(i) => i as f64,
        rusqlite::types::Value::Real(x) => x,
        rusqlite::types::Value::Text(t) => t.parse().unwrap_or_else(|_| panic!("GlobalParameters {name}: {t}")),
        other => panic!("GlobalParameters {name}: {other:?}"),
    }
}

fn str_list(v: &Value) -> Vec<String> {
    v.as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default()
}

fn turn(d: &Value) -> i64 {
    d["turn"].as_i64().unwrap()
}

/// The parts of the recorded game the planner does not decide, by turn (scenario.rs Scenario::load reads them).
pub fn scenario(rows: &[Value], db: &Connection) -> Value {
    let pairs: Vec<(&Value, &Value)> = rows.iter().zip(rows.iter().skip(1)).collect();
    // boosts by events the planner does not play
    let mut exo: Vec<(String, Value)> = vec![];
    for (a, b) in &pairs {
        let pa = &a["player"];
        let old: BTreeSet<String> = str_list(&pa["techsBoosted"]).into_iter().chain(str_list(&pa["civicsInspired"])).collect();
        let pb = &b["player"];
        for t in str_list(&pb["techsBoosted"]).into_iter().chain(str_list(&pb["civicsInspired"])) {
            if !old.contains(&t) && boost_class(db, &t).map(|c| EXOGENOUS_TRIGGERS.contains(&c.as_str())).unwrap_or(false) {
                push_by_turn(&mut exo, turn(a), Value::from(t.as_str()));
            }
        }
    }
    // faith from outside the cities (tribal villages, ...): stock jumps beyond the cities' faith
    let mut faith = vec![];
    for (a, b) in &pairs {
        let fa = a["player"]["faithStock"].as_f64().unwrap_or(0.0);
        let fb = b["player"]["faithStock"].as_f64().unwrap_or(0.0);
        let city: f64 = a["cityYields"].as_array().map(|ys| ys.iter().map(|y| y["modified"]["YIELD_FAITH"].as_f64().unwrap()).sum()).unwrap_or(0.0);
        let spent = if !b["player"]["pantheon"].is_null() && a["player"]["pantheon"].is_null() { param(db, "RELIGION_PANTHEON_MIN_FAITH") * 0.5 } else { 0.0 };
        let gift = fb - fa - city + spent;
        if gift > 0.5 {
            faith.push((turn(a).to_string(), Value::from(gift)));
        }
    }
    // gold from outside (tribal villages, allies, city-states): a rise of the treasury beyond the turn's income (a
    // turn's income can differ from the rate shown at its start by a few gold, hence the margin)
    let mut gold = vec![];
    for (a, b) in &pairs {
        let (pa, pb) = (&a["player"], &b["player"]);
        let g = pb["gold"].as_f64().unwrap() - pa["gold"].as_f64().unwrap() - pa["goldPerTurn"].as_f64().unwrap_or(0.0);
        if g > 5.0 {
            gold.push((turn(a).to_string(), Value::from(g)));
        }
    }
    // recon units trained by each turn: exploration rewards go only to a plan that has scouted as much by then
    let scouts: Vec<(String, Value)> = rows.iter().map(|d| (turn(d).to_string(), Value::from(units_trained(&d["player"], "UNIT_SCOUT")))).collect();
    // units from outside (tribal villages, allies, city-states): new units no city produced, on a turn the hut count
    // went up or no gold or faith was spent (a unit bought with them is a purchase, replayed as one)
    let mut gifts = vec![];
    for (a, b) in &pairs {
        let (pa, pb) = (&a["player"], &b["player"]);
        let gold_spent = pa["gold"].as_f64().unwrap() + pa["goldPerTurn"].as_f64().unwrap_or(0.0) - pb["gold"].as_f64().unwrap() > 10.0;
        let faith_spent = pa["faithStock"].as_f64().unwrap_or(0.0) - pb["faithStock"].as_f64().unwrap_or(0.0) > 10.0;
        if huts(pb) <= huts(pa) && (gold_spent || faith_spent) {
            continue;
        }
        for (u, n) in b["player"]["unitsTrained"].as_object().unwrap() {
            if n.as_f64().unwrap() - units_trained(&a["player"], u) > produced_between(a, b, u) {
                push_by_turn(&mut gifts, turn(a), Value::from(u.as_str()));
            }
        }
    }
    Value::obj(vec![
        ("exo_boosts", Value::Obj(exo)),
        ("faith_gift", Value::Obj(faith)),
        ("gold_gift", Value::Obj(gold)),
        ("scouts_by_turn", Value::Obj(scouts)),
        ("unit_gifts", Value::Obj(gifts)),
        ("gp_timeline", gp_timeline(rows)),
    ])
}

fn push_by_turn(out: &mut Vec<(String, Value)>, t: i64, v: Value) {
    let k = t.to_string();
    match out.iter_mut().find(|(x, _)| *x == k) {
        Some((_, Value::Arr(a))) => a.push(v),
        _ => out.push((k, Value::Arr(vec![v]))),
    }
}

/// Per great person class: [individual, first turn on offer, turn another player took it (null: not taken)], in the
/// order the recorded game offered them (the offers are shared by all players).
fn gp_timeline(rows: &[Value]) -> Value {
    let mut seen: Vec<((String, String), (i64, i64))> = vec![];
    for d in rows {
        for (cls, ind) in d["player"]["greatPeopleOnOffer"].as_object().map(|o| o.as_slice()).unwrap_or(&[]) {
            let Some(ind) = ind.as_str().filter(|s| !s.is_empty()) else { continue };
            let k = (cls.clone(), ind.to_string());
            match seen.iter_mut().find(|(x, _)| *x == k) {
                Some((_, fl)) => fl.1 = turn(d),
                None => seen.push((k, (turn(d), turn(d)))),
            }
        }
    }
    let own: BTreeSet<String> = rows.iter().flat_map(|d| d["player"]["greatPeopleRecruited"].as_array().cloned().unwrap_or_default())
        .filter_map(|g| g["individual"].as_str().map(String::from)).collect();
    let last_row = turn(rows.last().unwrap());
    // a stable sort by the first turn on offer
    seen.sort_by_key(|(_, (f, _))| *f);
    let mut out: Vec<(String, Value)> = vec![];
    for ((cls, ind), (f, l)) in seen {
        // still on offer at the last recorded turn, or recruited by the recorded player: no other player took it
        let taken = if l >= last_row || own.contains(&ind) { Value::Null } else { Value::from(l + 1) };
        let item = Value::Arr(vec![Value::from(ind.as_str()), Value::from(f), taken]);
        match out.iter_mut().find(|(c, _)| *c == cls) {
            Some((_, Value::Arr(a))) => a.push(item),
            _ => out.push((cls, Value::Arr(vec![item]))),
        }
    }
    Value::Obj(out)
}

/// The start state, from the row of the start turn (model.rs state_from reads it).
pub fn start_state(d: &Value, db: &Connection) -> Value {
    let p = &d["player"];
    let t = turn(d);
    let sorted = |v: &Value| {
        let mut x = str_list(v);
        x.sort();
        Value::Arr(x.iter().map(|s| Value::from(s.as_str())).collect())
    };
    let cities_in = d["cities"].as_array().unwrap();
    let cities: Vec<Value> = cities_in.iter().map(|c| {
        let mut c = c.clone();
        if let Value::Obj(o) = &mut c {
            if let Some((_, Value::Obj(prod))) = o.iter_mut().find(|(k, _)| k == "production") {
                if let Some((_, cur)) = prod.iter_mut().find(|(k, _)| k == "current") {
                    *cur = Value::Null;
                }
            }
        }
        c
    }).collect();
    let name = |c: &Value| c["name"].as_str().unwrap().to_string();
    let acc_cities: Vec<(String, Value)> = cities_in.iter().map(|c| (name(c), Value::obj(vec![
        ("population", c["population"].clone()),
        ("food_store", Value::from(c["foodStore"].as_f64().unwrap_or(0.0))),
        ("progress", c["production"]["progress"].clone()),
        ("overflow", Value::from(0.0)),
        ("locked_cost", Value::Obj(vec![])),
    ]))).collect();
    let acc = Value::obj(vec![
        ("gold", p["gold"].clone()),
        ("tech_progress", p["techProgress"].clone()),
        ("civic_progress", p["civicProgress"].clone()),
        ("techs", sorted(&p["techs"])),
        ("civics", sorted(&p["civics"])),
        ("units_trained", p["unitsTrained"].clone()),
        ("faith", Value::from(p["faithStock"].as_f64().unwrap_or(0.0))),
        ("tech_overflow", Value::from(0.0)),
        ("civic_overflow", Value::from(0.0)),
        ("cities", Value::Obj(acc_cities)),
        ("districts_snapshot", Value::Null),
    ]);
    let mut player: Vec<(String, Value)> = ["civilization", "leader", "policies", "government", "pantheon", "governors", "envoys", "playerId"]
        .iter().map(|k| (k.to_string(), p[*k].clone())).collect();
    if p["government"].is_null() {
        player[3].1 = Value::from("GOVERNMENT_CHIEFDOM");
    }
    let units_in = d["units"].as_array().unwrap();
    let mut units: Vec<(String, Value)> = vec![];
    for u in units_in {
        let ty = u["type"].as_str().unwrap();
        if ["UNIT_BUILDER", "UNIT_SETTLER", "UNIT_TRADER"].contains(&ty) {
            continue;
        }
        match units.iter_mut().find(|(k, _)| k == ty) {
            Some((_, n)) => *n = Value::from(n.as_f64().unwrap() + 1.0),
            None => units.push((ty.to_string(), Value::from(1.0))),
        }
    }
    let builders: Vec<Value> = units_in.iter().filter(|u| u["type"].as_str() == Some("UNIT_BUILDER")).map(|u| Value::obj(vec![
        ("charges", Value::from(u["charges"].as_f64().filter(|c| *c != 0.0).unwrap_or(3.0))),
        ("at", Value::Arr(vec![u["x"].clone(), u["y"].clone()])),
        ("ready", Value::from(t)),
    ])).collect();
    let civics: BTreeSet<String> = str_list(&p["civics"]).into_iter().collect();
    let mut q = db.prepare("select GovernmentType, PrereqCivic from Governments").unwrap();
    let mut known: Vec<String> = q.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, Option<String>>(1)?))).unwrap()
        .map(|x| x.unwrap()).filter(|(g, pc)| pc.as_ref().map(|c| civics.contains(c)).unwrap_or(true) && g != "GOVERNMENT_ANARCHY")
        .map(|(g, _)| g).collect();
    known.sort();
    let gov_plaza = cities_in.iter().any(|c| c["districts"].as_array().unwrap().iter().any(|x| x["type"].as_str() == Some("DISTRICT_GOVERNMENT")));
    let empty = || Value::Arr(vec![]);
    let none = || Value::Obj(vec![]);
    Value::obj(vec![
        ("turn", Value::from(t)),
        ("acc", acc),
        ("cities", Value::Arr(cities)),
        ("player", Value::Obj(player)),
        ("builders", Value::Arr(builders)),
        ("settlers", empty()),
        ("traders", empty()),
        ("units", Value::Obj(units)),
        ("research", Value::Null),
        ("civic", Value::Null),
        ("policy_dirty", Value::from(true)),
        ("known_govs", Value::Arr(known.iter().map(|g| Value::from(g.as_str())).collect())),
        ("border", Value::Obj(cities_in.iter().map(|c| (name(c), Value::Arr(vec![Value::from(0.0),
            Value::from(c["owned"].as_array().unwrap().len() as f64 - 7.0)]))).collect())),
        ("focus", Value::Obj(cities_in.iter().map(|c| (name(c), Value::from("balanced"))).collect())),
        ("free_building_given", empty()),
        ("gov_plaza", Value::from(gov_plaza)),
        ("score", Value::from(0.0)),
        ("last_output", none()),
        ("log", empty()),
        ("counters", none()),
        ("bought", empty()),
        ("tiles_bought", Value::from(0.0)),
        ("gp", none()),
        ("gp_ours", empty()),
        ("gp_pending", empty()),
        ("trace", Value::Null),
    ])
}

fn xy(v: &Value) -> (i64, i64) {
    (v["x"].as_i64().unwrap(), v["y"].as_i64().unwrap())
}

fn xy_json((x, y): (i64, i64)) -> Value {
    Value::Arr(vec![Value::from(x), Value::from(y)])
}

/// The recorded cities but the capital, in founding order: (name, centre).
pub fn recorded_sites(rows: &[Value]) -> Vec<(String, (i64, i64))> {
    let mut seen = BTreeSet::new();
    let mut out = vec![];
    for d in rows {
        for c in d["cities"].as_array().unwrap() {
            let name = c["name"].as_str().unwrap().to_string();
            if seen.insert(name.clone()) && c["capital"].as_bool() != Some(true) {
                out.push((name, xy(&c["center"])));
            }
        }
    }
    out
}

/// The tiles each recorded city got after its founding, in order: city centre -> [(turn, tile)].
fn recorded_tiles(rows: &[Value]) -> Vec<((i64, i64), Vec<(i64, (i64, i64))>)> {
    let mut out: Vec<((i64, i64), Vec<(i64, (i64, i64))>)> = vec![];
    for (a, b) in rows.iter().zip(rows.iter().skip(1)) {
        let before: HashMap<&str, BTreeSet<(i64, i64)>> = a["cities"].as_array().unwrap().iter()
            .map(|c| (c["name"].as_str().unwrap(), c["owned"].as_array().unwrap().iter().map(xy).collect())).collect();
        for c in b["cities"].as_array().unwrap() {
            let Some(old) = before.get(c["name"].as_str().unwrap()) else { continue };
            let new: BTreeSet<(i64, i64)> = c["owned"].as_array().unwrap().iter().map(xy).filter(|p| !old.contains(p)).collect();
            let centre = xy(&c["center"]);
            let i = match out.iter().position(|(k, _)| *k == centre) {
                Some(i) => i,
                None => {
                    out.push((centre, vec![]));
                    out.len() - 1
                }
            };
            out[i].1.extend(new.into_iter().map(|p| (turn(a), p)));
        }
    }
    out
}

/// The planner settings; "extra_builder" is filled in by the build command.
pub fn settings(rows: &[Value], horizon: i64, builder_candidates: i64) -> Value {
    let sites = recorded_sites(rows);
    Value::obj(vec![
        ("horizon", Value::from(horizon)),
        ("max_cities", Value::from(20i64)),
        ("settle_candidates", Value::from(5i64)),
        ("settle_radius", Value::from(8i64)),
        ("builder_candidates", Value::from(builder_candidates)),
        // the capital's first build is a scout: exploring is outside the model, and the opening scout is always built
        ("fixed", Value::Arr(vec![Value::Arr(vec![Value::Arr(vec![Value::from("prod"), Value::from("CAPITAL"), Value::from(0i64)]),
            Value::from("UNIT_SCOUT")])])),
        ("extra_sites", Value::Arr(sites.iter().map(|(_, p)| xy_json(*p)).collect())),
        ("site_names", Value::Arr(sites.iter().map(|(n, p)| Value::Arr(vec![xy_json(*p), Value::from(n.as_str())])).collect())),
        ("extra_builder", Value::Arr(vec![])),
        ("speed", Value::from("GAMESPEED_ONLINE")),
        ("recorded_tiles", Value::Arr(recorded_tiles(rows).into_iter().map(|(c, ts)| Value::Arr(vec![xy_json(c),
            Value::Arr(ts.into_iter().map(|(t, p)| Value::Arr(vec![Value::from(t), xy_json(p)])).collect())])).collect())),
    ])
}

/// The number in the first "Establishes in N turns" of a text.
fn establish_in(text: &str) -> Option<i64> {
    let mut best: Option<(usize, i64)> = None;
    for marker in ["Establishes in ", "establishes in "] {
        let mut from = 0;
        while let Some(i) = text[from..].find(marker) {
            let at = from + i;
            let rest = &text[at + marker.len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if !digits.is_empty() && rest[digits.len()..].starts_with(" turns") {
                if best.map(|(b, _)| at < b).unwrap_or(true) {
                    best = Some((at, digits.parse().unwrap()));
                }
                break;
            }
            from = at + marker.len();
        }
    }
    best.map(|(_, n)| n)
}

/// Turns each governor takes to establish, from its base promotion's English text (5 when the text does not say).
/// `text_db`: the English game text, Tag -> Text (civ6opt text, build_text_db).
pub fn establish_turns(db: &Connection, text_db: &Connection) -> Value {
    let govs: Vec<String> = db.prepare("select GovernorType from Governors").unwrap().query_map([], |r| r.get(0)).unwrap().map(|x| x.unwrap()).collect();
    let mut out = vec![];
    for g in govs {
        let descs: Vec<String> = db.prepare("select p.Description from GovernorPromotionSets s join GovernorPromotions p \
                on p.GovernorPromotionType=s.GovernorPromotion where s.GovernorType=? and p.Level=0").unwrap()
            .query_map([&g], |r| r.get::<_, Option<String>>(0)).unwrap().filter_map(|x| x.unwrap()).collect();
        let mut n = 5;
        for d in descs {
            let text: Option<String> = text_db.query_row("select Text from Text where Tag=?", [&d], |r| r.get(0)).ok();
            if let Some(k) = text.as_deref().and_then(establish_in) {
                n = k;
            }
        }
        out.push((g, Value::from(n)));
    }
    Value::Obj(out)
}

/// Rows of the export file (one JSON object per line).
pub fn read_rows(path: &str) -> Vec<Value> {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}")).lines().filter(|l| !l.trim().is_empty())
        .map(|l| crate::json::parse(l).unwrap()).collect()
}

/// The game data file without the parts that need a replay of the record.
pub fn assemble(prefix: &str, rules_path: &str, text_path: &str, player: i64, start: i64, horizon: i64, builder_candidates: i64) -> Value {
    let db = open(rules_path);
    let text = open(text_path);
    let rows = read_rows(&format!("{prefix}_p{player}.jsonl"));
    let map_path = format!("{prefix}_map_T{start}.json");
    let map = crate::json::parse(&std::fs::read_to_string(&map_path).unwrap_or_else(|e| panic!("{map_path}: {e}"))).unwrap();
    let start_row = rows.iter().find(|d| turn(d) == start).unwrap_or_else(|| panic!("no record row for turn {start}"));
    let st = start_state(start_row, &db);
    Value::obj(vec![
        ("rules", export_rules(&db)),
        ("map", map),
        ("start", st),
        ("settings", settings(&rows, horizon, builder_candidates)),
        ("recorded_genome", Value::Arr(vec![])),
        ("recorded_genome_tiles", Value::Arr(vec![])),
        ("establish_turns", establish_turns(&db, &text)),
        ("scenario", scenario(&rows, &db)),
        ("rows", Value::Arr(rows)),
    ])
}

/// An XML element, as Python's ElementTree keeps it: its attributes, its text up to its first child, its children.
struct Element {
    name: Vec<u8>,
    attrs: HashMap<String, String>,
    text: Option<String>,
    children: Vec<Element>,
}

/// The XML file as a tree; None when it is not well-formed (Python's ElementTree then raises ParseError).
fn parse_xml(bytes: &[u8]) -> Option<Element> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_reader(bytes);
    let mut buf = vec![];
    let mut stack: Vec<Element> = vec![Element { name: vec![], attrs: HashMap::new(), text: None, children: vec![] }];
    let start = |e: &quick_xml::events::BytesStart| -> Option<Element> {
        let mut attrs = HashMap::new();
        for a in e.attributes() {
            let a = a.ok()?;
            // XML reads a line end or tab in an attribute value as a space
            let v = a.normalized_value(quick_xml::XmlVersion::Implicit1_0).ok()?.into_owned();
            attrs.insert(a.key.as_ref().to_string(), v);
        }
        Some(Element { name: e.name().as_ref().as_bytes().to_vec(), attrs, text: None, children: vec![] })
    };
    // text goes to the open element only before its first child; line ends are read as \n, as XML requires
    let add_text = |stack: &mut Vec<Element>, s: &str| {
        let top = stack.last_mut().unwrap();
        if top.children.is_empty() {
            top.text.get_or_insert_with(String::new).push_str(&s.replace("\r\n", "\n").replace('\r', "\n"));
        }
    };
    loop {
        match reader.read_event_into(&mut buf).ok()? {
            Event::Start(e) => stack.push(start(&e)?),
            Event::Empty(e) => {
                let el = start(&e)?;
                stack.last_mut().unwrap().children.push(el);
            }
            Event::End(_) => {
                let el = stack.pop()?;
                stack.last_mut()?.children.push(el);
            }
            // the reader splits text at entity references (Event::GeneralRef), so text has none
            Event::Text(t) => add_text(&mut stack, &t.into_inner()),
            Event::CData(t) => add_text(&mut stack, &t.into_inner()),
            Event::GeneralRef(r) => {
                let s = match r.resolve_char_ref().ok()? {
                    Some(c) => c.to_string(),
                    None => match r.into_inner().as_ref() {
                        "lt" => "<".to_string(),
                        "gt" => ">".to_string(),
                        "amp" => "&".to_string(),
                        "apos" => "'".to_string(),
                        "quot" => "\"".to_string(),
                        _ => return None,
                    },
                };
                add_text(&mut stack, &s);
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }
    let mut doc = stack.pop()?;
    if !stack.is_empty() || doc.children.len() != 1 {
        return None;
    }
    doc.children.pop()
}

/// The (Tag, Text) of each Row / Replace element of one language (en_US when the element does not say) in an XML
/// file of the game's text: the Text attribute, else the text of a <Text> child. A file that is not well-formed
/// gives none.
fn text_rows(path: &std::path::Path, lang: &str) -> Vec<(String, String)> {
    fn walk(el: &Element, lang: &str, out: &mut Vec<(String, String)>) {
        if el.name == b"Row" || el.name == b"Replace" {
            let text = el.attrs.get("Text").cloned().or_else(|| el.children.iter().find(|c| c.name == b"Text").and_then(|c| c.text.clone()));
            let l = el.attrs.get("Language").map(|s| s.as_str()).unwrap_or("en_US");
            if let (Some(tag), Some(text)) = (el.attrs.get("Tag"), text) {
                if !tag.is_empty() && !text.is_empty() && l == lang {
                    out.push((tag.clone(), text));
                }
            }
        }
        for c in &el.children {
            walk(c, lang, out);
        }
    }
    let mut out = vec![];
    if let Some(root) = std::fs::read(path).ok().and_then(|b| parse_xml(&b)) {
        walk(&root, lang, &mut out);
    }
    out
}

/// Every file under `dir`, recursively.
fn files_under(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        if p.is_dir() {
            files_under(&p, out);
        } else {
            out.push(p);
        }
    }
}

/// Writes the English game text (base game, DLCs, the workshop mods' lang/english.xml) to `out` as a table
/// Text(Tag, Text, Source); later files override earlier ones, the mods last. The establish turns of the governors
/// are read from it.
pub fn build_text_db(out: &str, game: &str, workshop: &str) {
    let mut files = vec![];
    files_under(std::path::Path::new(game), &mut files);
    files.retain(|p| {
        let s = p.to_string_lossy();
        let name = p.file_name().unwrap().to_string_lossy();
        s.ends_with(".xml") && s.contains("/Text/") && !name.contains("Translations")
            && (s.contains("/en_US/") || s.ends_with("Text.xml") || s.contains("_Text"))
    });
    // sorted by path components, as Python sorts paths
    files.sort_by(|a, b| a.components().cmp(b.components()));
    let mut mods: Vec<std::path::PathBuf> = std::fs::read_dir(workshop).map(|rd| rd.flatten().map(|e| e.path().join("lang/english.xml"))
        .filter(|p| p.is_file()).collect()).unwrap_or_default();
    mods.sort_by(|a, b| a.components().cmp(b.components()));
    files.extend(mods);
    let _ = std::fs::remove_file(out);
    let mut db = Connection::open(out).unwrap_or_else(|e| panic!("{out}: {e}"));
    db.execute("create table Text (Tag text primary key, Text text, Source text)", []).unwrap();
    let tx = db.transaction().unwrap();
    let mut n = 0;
    {
        let mut ins = tx.prepare("insert or replace into Text values (?, ?, ?)").unwrap();
        for f in &files {
            let src = f.to_string_lossy();
            for (tag, text) in text_rows(f, "en_US") {
                ins.execute(rusqlite::params![tag, text, src]).unwrap();
                n += 1;
            }
        }
    }
    tx.commit().unwrap();
    eprintln!("{} files, {n} rows", files.len());
}
