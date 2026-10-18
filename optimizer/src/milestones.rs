//! Science and culture floors for a plan evaluated at an earlier turn: a tech (civic) due by a later deadline turn,
//! checked at the evaluation turn by projection. What the path to it still needs (the costs of its prerequisites not
//! done yet, less their progress) must not exceed what the plan can expect to earn by the deadline: its science
//! (culture) per turn at the evaluation turn times the recorded game's growth (the record's sum over the turns to the
//! deadline over its rate at the evaluation turn) plus the recorded boosts (inspirations) of those turns. A shortfall
//! costs `weight` per point.
use crate::model::*;
use crate::rules::Rules;
use crate::sim::{boost_amount, research_cost};
use crate::sym::Sym;
use std::collections::BTreeSet;

#[derive(Clone, Debug)]
pub struct Milestone {
    pub target: Sym,
    pub civic: bool,
    pub deadline: i64,
    /// the target and every prerequisite, with its cost at the deadline's game era
    pub costs: Vec<(Sym, f64)>,
    pub growth: f64,
    pub bonus: f64,
}

#[derive(Clone, Debug, Default)]
pub struct Milestones {
    pub list: Vec<Milestone>,
    pub weight: f64,
    /// what the plan must have at the evaluation turn: (district type placed, or "military" for units trained that
    /// do nothing for the objective, count); each one missing costs `require_weight`
    pub require: Vec<(String, i64)>,
    pub require_weight: f64,
    /// a required tech or civic ("TECH_X" / "CIVIC_X" in `require`): it and its prerequisites with their costs; one
    /// not done costs `require_weight` times the share of that path's cost still to research (a slope to climb)
    pub paths: Vec<(String, Vec<(Sym, f64)>)>,
    /// what the plan is rewarded for at the evaluation turn: (as in `require`, points per one), e.g. horsemen trained
    pub reward: Vec<(String, f64)>,
    /// a required great person class ("GREAT_PERSON_CLASS_X" in `require`): its individuals and the points one
    /// costs; until one is recruited it costs `require_weight` plus as much again times the share of those points still missing
    pub gp: Vec<(String, Vec<Sym>, f64)>,
    /// a turn of its own for an item of `require` (`reward`): checked at the end of that turn ("X:N@T"), else at the
    /// evaluation's end
    pub require_at: Vec<Option<i64>>,
    pub reward_at: Vec<Option<i64>>,
}

impl Milestone {
    /// (needed, expected) for a state at the end of the evaluation turn
    pub fn balance(&self, st: &State) -> (f64, f64) {
        let (done, prog) = if self.civic { (&st.acc.civics, &st.acc.civic_progress) } else { (&st.acc.techs, &st.acc.tech_progress) };
        let need: f64 = self.costs.iter().filter(|(x, _)| !done.contains(x)).map(|(x, c)| (c - prog.get(x).copied().unwrap_or(0.0)).max(0.0)).sum();
        let rate = st.last_output[if self.civic { 4 } else { 3 }];
        (need, rate * self.growth + self.bonus)
    }
}

impl Milestones {
    pub fn penalty(&self, st: &State) -> f64 {
        let floors: f64 = self.list.iter().map(|m| { let (need, have) = m.balance(st); self.weight * (need - have).max(0.0) }).sum();
        let at = |k: u32| st.req_at.iter().find(|x| x.0 == k).map(|x| x.1);
        let req: f64 = (0..self.require.len()).map(|i| at(i as u32).unwrap_or_else(|| self.req_term(i, st))).sum();
        let reward: f64 = (0..self.reward.len()).map(|j| at(REWARD_KEY + j as u32).unwrap_or_else(|| self.reward_term(j, st))).sum();
        floors + req - reward
    }

    /// The items due at the end of the turn just played (st.turn - 1): their terms kept in the state.
    pub fn record(&self, st: &mut State) {
        let done = st.turn - 1;
        for i in 0..self.require.len() {
            if self.require_at.get(i).copied().flatten() == Some(done) {
                let v = self.req_term(i, st);
                st.req_at.push((i as u32, v));
            }
        }
        for j in 0..self.reward.len() {
            if self.reward_at.get(j).copied().flatten() == Some(done) {
                let v = self.reward_term(j, st);
                st.req_at.push((REWARD_KEY + j as u32, v));
            }
        }
    }

    fn reward_term(&self, j: usize, st: &State) -> f64 {
        let (what, w) = &self.reward[j];
        w * have(st, what) as f64
    }

    fn req_term(&self, i: usize, st: &State) -> f64 {
        let (what, n) = &self.require[i];
        if let Some((_, members, need)) = self.gp.iter().find(|(w, _, _)| w == what) {
            let n_have = st.gp_ours.iter().filter(|g| members.contains(g)).count() as i64;
            if n_have >= *n {
                return 0.0;
            }
            let cls = what.split('>').next().unwrap();
            let pts = st.gp.get(&crate::sym::sym(cls)).copied().unwrap_or(0.0);
            return self.require_weight * ((n - n_have) as f64 + (1.0 - (pts / need).min(1.0)));
        }
        if let Some((_, path)) = self.paths.iter().find(|(w, _)| w == what) {
            if have(st, what) >= 1 {
                return 0.0;
            }
            let civic = what.starts_with("CIVIC_");
            let (done, prog) = if civic { (&st.acc.civics, &st.acc.civic_progress) } else { (&st.acc.techs, &st.acc.tech_progress) };
            let total: f64 = path.iter().map(|x| x.1).sum::<f64>().max(1.0);
            let left: f64 = path.iter().filter(|x| !done.contains(&x.0)).map(|x| (x.1 - prog.get(&x.0).copied().unwrap_or(0.0)).max(0.0)).sum();
            return self.require_weight * (1.0 + left / total);
        }
        self.require_weight * (n - have(st, what)).max(0) as f64
    }

    /// One line per requirement / reward: what it is, when it is checked, and its term for this state.
    pub fn report(&self, st: &State) -> Vec<String> {
        let at = |k: u32| st.req_at.iter().find(|x| x.0 == k).map(|x| x.1);
        let mut out = vec![];
        for i in 0..self.require.len() {
            let v = at(i as u32).unwrap_or_else(|| self.req_term(i, st));
            let when = self.require_at.get(i).copied().flatten().map(|t| format!("T{t}")).unwrap_or("end".into());
            out.push(format!("require {}:{} by {when}: {}", self.require[i].0, self.require[i].1, if v <= 0.0 { "met".to_string() } else { format!("NOT met, penalty {v:.0}") }));
        }
        for j in 0..self.reward.len() {
            let v = at(REWARD_KEY + j as u32).unwrap_or_else(|| self.reward_term(j, st));
            let when = self.reward_at.get(j).copied().flatten().map(|t| format!("T{t}")).unwrap_or("end".into());
            out.push(format!("reward {} by {when}: {v:.0}", self.reward[j].0));
        }
        out
    }
}

/// st.req_at keys: requirement i is i, reward j is REWARD_KEY + j
const REWARD_KEY: u32 = 1 << 16;

/// How many of `what` the plan has: districts of a type placed, cities with a building (or wonder), ("military") idle units trained, units of a type
/// trained, or ("civics") civics completed.
pub fn have(st: &State, what: &str) -> i64 {
    if what == "military" {
        return st.acc.units_trained.0.iter().filter(|(u, _)| crate::opt::idle_unit(*u)).map(|(_, n)| *n).sum();
    }
    if what == "civics" {
        return st.acc.civics.len() as i64;
    }
    if what.starts_with("TECH_") {
        return st.acc.techs.iter().any(|x| x.as_string() == what) as i64;
    }
    if what.starts_with("CIVIC_") {
        return st.acc.civics.iter().any(|x| x.as_string() == what) as i64;
    }
    if what.starts_with("BUILDING_") {
        return st.cities.iter().filter(|c| c.buildings.iter().any(|b| b.as_string() == what)).count() as i64;
    }
    if what.starts_with("UNIT_") {
        return st.acc.units_trained.0.iter().filter(|(u, _)| u.as_string() == what).map(|(_, n)| *n).sum();
    }
    st.cities.iter().flat_map(|c| c.districts.iter()).filter(|d| d.dtype.as_string() == what).count() as i64
}

pub fn closure(r: &Rules, goal: Sym, civic: bool) -> BTreeSet<Sym> {
    let (t, k, c) = if civic { ("CivicPrereqs", "Civic", "PrereqCivic") } else { ("TechnologyPrereqs", "Technology", "PrereqTech") };
    let mut out = BTreeSet::new();
    let mut todo = vec![goal];
    while let Some(x) = todo.pop() {
        if out.insert(x) {
            todo.extend(r.list_sym(t, k, x, c));
        }
    }
    out
}

/// One milestone calibrated on the recorded game (problem rows): `at` the evaluation turn.
pub fn calibrate(r: &Rules, rows: &crate::json::Value, sc: &crate::scenario::Scenario, speed: Sym, target: Sym, civic: bool,
                 deadline: i64, at: i64) -> Milestone {
    let rows = rows.as_array().unwrap();
    let row = |t: i64| rows.iter().find(|d| d["turn"].as_i64() == Some(t));
    let ykey = if civic { "YIELD_CULTURE" } else { "YIELD_SCIENCE" };
    // the yield during turn t is the next save's "previous turn" yields
    let y = |t: i64| row(t + 1).and_then(|d| d["player"]["yields"][ykey].as_f64());
    let last = (at..deadline).rev().find_map(|t| y(t)).unwrap_or(0.0);
    let base = y(at).unwrap_or(1.0).max(1e-9);
    let sum: f64 = (at + 1..deadline).map(|t| y(t).unwrap_or(last)).sum();
    let era = sc.game_era(deadline.min(rows.iter().filter_map(|d| d["turn"].as_i64()).max().unwrap_or(deadline)));
    let cost = |x: Sym| research_cost(r, x, speed, era);
    let bkey = if civic { "civicsInspired" } else { "techsBoosted" };
    let set = |t: i64| -> BTreeSet<Sym> { row(t).map(|d| d["player"][bkey].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(crate::sym::sym)).collect()).unwrap_or_default()).unwrap_or_default() };
    let end = rows.iter().filter_map(|d| d["turn"].as_i64()).max().unwrap_or(deadline).min(deadline);
    let (b0, b1) = (set(at + 1), set(end));
    let bonus: f64 = b1.difference(&b0).map(|x| boost_amount(cost(*x), 0.0)).sum();
    let costs = closure(r, target, civic).into_iter().map(|x| (x, cost(x))).collect();
    Milestone { target, civic, deadline, costs, growth: sum / base, bonus }
}
