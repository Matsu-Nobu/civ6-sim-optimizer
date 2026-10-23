//! What a decision's options are worth: the features of a decision and of each of its options, and a model of
//! gradient boosted trees (trained by tools/learn_train.py) that predicts how much choosing an option changes the
//! final score from the plan's own choice.
//!
//! A row describes one option of one decision: the kind of decision, a name for the option (categorical: an item, an
//! action, ...), and numbers of the state at the decision (the player's yields and stock), of the city the decision
//! belongs to (when it belongs to one), and of the option (its cost, whether the base policy picks it).
use crate::json::{self, Value};
use crate::model::*;
use crate::planner::{Chooser, Planner};
use crate::sym::Sym;

/// The value of a number the decision does not have (a city's when it belongs to none): below every real one, so
/// the trees need no rule for missing values.
pub const MISSING: f64 = -999.0;

pub const NUMERIC: [&str; 35 + N_OPTION] = [
    "turn", "n", "n_options", "food", "production", "gold", "science", "culture", "faith", "pop", "cities", "districts",
    "buildings", "improvements", "invested", "food_store", "charges", "settlers", "traders", "builders", "techs", "civics",
    "gold_stock", "military", "city_pop", "city_food_store", "city_housing_left", "city_districts", "city_buildings",
    "city_food", "city_production", "city_progress", "req_penalty", "req_research_left", "req_slack", "option_cost", "option_is_base", "option_turns", "target_pop",
    "target_housing_left", "target_food", "target_production", "tile_food", "tile_production", "tile_worked",
    "tile_resource", "tile_improved", "option_on_req_path", "option_reward",
];

/// The number of an option's own features (option_features).
pub const N_OPTION: usize = 14;

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
    let _g = crate::prof::start(14);
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
    f.extend_from_slice(&requirement_numbers(p, st, v[21], v[4]));
    f
}

/// What the score asks for (Planner::milestones): its penalty at the state (value::features' shortfall), the research
/// still needed for the required techs and civics (their paths' costs not done, less progress), and the turns that
/// research takes at the current science less the turns left to the evaluation (> 0: late at this pace).
fn requirement_numbers(p: &Planner, st: &State, penalty: f64, science: f64) -> [f64; 3] {
    let Some(m) = p.milestones.as_ref() else { return [0.0; 3] };
    let left: f64 = m.paths.iter().flat_map(|(_, path)| path.iter())
        .filter(|(x, _)| !st.acc.techs.contains(x) && !st.acc.civics.contains(x))
        .map(|(x, c)| (c - st.acc.tech_progress.get(x).or(st.acc.civic_progress.get(x)).copied().unwrap_or(0.0)).max(0.0)).sum();
    let turns_left = (p.s.horizon - st.turn + 1) as f64;
    [penalty, left, left / science.max(1.0) - turns_left]
}

/// The numbers of one option (appended to the decision's): its cost and whether the base policy (`base`,
/// Planner::base) picks it; the turns it takes the city to make it; the city it acts on (a builder's, a purchase's,
/// a governor's, a route's destination; else the decision's city); the tile it acts on (a builder's, a bought one, a
/// new city's site): its yields before the action, whether a citizen works it, has a resource, an improvement;
/// whether it is a required tech or civic or one on the way to it, and the points the score gives for one (a rewarded
/// unit).
pub fn option_features(p: &Planner, st: &State, key: &Key, base: Option<&Choice>, c: &Choice, eng: &crate::engine::Engine) -> [f64; N_OPTION] {
    let _g = crate::prof::start(15);
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
    let item = match c {
        Choice::S(x) | Choice::BuyItem(_, x, _, _) => Some(*x),
        _ => None,
    };
    let (on_path, reward) = match (item, p.milestones.as_ref()) {
        (Some(x), Some(m)) => (m.paths.iter().any(|(_, path)| path.iter().any(|(y, _)| *y == x)) as i64 as f64,
                               m.reward.iter().filter(|(what, _)| *what == x.as_string()).map(|(_, pts)| *pts).sum::<f64>()),
        _ => (0.0, 0.0),
    };
    [cost, (base == Some(c)) as i64 as f64, turns, city[0], city[1], city[2], city[3], tile[0], tile[1], tile[2], tile[3], tile[4], on_path, reward]
}

/// The two parts of a play's score the model learns apart (`score` and `st` as opt::play returns them): the score
/// without what missed requirements cost (the objective and the rewards), and the research done by the end (the costs
/// of the techs and civics done, at the end's game era, and the progress on the others). A plan that keeps its
/// requirement by a narrow margin loses the whole requirement weight for one point of science short; apart, both
/// parts change smoothly with a decision.
pub fn score_parts(p: &Planner, score: f64, st: &State) -> (f64, f64) {
    let missed = p.milestones.as_ref().map(|m| m.require_penalty(st)).unwrap_or(0.0);
    let era = p.sc.game_era(st.turn);
    let cost = |x: &Sym| crate::sim::research_cost(p.r, *x, p.speed, era);
    let research: f64 = st.acc.techs.iter().chain(st.acc.civics.iter()).map(cost).sum::<f64>()
        + st.acc.tech_progress.0.iter().chain(st.acc.civic_progress.0.iter()).filter(|(x, _)| !st.acc.techs.contains(x) && !st.acc.civics.contains(x)).map(|x| x.1).sum::<f64>();
    (score + missed, research)
}

/// Gradient boosted regression trees, as tools/learn_train.py writes them.
pub struct Trees {
    kinds: std::collections::HashMap<String, usize>,
    names: std::collections::HashMap<String, usize>,
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
        let index = |v: Vec<String>| v.into_iter().enumerate().map(|(i, s)| (s, i)).collect();
        let t = Trees { kinds: index(strs("kinds")), names: index(strs("names")), trees };
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
        let _g = crate::prof::start(16);
        // a name the training did not see gets a code of no category, which every categorical split sends right
        let code = |vocab: &std::collections::HashMap<String, usize>, s: &str| vocab.get(s).copied().unwrap_or(vocab.len()) as f64;
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

/// Plays the plan's choices up to turn `from`, the model's in the turns after it up to `to`, and the plan's again
/// after `to`. At each decision with options the model takes the option `model` ranks first; with `research` (trees
/// of the change of the research done, tools/learn_train.py --target delta_research), only among the options it
/// predicts to lose at most `slack` research, else the one it predicts to lose least. Keeps every choice it makes in
/// `taken`, which plays the same game as a plan.
pub struct PolicyChooser<'g> {
    pub genome: &'g std::collections::HashMap<Key, Choice>,
    pub model: &'g Trees,
    pub research: Option<&'g Trees>,
    pub slack: f64,
    pub from: i64,
    pub to: i64,
    pub taken: std::collections::HashMap<Key, Choice>,
}

impl Chooser for PolicyChooser<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        let c = if st.turn <= self.from || st.turn > self.to || options.len() < 2 {
            crate::opt::GenomeChooser { genome: self.genome }.choose(p, st, key, options)?
        } else {
            let eng = p.engine(&p.player_env(st), &st.cities);
            let base = p.base(st, key, options);
            let d = decision_features(p, st, key, options, &eng);
            let of: Vec<_> = options.iter().map(|o| option_features(p, st, key, base.as_ref(), o, &eng)).collect();
            let scores: Vec<f64> = options.iter().zip(&of).map(|(o, f)| self.model.predict(&d, key.kind, o, f)).collect();
            let kept: Vec<usize> = match self.research {
                Some(r) => {
                    let lost: Vec<f64> = options.iter().zip(&of).map(|(o, f)| -r.predict(&d, key.kind, o, f)).collect();
                    let ok: Vec<usize> = (0..options.len()).filter(|&j| lost[j] <= self.slack).collect();
                    if ok.is_empty() { vec![(0..options.len()).fold(0, |b, j| if lost[j] < lost[b] { j } else { b })] } else { ok }
                }
                None => (0..options.len()).collect(),
            };
            let best = kept.iter().copied().fold(kept[0], |b, j| if scores[j] > scores[b] { j } else { b });
            options[best].clone()
        };
        self.taken.insert(key.clone(), c.clone());
        Some(c)
    }
}

/// The option `model` ranks first at a decision.
pub fn model_pick(model: &Trees, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
    if options.len() < 2 {
        return options.first().cloned();
    }
    let eng = p.engine(&p.player_env(st), &st.cities);
    let base = p.base(st, key, options);
    let d = decision_features(p, st, key, options, &eng);
    options.iter().map(|o| (model.predict(&d, key.kind, o, &option_features(p, st, key, base.as_ref(), o, &eng)), o))
        .fold(None, |b: Option<(f64, &Choice)>, x| if b.map(|b| x.0 > b.0).unwrap_or(true) { Some(x) } else { b }).map(|x| x.1.clone())
}

/// The model that takes a decision whose planned choice cannot be taken (opt::GenomeChooser), set once from
/// `--fallback-model`; without it the base policy takes it.
pub static FALLBACK: std::sync::OnceLock<Trees> = std::sync::OnceLock::new();
