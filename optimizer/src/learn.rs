//! What a decision's options are worth: the features of a decision and of each of its options, and a model of
//! gradient boosted trees (trained by tools/learn_train.py) that predicts how much choosing an option changes the
//! final score from the plan's own choice.
//!
//! A row describes one option of one decision: the kind of decision, a name for the option (categorical: an item, an
//! action, ...), and numbers of the state at the decision (the player's yields and stock), of the city the decision
//! belongs to (when it belongs to one), and of the option (its cost, whether the base policy picks it).
use crate::json::{self, Value};
use crate::model::*;
use crate::planner::Planner;
use crate::sym::Sym;

/// The value of a number the decision does not have (a city's when it belongs to none): below every real one, so
/// the trees need no rule for missing values.
pub const MISSING: f64 = -999.0;

pub const NUMERIC: [&str; 32 + N_OPTION] = [
    "turn", "n", "n_options", "food", "production", "gold", "science", "culture", "faith", "pop", "cities", "districts",
    "buildings", "improvements", "invested", "food_store", "charges", "settlers", "traders", "builders", "techs", "civics",
    "gold_stock", "military", "city_pop", "city_food_store", "city_housing_left", "city_districts", "city_buildings",
    "city_food", "city_production", "city_progress", "option_cost", "option_is_base", "option_turns", "target_pop",
    "target_housing_left", "target_food", "target_production", "tile_food", "tile_production", "tile_worked",
    "tile_resource", "tile_improved",
];

/// The number of an option's own features (option_features).
pub const N_OPTION: usize = 12;

/// A name for the option the trees split on: the item, the builder's action and its improvement, ...
pub fn option_name(c: &Choice) -> String {
    match c {
        Choice::S(x) => x.as_string(),
        Choice::Xy(..) => "XY".into(),
        Choice::Act(a, _, _, _, what) => format!("ACT_{}_{}", a.as_string(), what.as_string()),
        Choice::Wait => "WAIT".into(),
        Choice::Nothing => "NONE".into(),
        Choice::BuyItem(_, item, _, _) => format!("BUY_{}", item.as_string()),
        Choice::BuyTile(..) => "BUY_TILE".into(),
        Choice::Appoint(g, _) => format!("APPOINT_{}", g.as_string()),
        Choice::Promote(_, pr) => format!("PROMOTE_{}", pr.as_string()),
        Choice::Route(..) => "ROUTE".into(),
    }
}

fn option_cost(p: &Planner, c: &Choice) -> f64 {
    match c {
        Choice::S(x) | Choice::BuyItem(_, x, _, _) => p.r.cost(*x),
        _ => 0.0,
    }
}

/// The city a decision belongs to (production, citizen focus, ...), if it belongs to one.
fn decision_city<'s>(st: &'s State, key: &Key) -> Option<&'s City> {
    match &key.ent {
        Ent::S(n) => st.cities.iter().find(|c| c.name == *n),
        Ent::I(_) => None,
    }
}

/// A city's population, housing left, food and production per turn.
fn city_numbers(p: &Planner, st: &State, eng: &crate::engine::Engine, c: &City) -> [f64; 4] {
    let pop = st.cacc(c.name).population as f64;
    let out = st.city_output.iter().find(|x| x.0 == c.name).map(|x| x.1).unwrap_or([MISSING; 6]);
    [pop, crate::growth::housing(p.r, c, Some(eng)) - pop, out[0], out[1]]
}

/// The numbers of the state at a decision that are the same for all its options; `eng` is the engine of the state
/// (Planner::engine).
pub fn decision_features(p: &Planner, st: &State, key: &Key, options: &[Choice], eng: &crate::engine::Engine) -> Vec<f64> {
    let v = crate::value::features(p, st);
    let mut f = vec![st.turn as f64, key.n as f64, options.len() as f64];
    // the value's features without its constant, in NUMERIC's order
    f.extend_from_slice(&v[1..15]);
    f.extend_from_slice(&[v[15], v[16], st.builders.len() as f64, v[17], v[18], v[19], v[20]]);
    match decision_city(st, key) {
        Some(c) => {
            let acc = st.cacc(c.name);
            let [pop, housing_left, food, production] = city_numbers(p, st, eng, c);
            let progress = c.current.and_then(|i| acc.progress.get(&i).copied()).unwrap_or(0.0);
            f.extend_from_slice(&[pop, acc.food_store, housing_left, c.districts.iter().filter(|d| d.complete).count() as f64,
                c.buildings.len() as f64, food, production, progress]);
        }
        None => f.extend_from_slice(&[MISSING; 8]),
    }
    f
}

/// The numbers of one option (appended to the decision's): its cost and whether the base policy (`base`,
/// Planner::base) picks it; the turns it takes the city to make it; the city it acts on (a builder's, a purchase's,
/// a governor's, a route's destination; else the decision's city); the tile it acts on (a builder's, a bought one, a
/// new city's site): its yields before the action, whether a citizen works it, has a resource, an improvement.
pub fn option_features(p: &Planner, st: &State, key: &Key, base: Option<&Choice>, c: &Choice, eng: &crate::engine::Engine) -> [f64; N_OPTION] {
    let cost = option_cost(p, c);
    let target = match c {
        Choice::Act(_, city, ..) | Choice::BuyItem(city, ..) | Choice::BuyTile(city, ..) | Choice::Appoint(_, city) | Choice::Route(_, city) =>
            st.cities.iter().find(|x| x.name == *city),
        _ => decision_city(st, key),
    };
    let city = target.map(|t| city_numbers(p, st, eng, t)).unwrap_or([MISSING; 4]);
    let turns = if cost > 0.0 && city[3] > 0.0 { cost / city[3] } else { MISSING };
    let xy = match c {
        Choice::Act(_, _, x, y, _) | Choice::BuyTile(_, _, x, y) | Choice::Xy(x, y) => Some((*x, *y)),
        _ => None,
    };
    let plot = xy.and_then(|xy| st.cities.iter().flat_map(|c| c.owned.iter().chain(c.near.iter())).find(|q| q.xy() == xy)
        .or_else(|| p.w.map.plots.get(&xy)));
    let tile = match plot {
        Some(q) => {
            let y = crate::yields::tile_yields(p.r, q, &st.acc.techs, &st.acc.civics, false, &[], None, None, None);
            let of = |name: &str| y.iter().filter(|(k, _)| k.as_string() == name).map(|(_, v)| *v).sum::<f64>();
            let worked = st.cities.iter().any(|c| c.worked.iter().any(|w| w.xy() == q.xy()));
            [of("YIELD_FOOD"), of("YIELD_PRODUCTION"), worked as i64 as f64, q.resource.is_some() as i64 as f64, q.improvement.is_some() as i64 as f64]
        }
        None => [MISSING; 5],
    };
    [cost, (base == Some(c)) as i64 as f64, turns, city[0], city[1], city[2], city[3], tile[0], tile[1], tile[2], tile[3], tile[4]]
}

/// Gradient boosted regression trees, as tools/learn_train.py writes them.
pub struct Trees {
    kinds: Vec<String>,
    names: Vec<String>,
    trees: Vec<Tree>,
}

struct Tree {
    /// internal nodes: (feature, threshold or categories, left child, right child); a child < 0 is the leaf !child
    nodes: Vec<(usize, Split, i64, i64)>,
    leaves: Vec<f64>,
}

enum Split {
    Le(f64),
    In(Vec<i64>),
}

impl Trees {
    pub fn load(path: &str) -> Trees {
        let v = json::parse(&std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap();
        let strs = |k: &str| v[k].as_array().unwrap().iter().map(|x| x.as_str().unwrap().to_string()).collect::<Vec<_>>();
        let numeric = strs("numeric");
        assert_eq!(numeric, NUMERIC.iter().map(|s| s.to_string()).collect::<Vec<_>>(), "{path}: trained on other features");
        let trees = v["trees"].as_array().unwrap().iter().map(|t| Tree {
            nodes: t["nodes"].as_array().unwrap().iter().map(|n| {
                let n = n.as_array().unwrap();
                let split = match &n[1] {
                    Value::Arr(cats) => Split::In(cats.iter().map(|c| c.as_i64().unwrap()).collect()),
                    x => Split::Le(x.as_f64().unwrap()),
                };
                (n[0].as_u64().unwrap() as usize, split, n[2].as_i64().unwrap(), n[3].as_i64().unwrap())
            }).collect(),
            leaves: t["leaves"].as_array().unwrap().iter().map(|x| x.as_f64().unwrap()).collect(),
        }).collect();
        let t = Trees { kinds: strs("kinds"), names: strs("names"), trees };
        for c in v["check"].as_array().unwrap() {
            let x: Vec<f64> = c[0].as_array().unwrap().iter().map(|v| v.as_f64().unwrap()).collect();
            let (want, got) = (c[1].as_f64().unwrap(), t.eval(&x));
            assert!((want - got).abs() < 1e-6 * want.abs().max(1.0), "{path}: the trees give {got} where LightGBM gave {want}");
        }
        t
    }

    fn eval(&self, x: &[f64]) -> f64 {
        self.trees.iter().map(|t| t.eval(x)).sum()
    }

    /// The predicted change of the final score when the option is chosen instead of the plan's choice.
    pub fn predict(&self, decision: &[f64], kind: Sym, option: &Choice, opt: &[f64; N_OPTION]) -> f64 {
        // a name the training did not see gets a code of no category, which every categorical split sends right
        let code = |vocab: &[String], s: &str| vocab.iter().position(|x| x == s).unwrap_or(vocab.len()) as f64;
        let mut x = vec![code(&self.kinds, &kind.as_string()), code(&self.names, &option_name(option))];
        x.extend_from_slice(decision);
        x.extend_from_slice(opt);
        self.eval(&x)
    }
}

impl Tree {
    fn eval(&self, x: &[f64]) -> f64 {
        if self.nodes.is_empty() {
            return self.leaves[0];
        }
        let mut i = 0i64;
        loop {
            let (f, split, l, r) = &self.nodes[i as usize];
            let v = x[*f];
            let left = match split {
                Split::Le(t) => v <= *t,
                Split::In(cats) => cats.contains(&(v as i64)),
            };
            i = if left { *l } else { *r };
            if i < 0 {
                return self.leaves[!i as usize];
            }
        }
    }
}
