//! A learned value of game states and the lookahead that uses it to take the decisions a plan does not fix.
//!
//! The value of the state at the start of turn t is a linear function of a few general measures of the state (the
//! yields of the last turn, population, cities, districts, buildings, research progress, what the requirements still
//! miss, ...), with weights of its own for each turn, fitted (ridge regression) to the final scores of many plays.
//! A decision the genome does not fix (a new city's production, a new builder's work) is taken by trying each option:
//! the turn is played again from its start with that option, then a few more turns, and the state reached is valued.
use crate::json::Value;
use crate::model::*;
use crate::planner::Planner;
use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::OnceLock;

pub const N_FEATURES: usize = 22;

/// The general measures of a state the value is linear in (the first is a constant 1).
pub fn features(p: &Planner, st: &State) -> [f64; N_FEATURES] {
    let o = st.last_output;
    let pop: f64 = st.cities.iter().map(|c| st.cacc(c.name).population as f64).sum();
    let districts = st.cities.iter().flat_map(|c| c.districts.iter()).filter(|d| d.complete).count() as f64;
    let buildings: f64 = st.cities.iter().map(|c| c.buildings.len() as f64).sum();
    let improvements = st.cities.iter().flat_map(|c| c.owned.iter()).filter(|q| q.improvement.is_some()).count() as f64;
    let invested: f64 = st.cities.iter().map(|c| st.cacc(c.name).progress.0.iter().map(|x| x.1).sum::<f64>()).sum();
    let food_store: f64 = st.cities.iter().map(|c| st.cacc(c.name).food_store).sum();
    let charges: f64 = st.builders.iter().map(|b| b.charges as f64).sum();
    let tech_progress: f64 = st.acc.tech_progress.0.iter().map(|x| x.1).sum();
    let civic_progress: f64 = st.acc.civic_progress.0.iter().map(|x| x.1).sum();
    let military = crate::milestones::have(st, "military") as f64;
    let shortfall = p.milestones.as_ref().map(|m| m.penalty(st)).unwrap_or(0.0);
    [1.0, o[0], o[1], o[2], o[3], o[4], o[5], pop, st.cities.len() as f64, districts, buildings, improvements, invested,
     food_store, charges, st.settlers.len() as f64, st.traders.len() as f64, st.acc.techs.len() as f64 * 10.0 + tech_progress / 10.0,
     st.acc.civics.len() as f64 * 10.0 + civic_progress / 10.0, st.acc.gold, military, shortfall]
}

/// Weights per turn (the turn the state starts), fitted on standardized features.
pub struct ValueModel {
    pub turns: HashMap<i64, ([f64; N_FEATURES], [f64; N_FEATURES], [f64; N_FEATURES])>,
}

impl ValueModel {
    /// The value of a state at the start of its turn; a turn the model was not fitted for falls back to the nearest
    /// earlier one.
    pub fn value(&self, p: &Planner, st: &State) -> f64 {
        let t = (0..=st.turn).rev().find(|t| self.turns.contains_key(t)).expect("value model: no turn at or before the state's");
        let (w, mean, sd) = &self.turns[&t];
        let x = features(p, st);
        w[0] + (1..N_FEATURES).map(|i| w[i] * (x[i] - mean[i]) / sd[i]).sum::<f64>()
    }

    /// Ridge regression of `scores` on the features of `states` (rows of one turn).
    pub fn fit_turn(rows: &[[f64; N_FEATURES]], scores: &[f64], ridge: f64) -> ([f64; N_FEATURES], [f64; N_FEATURES], [f64; N_FEATURES]) {
        let n = rows.len() as f64;
        let mut mean = [0.0; N_FEATURES];
        let mut sd = [1.0; N_FEATURES];
        for i in 1..N_FEATURES {
            mean[i] = rows.iter().map(|r| r[i]).sum::<f64>() / n;
            let v = rows.iter().map(|r| (r[i] - mean[i]).powi(2)).sum::<f64>() / n;
            sd[i] = if v > 1e-12 { v.sqrt() } else { 1.0 };
        }
        let z: Vec<[f64; N_FEATURES]> = rows.iter().map(|r| {
            let mut x = [1.0; N_FEATURES];
            for i in 1..N_FEATURES {
                x[i] = (r[i] - mean[i]) / sd[i];
            }
            x
        }).collect();
        // normal equations (X'X + ridge I, the intercept not shrunk) solved by Gaussian elimination
        let mut a = [[0.0; N_FEATURES + 1]; N_FEATURES];
        for (x, y) in z.iter().zip(scores) {
            for i in 0..N_FEATURES {
                for j in 0..N_FEATURES {
                    a[i][j] += x[i] * x[j];
                }
                a[i][N_FEATURES] += x[i] * y;
            }
        }
        for (i, row) in a.iter_mut().enumerate().skip(1) {
            row[i] += ridge * n;
        }
        for c in 0..N_FEATURES {
            let piv = (c..N_FEATURES).max_by(|&i, &j| a[i][c].abs().partial_cmp(&a[j][c].abs()).unwrap()).unwrap();
            a.swap(c, piv);
            let d = a[c][c];
            if d.abs() < 1e-12 {
                continue;
            }
            for j in c..=N_FEATURES {
                a[c][j] /= d;
            }
            for i in 0..N_FEATURES {
                if i != c {
                    let f = a[i][c];
                    for j in c..=N_FEATURES {
                        a[i][j] -= f * a[c][j];
                    }
                }
            }
        }
        let mut w = [0.0; N_FEATURES];
        for i in 0..N_FEATURES {
            w[i] = a[i][N_FEATURES];
        }
        (w, mean, sd)
    }

    pub fn to_json(&self) -> Value {
        let mut turns: Vec<_> = self.turns.iter().collect();
        turns.sort_by_key(|x| x.0);
        let arr = |v: &[f64; N_FEATURES]| Value::Arr(v.iter().map(|x| Value::from(*x)).collect());
        Value::Arr(turns.into_iter().map(|(t, (w, m, s))| Value::Arr(vec![Value::from(*t), arr(w), arr(m), arr(s)])).collect())
    }

    pub fn from_json(v: &Value) -> ValueModel {
        let arr = |x: &Value| {
            let mut a = [0.0; N_FEATURES];
            for (i, y) in x.as_array().unwrap().iter().enumerate() {
                a[i] = y.as_f64().unwrap();
            }
            a
        };
        ValueModel { turns: v.as_array().unwrap().iter().map(|e| (e[0].as_i64().unwrap(), (arr(&e[1]), arr(&e[2]), arr(&e[3])))).collect() }
    }
}

/// The lookahead's settings: the model and how many turns after the decision's turn are played before valuing.
pub struct Lookahead {
    pub model: ValueModel,
    pub turns: i64,
}

/// Set once (civ6opt --value); without it, decisions the genome does not fix take the base policy.
pub static LOOKAHEAD: OnceLock<Lookahead> = OnceLock::new();

thread_local! {
    /// The state at the start of the turn being played (Planner::advance keeps it while a lookahead is set).
    static TURN_START: RefCell<Option<State>> = const { RefCell::new(None) };
    /// Inside a lookahead's own plays: decisions there take the base policy.
    static INSIDE: RefCell<bool> = const { RefCell::new(false) };
}

/// Called by Planner::advance at the start of every turn.
pub fn turn_start(st: &State) {
    if LOOKAHEAD.get().is_some() && !INSIDE.with(|x| *x.borrow()) {
        TURN_START.with(|x| *x.borrow_mut() = Some(st.clone()));
    }
}

/// The option of decision `key` with the best value `look.turns` turns after this one, the turn replayed from its
/// start with the decisions already taken in it (from `st.log`) and `genome` elsewhere; None outside a lookahead's
/// reach (no turn start kept, or inside a lookahead's own plays).
pub fn choose(p: &Planner, st: &State, key: &Key, options: &[Choice], genome: &HashMap<Key, Choice>) -> Option<Choice> {
    let look = LOOKAHEAD.get()?;
    if options.len() < 2 || INSIDE.with(|x| *x.borrow()) {
        return None;
    }
    let start = TURN_START.with(|x| x.borrow().clone())?;
    if start.turn != st.turn {
        return None;
    }
    let mut g = genome.clone();
    for (k, t, c) in st.log.iter().rev().take_while(|e| e.1 == st.turn) {
        debug_assert_eq!(*t, st.turn);
        if let Some(c) = c {
            g.insert(k.clone(), c.clone());
        }
    }
    INSIDE.with(|x| *x.borrow_mut() = true);
    let mut best: Option<(f64, &Choice)> = None;
    for o in options {
        g.insert(key.clone(), o.clone());
        let mut s = start.clone();
        let end = (st.turn + look.turns).min(p.s.horizon);
        while s.turn <= end {
            p.advance(&mut s, &mut crate::opt::GenomeChooser { genome: &g });
        }
        let v = look.model.value(p, &s);
        if best.map(|b| v > b.0).unwrap_or(true) {
            best = Some((v, o));
        }
    }
    INSIDE.with(|x| *x.borrow_mut() = false);
    best.map(|b| b.1.clone())
}
