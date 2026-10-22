//! civ6opt: the Rust simulator / planner / search.
//!
//!   civ6opt run <problem.json> [--turn 30] [--genome recorded|recorded_tiles|base|<file.json>]
//!       (recorded_tiles: the recorded game with its cities' tiles as recorded, Settings.recorded_tiles)
//!       play a plan; one JSON line per turn (cities' yields, population, items, worked plots), then the total
//!   civ6opt search <problem.json> [--turn 30] [--budget 20000] [--runs 12] [--threads 12] [--init recorded|none]
//!       [--out result.json]
//!   civ6opt decisions <problem.json> [--turn 30] [--genome ...]
//!       every decision of a plan: turn, key, choice, number of options
//!   civ6opt compare <problem.json> --key '["governor", "", 0]' [--turn 30] [--genome ...] [--budget 5000] [--runs 2]
//!       [--max-options 12]
//!       one decision, each option fixed in turn and the rest searched again from the plan: best score and loss
//!   civ6opt validate <problem.json> --turn 60 [--from 2]
//!       each recorded turn from its save: one turn with the recorded choices, compared with the next save
//!   all: [--ext 10] [--ext-weight 1.0]  the evaluation adds weight x the objective of 10 more turns played by the
//!       fixed extension policy (opt::ExtChooser), so the plan's state at --turn counts
//!       [--rounds 1]
//!       NRPA runs (incumbent prior) in parallel; with rounds, each round's runs start from the best plan so far
//!       (budget split over the rounds); prints each run's best, saves the best plan
// musl's malloc takes one global lock: the musl build uses the thread-caching allocator; glibc's is used as it is
#[cfg(target_env = "musl")]
#[global_allocator]
static GLOBAL: civ6opt::alloc::Fast = civ6opt::alloc::Fast;
// Windows: the system heap does not scale over the search threads; mimalloc does
#[cfg(windows)]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

#[macro_use]
extern crate civ6opt;
use civ6opt::model::*;
use civ6opt::opt::{self, GenomeChooser};
use civ6opt::planner::Planner;
use civ6opt::rules::Rules;
use civ6opt::scenario::{choice_from, choice_json, key_from, key_json, Scenario, Settings};
use civ6opt::sym::{sym, Sym};
use civ6opt::world::{Map, World};
use civ6opt::json::{self, Value};
use std::collections::HashMap;
use std::time::Instant;

/// Boost triggers by events the model does not play (the record supplies them): meeting, exploring, combat,
/// religion, alliances, military formations (plan2.EXOGENOUS_TRIGGERS and more).
const EXOGENOUS_BOOSTS: [&str; 15] = ["BOOST_TRIGGER_MEET_CIV", "BOOST_TRIGGER_MEET_X_CITY_STATES", "BOOST_TRIGGER_FIND_NATURAL_WONDER",
    "BOOST_TRIGGER_KILL_WITH", "BOOST_TRIGGER_CLEAR_CAMP", "BOOST_TRIGGER_NUM_BARBS_KILLED", "BOOST_TRIGGER_RECEIVE_DOW",
    "BOOST_TRIGGER_HAVE_AN_ALLIANCE", "BOOST_TRIGGER_HAVE_ALLIANCE_LEVEL_X", "BOOST_TRIGGER_FOUND_RELIGION",
    "BOOST_TRIGGER_HAVE_X_CITIES_FOLLOWING_YOUR_RELIGION", "BOOST_TRIGGER_HAVE_X_LAND_UNITS", "BOOST_TRIGGER_LAND_UNIT_LEVEL",
    "BOOST_TRIGGER_HAVE_X_CORPS", "BOOST_TRIGGER_HAVE_X_ARMIES"];

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

struct Problem {
    v: Value,
    rules: &'static Rules,
}

fn load(path: &str) -> Problem {
    let t = Instant::now();
    let v: Value = json::parse(&std::fs::read_to_string(path).expect("read problem")).expect("parse problem");
    let rules: &'static Rules = Box::leak(Box::new(Rules::load(&v["rules"])));
    eprintln!("loaded {path} in {:.1}s", t.elapsed().as_secs_f64());
    Problem { v, rules }
}

fn planner(p: &Problem) -> (Planner<'static>, State) {
    let r = p.rules;
    let start = state_from(&p.v["start"]);
    let map = Map::load(&p.v["map"]);
    let world = World::new(r, map, start.player.player_id.unwrap_or(0));
    let sc = Scenario::load(&p.v["rows"], &p.v["scenario"]);
    let s = Settings::load(&p.v["settings"]);
    let speed = sym(p.v["settings"]["speed"].as_str().unwrap_or("GAMESPEED_ONLINE"));
    let establish: HashMap<_, _> = p.v["establish_turns"].as_object().unwrap().iter().map(|(k, v)| (sym(k), v.as_i64().unwrap())).collect();
    let mut pl = Planner::new(r, world, sc, s, speed, establish);
    pl.record_exo_boosts(&p.v["rows"]);
    pl.record_captures(&p.v["rows"]);
    pl.milestones = MILESTONES.get().cloned().flatten();
    pl.s.allow_idle_units = ALLOW_MILITARY.get().copied().unwrap_or(false);
    pl.s.no_harvest_before = NO_HARVEST_BEFORE.get().copied().unwrap_or(0);
    if TEAM_LUX.get().copied().unwrap_or(false) {
        pl.sc.extra_luxuries = infer_extra_luxuries(p, &pl);
        if civ6opt::envcfg::trace() == Some("amen") { let mut v: Vec<_> = pl.sc.extra_luxuries.iter().collect(); v.sort(); eprintln!("EXTRA {v:?}"); }
        pl.sc.allies = infer_allies(p, start.player.player_id);
    }
    if let Some(a) = ALLIES.get().cloned().flatten() {
        pl.sc.allies = a;
    }
    if GOLD_OUT.get().copied().unwrap_or(false) {
        // --gold-out: the gold the recorded player gave to allies is gone from every plan too
        let rec = civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]);
        pl.sc.gold_out = rec.gold_given(&pl.sc.gold_gift);
    }
    (pl, start)
}

/// A rollout trial: (job id, start state, objective before it, plan) -> (job id, score).
type RoJob = (usize, State, f64, HashMap<Key, Choice>);
/// A plan's decisions (turn, key, choice, options, and when the search learns: the features of crate::learn of the
/// decision and of each option).
type RoLog = Vec<(i64, Key, Choice, Vec<Choice>, Option<std::rc::Rc<DecFeats>>)>;
type DecFeats = (Vec<f64>, Vec<[f64; civ6opt::learn::N_OPTION]>);

/// The plan's choices, recording its decisions (with their features of crate::learn when `feats`).
struct RoRecorder<'g> {
    genome: &'g HashMap<Key, Choice>,
    feats: bool,
    log: RoLog,
}

impl civ6opt::planner::Chooser for RoRecorder<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        let c = GenomeChooser { genome: self.genome }.choose(p, st, key, options)?;
        let f = (self.feats && options.len() > 1).then(|| {
            let base = p.base(st, key, options);
            let eng = p.engine(&p.player_env(st), &st.cities);
            let of = options.iter().map(|o| civ6opt::learn::option_features(p, st, key, base.as_ref(), o, &eng)).collect();
            std::rc::Rc::new((civ6opt::learn::decision_features(p, st, key, options, &eng), of))
        });
        self.log.push((st.turn, key.clone(), c.clone(), options.to_vec(), f));
        Some(c)
    }
}

/// What the rollout learns from and with: the option model that picks the options worth trying, and the file the
/// trials are written to as rows of crate::learn.
#[derive(Default)]
struct Learning {
    model: Option<civ6opt::learn::Trees>,
    /// only the options the model predicts best are tried, this many per decision
    top: usize,
    rows: Option<std::io::BufWriter<std::fs::File>>,
    /// the number of the next decision written to `rows`
    decisions: usize,
}

impl Learning {
    fn from_args(args: &[String]) -> Learning {
        let rows = arg(args, "--rows").map(|path| {
            use std::io::Write as _;
            let mut w = std::io::BufWriter::new(std::fs::File::create(&path).unwrap());
            writeln!(w, "plan\tdecision\tkind\tname\t{}\tchosen\tdelta", civ6opt::learn::NUMERIC.join("\t")).unwrap();
            w
        });
        Learning {
            model: arg(args, "--model").map(|path| civ6opt::learn::Trees::load(&path)),
            top: arg(args, "--top").map(|x| x.parse().unwrap()).unwrap_or(3),
            rows,
            decisions: 0,
        }
    }

    fn on(&self) -> bool {
        self.model.is_some() || self.rows.is_some()
    }

    /// The options of decision `d` other than its choice, the model's best first; all of them without a model.
    fn alternatives(&self, d: &(i64, Key, Choice, Vec<Choice>, Option<std::rc::Rc<DecFeats>>)) -> Vec<Choice> {
        let alts: Vec<usize> = (0..d.3.len()).filter(|&j| d.3[j] != d.2).collect();
        let (Some(m), Some(f)) = (&self.model, &d.4) else { return alts.into_iter().map(|j| d.3[j].clone()).collect() };
        let mut scored: Vec<(f64, usize)> = alts.into_iter().map(|j| (m.predict(&f.0, d.1.kind, &d.3[j], &f.1[j]), j)).collect();
        scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
        scored.into_iter().take(self.top).map(|(_, j)| d.3[j].clone()).collect()
    }

    /// Writes the trials of decision `d` (the scores of `alts`, the plan scoring `best`) as rows.
    fn write(&mut self, d: &(i64, Key, Choice, Vec<Choice>, Option<std::rc::Rc<DecFeats>>), alts: &[Choice], scores: &[f64], best: f64) {
        let (Some(w), Some(f)) = (&mut self.rows, &d.4) else { return };
        use std::io::Write as _;
        let n = self.decisions;
        self.decisions += 1;
        let mut row = |c: &Choice, delta: f64| {
            let j = d.3.iter().position(|o| o == c).unwrap();
            let nums: Vec<String> = f.0.iter().chain(f.1[j].iter()).map(|x| format!("{x}")).collect();
            writeln!(w, "0\t{n}\t{}\t{}\t{}\t{}\t{delta}", d.1.kind.as_string(), civ6opt::learn::option_name(c), nums.join("\t"), (*c == d.2) as i64).unwrap();
        };
        row(&d.2, 0.0);
        for (a, s) in alts.iter().zip(scores) {
            row(a, s - best);
        }
    }
}

/// `threads` workers, each with its own planner, playing rollout trials to the end of the evaluation.
fn rollout_workers<'s, 'e>(sc: &'s std::thread::Scope<'s, 'e>, p: &'e Problem, threads: usize, turn: i64, ext: i64, horizon: opt::Horizon)
    -> (std::sync::mpsc::Sender<RoJob>, std::sync::mpsc::Receiver<(usize, f64)>) {
    let (job_tx, job_rx) = std::sync::mpsc::channel::<RoJob>();
    let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
    let (res_tx, res_rx) = std::sync::mpsc::channel::<(usize, f64)>();
    for _ in 0..threads {
        let (job_rx, res_tx) = (job_rx.clone(), res_tx.clone());
        sc.spawn(move || {
            let (mut wpl, wst0) = planner(p);
            wpl.s.horizon = turn + ext;
            loop {
                let job = { job_rx.lock().unwrap().recv() };
                let Ok((id, start, prefix, cand)) = job else { break };
                let sc = opt::play_from(&wpl, &wst0, &start, prefix, &mut GenomeChooser { genome: &cand }, horizon);
                if res_tx.send((id, sc)).is_err() {
                    break;
                }
            }
        });
    }
    (job_tx, res_rx)
}

/// Rollout policy improvement (Bertsekas' rollout algorithm; the pilot method) of plan `g` with the workers: its
/// decisions are taken in order; at each, every other option is tried (that option, the plan's choices elsewhere,
/// played to the end from the state at the start of the turn); the best, if better, replaces it, the game is replayed
/// from that turn on and the next decision follows. A pass never lowers the score; passes repeat until one changes
/// nothing (or `passes`, or `deadline`). Only decisions of turns in `span` are tried, none in `frozen` nor one whose
/// choice is `kept`. `on_better` sees every better plan. Returns the plan's score.
struct Rollout<'a> {
    pl: &'a Planner<'static>,
    st0: &'a State,
    turn: i64,
    horizon: opt::Horizon,
    job_tx: std::sync::mpsc::Sender<RoJob>,
    res_rx: std::sync::mpsc::Receiver<(usize, f64)>,
    tried: usize,
    learning: Learning,
}

impl Rollout<'_> {
    /// The plan's score, decisions and the state (with the objective so far) at the start of every turn.
    fn describe(&self, g: &HashMap<Key, Choice>) -> (f64, RoLog, Vec<(State, f64)>) {
        let mut rec = RoRecorder { genome: g, feats: self.learning.on(), log: vec![] };
        let mut st = self.st0.clone();
        let mut total = 0.0;
        let mut snaps = vec![];
        while st.turn <= self.turn {
            snaps.push((st.clone(), total));
            self.pl.advance(&mut st, &mut rec);
            total += opt::objective(&st.last_output);
        }
        let score = opt::play(self.pl, self.st0, &mut GenomeChooser { genome: g }, self.horizon).0;
        (score, rec.log.into_iter().filter(|d| d.0 <= self.turn).collect(), snaps)
    }

    /// The scores of plans that differ from `g` in decision `key` (of turn `t`), started from snapshot `snap`.
    fn trials(&mut self, g: &HashMap<Key, Choice>, snap: &(State, f64), key: &Key, alts: &[Choice]) -> Vec<f64> {
        for (j, a) in alts.iter().enumerate() {
            let mut cand = g.clone();
            cand.insert(key.clone(), a.clone());
            self.job_tx.send((j, snap.0.clone(), snap.1, cand)).unwrap();
        }
        let mut scores = vec![f64::MIN; alts.len()];
        for _ in 0..alts.len() {
            let (j, sc) = self.res_rx.recv().unwrap();
            scores[j] = sc;
        }
        self.tried += alts.len();
        scores
    }

    #[allow(clippy::too_many_arguments)]
    fn improve(&mut self, g: &mut HashMap<Key, Choice>, span: (i64, i64), frozen: &[Key], kept: &dyn Fn(&Choice) -> bool, passes: usize,
               deadline: Instant, quiet: bool, on_better: &mut dyn FnMut(usize, i64, f64, &HashMap<Key, Choice>)) -> f64 {
        let t0 = Instant::now();
        let (mut best, mut log, mut snaps) = self.describe(g);
        let first = self.st0.turn;
        'passes: for pass in 0..passes {
            let mut changed = 0;
            let mut i = 0;
            while i < log.len() {
                if Instant::now() > deadline {
                    break 'passes;
                }
                let d = log[i].clone();
                let (t, key, ch) = (d.0, d.1.clone(), d.2.clone());
                if d.3.len() < 2 || t < span.0 || t > span.1 || frozen.contains(&key) || kept(&ch) {
                    i += 1;
                    continue;
                }
                let alts = self.learning.alternatives(&d);
                let snap = snaps[(t - first) as usize].clone();
                let scores = self.trials(g, &snap, &key, &alts);
                self.learning.write(&d, &alts, &scores, best);
                let (jb, sb) = scores.iter().enumerate().fold((0, f64::MIN), |b, (j, s)| if *s > b.1 { (j, *s) } else { b });
                if sb > best + 1e-9 {
                    g.insert(key.clone(), alts[jb].clone());
                    let before = best;
                    let (nb, nlog, nsnaps) = self.describe(g);
                    if (nb - sb).abs() > 1e-6 {
                        eprintln!("warning: the replayed plan scores {nb:.3}, its trial {sb:.3}");
                    }
                    if !quiet {
                        eprintln!("pass {pass} T{t} {} {} -> {}: {before:.1} -> {nb:.1} ({:.0}s, {} tried)", key.kind.as_string(),
                                  choice_json(&ch), choice_json(&alts[jb]), t0.elapsed().as_secs_f64(), self.tried);
                    }
                    best = nb;
                    log = nlog;
                    snaps = nsnaps;
                    changed += 1;
                    on_better(pass, t, best, g);
                    // the same position in the new game: the first decision of this turn after this one
                    i = log.iter().position(|d| d.0 == t && d.1 == key).map(|x| x + 1).unwrap_or_else(|| log.iter().position(|d| d.0 > t).unwrap_or(log.len()));
                    continue;
                }
                i += 1;
            }
            if !quiet {
                eprintln!("pass {pass} done: {changed} changes, best {best:.1} ({:.0}s)", t0.elapsed().as_secs_f64());
            }
            if changed == 0 {
                break;
            }
        }
        best
    }
}

/// Large neighbourhood search: the incumbent plan is searched again a window of `window` turns at a time
/// (rollouts start from the incumbent's state at the window; decisions after it take the incumbent's choice, else
/// the base policy's), a better plan (by a full replay) replaces it; `passes` over all windows.
#[allow(clippy::too_many_arguments)]
fn lns(p: &Problem, g: HashMap<Key, Choice>, turn: i64, ext: i64, horizon: opt::Horizon, window: i64, budget: usize, threads: usize,
       passes: usize, tag: &str) -> (f64, HashMap<Key, Choice>, Vec<Value>) {
    lns_span(p, g, turn, ext, horizon, window, budget, threads, passes, tag, (0, turn), 0)
}

/// The LNS over the windows that start in `span` (turns), `seed0` varying the random numbers.
#[allow(clippy::too_many_arguments)]
fn lns_span(p: &Problem, mut g: HashMap<Key, Choice>, turn: i64, ext: i64, horizon: opt::Horizon, window: i64, budget: usize, threads: usize,
       passes: usize, tag: &str, span: (i64, i64), seed0: u64) -> (f64, HashMap<Key, Choice>, Vec<Value>) {
    let (mut pl, st0) = planner(p);
    pl.s.horizon = turn + ext;
    let eval = |g: &HashMap<Key, Choice>| opt::play(&pl, &st0, &mut GenomeChooser { genome: g }, horizon).0;
    let mut best = eval(&g);
    let t0 = Instant::now();
    if !tag.starts_with('-') {
        eprintln!("{tag}start {best:.1}");
    }
    let mut hist: Vec<Value> = vec![];
    for pass in 0..passes {
        let mut a = st0.turn.max(span.0);
        while a <= turn.min(span.1) {
            let b = (a + window - 1).min(turn);
            let h = opt::Horizon { free: Some((a, b)), ..horizon };
            let found = std::sync::Mutex::new(vec![]);
            if threads == 1 {
                // in this thread, with its planner (one planner's caches per thread)
                let mut sa = st0.clone();
                let mut gc = GenomeChooser { genome: &g };
                while sa.turn < a {
                    pl.advance(&mut sa, &mut gc);
                }
                let seed = seed0.wrapping_mul(1_000_003) ^ (pass * 100000 + a as usize * 100) as u64;
                let s = opt::nrpa(&pl, &sa, h, budget, 2, 1.0, 3.0, seed, Some(&g), None);
                found.lock().unwrap().push(s.best_seq);
            } else {
            std::thread::scope(|sc| {
                for j in 0..threads {
                    let (g, found) = (&g, &found);
                    sc.spawn(move || {
                        let (mut pl, mut sa) = planner(p);
                        pl.s.horizon = turn + ext;
                        let mut gc = GenomeChooser { genome: g };
                        while sa.turn < a {
                            pl.advance(&mut sa, &mut gc);
                        }
                        let seed = seed0.wrapping_mul(1_000_003) ^ (pass * 100000 + a as usize * 100 + j) as u64;
                        let s = opt::nrpa(&pl, &sa, h, budget, 2, 1.0, 3.0, seed, Some(g), None);
                        found.lock().unwrap().push(s.best_seq);
                    });
                }
            });
            }
            let before = best;
            for seq in found.into_inner().unwrap() {
                let mut cand = g.clone();
                cand.extend(opt::genome_of(&seq));
                let sc = eval(&cand);
                if sc > best + 1e-9 {
                    best = sc;
                    g = cand;
                }
            }
            if !tag.starts_with('-') {
                eprintln!("{tag}pass {pass} turns {a}-{b}: {before:.1} -> {best:.1} ({:.0}s)", t0.elapsed().as_secs_f64());
            }
            hist.push(Value::Arr(vec![Value::from(pass as i64), Value::from(a), Value::from(best), Value::from(t0.elapsed().as_secs_f64())]));
            a += window;
        }
    }
    (best, g, hist)
}

/// A plan (genome) as a search result file.
/// The option the model (crate::learn) ranks first among the options of decision `key` other than `ch`, at the state
/// the plan `g` reaches that decision in.
fn model_first(pl: &Planner, st0: &State, g: &HashMap<Key, Choice>, model: &civ6opt::learn::Trees, key: &Key, ch: &Choice) -> Option<Choice> {
    struct At<'a> { g: &'a HashMap<Key, Choice>, model: &'a civ6opt::learn::Trees, key: &'a Key, ch: &'a Choice, found: Option<Choice> }
    impl civ6opt::planner::Chooser for At<'_> {
        fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
            if key == self.key && self.found.is_none() {
                let eng = p.engine(&p.player_env(st), &st.cities);
                let base = p.base(st, key, options);
                let d = civ6opt::learn::decision_features(p, st, key, options, &eng);
                self.found = options.iter().filter(|o| *o != self.ch)
                    .map(|o| (self.model.predict(&d, key.kind, o, &civ6opt::learn::option_features(p, st, key, base.as_ref(), o, &eng)), o))
                    .max_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).map(|x| x.1.clone());
            }
            GenomeChooser { genome: self.g }.choose(p, st, key, options)
        }
    }
    let mut at = At { g, model, key, ch, found: None };
    let mut st = st0.clone();
    while at.found.is_none() && st.turn <= pl.s.horizon {
        pl.advance(&mut st, &mut at);
    }
    at.found
}

fn save_plan(out: &str, turn: i64, best: f64, g: &HashMap<Key, Choice>, extra: Vec<(&str, Value)>) {
    let mut plan: Vec<(&Key, &Choice)> = g.iter().collect();
    plan.sort_by(|x, y| format!("{:?}", x.0).cmp(&format!("{:?}", y.0)));
    let plan: Vec<Value> = plan.iter().map(|(k, c)| Value::Arr(vec![key_json(k), choice_json(c)])).collect();
    let mut kv = vec![("turn", Value::from(turn)), ("best", Value::from(best))];
    kv.extend(extra);
    kv.push(("plan", Value::Arr(plan)));
    std::fs::write(out, Value::obj(kv).to_string()).unwrap();
}

/// The science / culture floors of this run (--milestones TECH@turn,CIVIC@turn [--milestone-weight w]), calibrated on
/// the problem's recorded game for the evaluation turn.
static ALLOW_MILITARY: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static NO_HARVEST_BEFORE: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
static GOLD_OUT: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
static TEAM_LUX: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
/// --allies 1,3: the allied players (default with --team: the major players the record's trade routes went to)
static ALLIES: std::sync::OnceLock<Option<Vec<i64>>> = std::sync::OnceLock::new();
static MILESTONES: std::sync::OnceLock<Option<std::sync::Arc<civ6opt::milestones::Milestones>>> = std::sync::OnceLock::new();

fn setup_milestones(p: &Problem, args: &[String], turn: i64) {
    // --require 'DISTRICT_CAMPUS:3,military:2,TECH_CASTLES:1': what the plan must have at the evaluation turn
    // (--require-weight, 500 each; a tech or civic by the share of its path still to research); 'X:N@T' checks
    // that item at the end of turn T instead (e.g. TECH_CASTLES:1@44 with a T67 evaluation)
    let split = |y: &str| -> (String, String, Option<i64>) {
        let (a, b) = y.split_once(':').unwrap();
        match b.split_once('@') { Some((n, t)) => (a.to_string(), n.to_string(), Some(t.parse().unwrap())), None => (a.to_string(), b.to_string(), None) }
    };
    let req3: Vec<(String, String, Option<i64>)> = arg(args, "--require").map(|x| x.split(',').map(|y| split(y)).collect()).unwrap_or_default();
    let require: Vec<(String, i64)> = req3.iter().map(|(a, n, _)| (a.clone(), n.parse().unwrap())).collect();
    let require_at: Vec<Option<i64>> = req3.iter().map(|x| x.2).collect();
    let require_weight: f64 = arg(args, "--require-weight").map(|x| x.parse().unwrap()).unwrap_or(500.0);
    // --require-smooth: a missed tech or civic costs only the share of its path still to research (Milestones::smooth)
    let smooth = args.iter().any(|a| a == "--require-smooth");
    // --fallback-model trees.json: a decision whose planned choice cannot be taken takes the model's first option
    // (crate::learn::FALLBACK) instead of the base policy's, in every command
    if let Some(path) = arg(args, "--fallback-model") {
        let _ = civ6opt::learn::FALLBACK.set(civ6opt::learn::Trees::load(&path));
    }
    // --reward 'UNIT_HORSEMAN:200': points per one the plan has at the evaluation turn (as in --require; '@T' too)
    let rew3: Vec<(String, String, Option<i64>)> = arg(args, "--reward").map(|x| x.split(',').map(|y| split(y)).collect()).unwrap_or_default();
    let reward: Vec<(String, f64)> = rew3.iter().map(|(a, n, _)| (a.clone(), n.parse().unwrap())).collect();
    let reward_at: Vec<Option<i64>> = rew3.iter().map(|x| x.2).collect();
    let sc = Scenario::load(&p.v["rows"], &p.v["scenario"]);
    let speed = sym(p.v["settings"]["speed"].as_str().unwrap_or("GAMESPEED_ONLINE"));
    let paths: Vec<(String, Vec<(Sym, f64)>)> = req3.iter().filter(|(w, _, _)| w.starts_with("TECH_") || w.starts_with("CIVIC_")).map(|(w, _, at)| {
        let era = sc.game_era(at.unwrap_or(turn));
        let path = civ6opt::milestones::closure(p.rules, sym(w), w.starts_with("CIVIC_")).into_iter()
            .map(|x| (x, civ6opt::sim::research_cost(p.rules, x, speed, era))).collect();
        (w.clone(), path)
    }).collect();
    // 'GREAT_PERSON_CLASS_GENERAL>ERA_RENAISSANCE': only individuals of that era or later count
    let gp: Vec<(String, Vec<Sym>, f64)> = req3.iter().filter(|(w, _, _)| w.starts_with("GREAT_PERSON_CLASS_")).map(|(w, _, at)| {
        let t = at.unwrap_or(turn);
        let era = sc.game_era(t);
        let (cls, min_era) = match w.split_once('>') { Some((c, e)) => (c, p.rules.era_index(sym(e))), None => (w.as_str(), None) };
        let gt = p.rules.tab("GreatPersonIndividuals");
        let members: Vec<Sym> = gt.find("GreatPersonClassType", sym(cls)).iter()
            .filter(|&&i| min_era.map(|m| gt.v(i, "EraType").sym().and_then(|e| p.rules.era_index(e)).unwrap_or(0) >= m).unwrap_or(true))
            .filter_map(|&i| gt.v(i, "GreatPersonIndividualType").sym()).collect();
        // the points of the one still on offer at the check's turn (the others' recruits as recorded): its era's
        // base cost at this speed, x1.3 (to a multiple of 5) while the game is in an earlier era (Planner::gp_cost)
        let offered = sc.gp_offer(sym(cls), t, &[]);
        let ind_era = offered.and_then(|i| p.rules.lookup_sym("GreatPersonIndividuals", "GreatPersonIndividualType", i, "EraType")).unwrap_or(era);
        let mut need = p.rules.lookup_f("Eras", "EraType", ind_era, "GreatPersonBaseCost") * p.rules.game_speed_cost_multiplier(speed);
        if p.rules.era_index(era).unwrap_or(0) < p.rules.era_index(ind_era).unwrap_or(0) {
            need = (need * 1.3 / 5.0).floor() * 5.0;
        }
        eprintln!("requirement {w}: the one on offer at T{t} is {:?}, {need:.0} points", offered.map(|x| x.as_string()));
        (w.clone(), members, need.max(1.0))
    }).collect();
    let Some(spec) = arg(args, "--milestones") else {
        let m = if require.is_empty() && reward.is_empty() { None } else { Some(std::sync::Arc::new(civ6opt::milestones::Milestones { list: vec![], weight: 0.0, require, require_weight, smooth, paths, reward, gp, require_at, reward_at })) };
        let _ = MILESTONES.set(m);
        return;
    };
    let weight: f64 = arg(args, "--milestone-weight").map(|x| x.parse().unwrap()).unwrap_or(2.0);
    let mut list = vec![];
    for item in spec.split(',') {
        let (t, d) = item.split_once('@').expect("--milestones TECH_X@85,CIVIC_Y@90");
        let civic = t.starts_with("CIVIC_");
        let m = civ6opt::milestones::calibrate(p.rules, &p.v["rows"], &sc, speed, sym(t), civic, d.parse().unwrap(), turn);
        eprintln!("milestone {t} by T{d}: path {} items, cost {:.0}; growth x{:.1}, recorded boosts {:.0}", m.costs.len(),
                  m.costs.iter().map(|x| x.1).sum::<f64>(), m.growth, m.bonus);
        list.push(m);
    }
    let _ = MILESTONES.set(Some(std::sync::Arc::new(civ6opt::milestones::Milestones { list, weight, require, require_weight, smooth, paths, reward, gp, require_at, reward_at })));
}

fn genome_from(v: &Value) -> HashMap<Key, Choice> {
    v.as_array().unwrap().iter().filter_map(|e| Some((key_from(&e[0]), choice_from(&e[1])?))).collect()
}

/// The recorded player's choices under the model's rules (no syncing to the saves): the planner plays with the
/// record's chooser (crate::record) and every decision it takes is kept. `wide`: every builder action is offered
/// (as in the replay), else the search's candidates only.
fn record_genome(p: &Problem, wide: bool) -> HashMap<Key, Choice> {
    record_genome_with(p, wide, false, None)
}

/// record_genome; `tiles`: the recorded cities also get their tiles on the recorded turns (Settings.recorded_tiles);
/// `until`: the last turn played (default: the last recorded turn).
fn record_genome_with(p: &Problem, wide: bool, tiles: bool, until: Option<i64>) -> HashMap<Key, Choice> {
    struct Keep<'a, 'r> {
        inner: civ6opt::record::RecordedChooser<'r>,
        g: &'a mut HashMap<Key, Choice>,
    }
    impl civ6opt::planner::Chooser for Keep<'_, '_> {
        fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
            let c = self.inner.choose(p, st, key, options)?;
            self.g.insert(key.clone(), c.clone());
            Some(c)
        }
    }
    let (mut pl, st0) = planner(p);
    pl.s.fixed.clear();
    if wide {
        pl.s.builder_candidates = None;
    }
    if tiles {
        pl.s.recorded_tiles = Some(civ6opt::scenario::recorded_tiles_from(&p.v["settings"]["recorded_tiles"]));
    }
    let rec = std::rc::Rc::new(civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]));
    pl.tiles_rec = Some(rec.clone());
    let last = p.v["rows"].as_array().unwrap().iter().filter_map(|r| r["turn"].as_i64()).max().unwrap_or(0);
    let mut g = HashMap::new();
    let mut ch = Keep { inner: civ6opt::record::RecordedChooser::new(&rec), g: &mut g };
    opt::play(&pl, &st0, &mut ch, opt::Horizon::plain(until.unwrap_or(last)));
    g
}

/// The recorded play as a plan for the searches (--init record_search): the replay's choices (record_genome_with),
/// played under the model's rules (as a search plays plans) with the record's choices where the replay set them
/// without a decision (research and civics in the record's order, policies, governments), every choice kept.
fn record_genome_search(p: &Problem) -> HashMap<Key, Choice> {
    struct Layered<'a, 'r> {
        first: &'a HashMap<Key, Choice>,
        rec: civ6opt::record::RecordedChooser<'r>,
        g: HashMap<Key, Choice>,
    }
    impl civ6opt::planner::Chooser for Layered<'_, '_> {
        fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
            let c = self.first.get(key).filter(|c| options.contains(c)).cloned().or_else(|| self.rec.choose(p, st, key, options))?;
            self.g.insert(key.clone(), c.clone());
            Some(c)
        }
    }
    let first = record_genome_with(p, false, true, None);
    let (pl, st0) = planner(p);
    let rec = civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]);
    let last = p.v["rows"].as_array().unwrap().iter().filter_map(|r| r["turn"].as_i64()).max().unwrap_or(0);
    let mut ch = Layered { first: &first, rec: civ6opt::record::RecordedChooser::new(&rec), g: HashMap::new() };
    opt::play(&pl, &st0, &mut ch, opt::Horizon::plain(last));
    ch.g
}

/// The recorded player's own play (--genome record | record_wide) keeps the recorded citizens' tiles and the
/// settings it was read with.
fn user_play_setup(p: &Problem, pl: &mut Planner, which: Option<&str>) {
    if let Some(w) = which.filter(|w| w.starts_with("record") && *w != "recorded" && *w != "recorded_tiles" && *w != "record_search") {
        pl.s.fixed.clear();
        if w == "record_wide" {
            pl.s.builder_candidates = None;
        }
        pl.tiles_rec = Some(std::rc::Rc::new(civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"])));
        // the recorded cities' borders grow as recorded (which plot a city gets next is not decided by the rules)
        pl.s.recorded_tiles = Some(civ6opt::scenario::recorded_tiles_from(&p.v["settings"]["recorded_tiles"]));
    }
}

/// The major players the record's trade routes went to (in team games, routes go to allies).
fn infer_allies(p: &Problem, me: Option<i64>) -> Vec<i64> {
    let mut out: Vec<i64> = vec![];
    for row in p.v["rows"].as_array().unwrap() {
        for tr in row["tradeRoutes"].as_array().cloned().unwrap_or_default() {
            let (o, d) = (tr["origin"]["playerId"].as_i64(), tr["destination"]["playerId"].as_i64());
            let civ = tr["destination"]["civilization"].as_str().map(sym);
            let cs = civ.and_then(|c| p.rules.lookup_sym("Civilizations", "CivilizationType", c, "StartingCivilizationLevelType")) == Some(sym("CIVILIZATION_LEVEL_CITY_STATE"));
            if o == me && d != me && !cs {
                if let Some(d) = d.filter(|d| !out.contains(d)) {
                    out.push(d);
                }
            }
        }
    }
    eprintln!("allies (record's routes): {out:?}");
    out
}

/// Luxury types from outside the player's land on each recorded turn: the number that, added to the model's own
/// amenities, best gives each city the happiness the record shows (its production's modified / base: +8% happy, +16%
/// ecstatic, -10% displeased ...); ties to the smaller number.
fn infer_extra_luxuries(p: &Problem, pl: &Planner) -> HashMap<i64, i64> {
    use civ6opt::amenities::{amenity_scores, happiness};
    let mut out = HashMap::new();
    for row in p.v["rows"].as_array().unwrap() {
        let Some(t) = row["turn"].as_i64() else { continue };
        let Some(cs) = row["cities"].as_array().filter(|a| !a.is_empty()) else { continue };
        let ys = row["cityYields"].as_array().cloned().unwrap_or_default();
        // the recorded non-food percent of each city (by name), where its production tells it
        let want: Vec<(Sym, f64)> = cs.iter().zip(ys.iter()).filter_map(|(c, y)| {
            let b = y["base"]["YIELD_PRODUCTION"].as_f64()?;
            let m = y["modified"]["YIELD_PRODUCTION"].as_f64()?;
            (b >= 3.0).then(|| (sym(c["name"].as_str().unwrap_or("")), (m / b - 1.0) * 100.0))
        }).collect();
        if want.is_empty() {
            continue;
        }
        let st = pl.state_at(row);
        let env = pl.player_env(&st);
        let e = pl.engine(&env, &st.cities);
        let mut best = (usize::MAX, 0i64);
        for k in 0..=6 {
            let sc = amenity_scores(p.rules, &st.cities, &e, k);
            let miss = want.iter().filter(|(n, pct)| sc.iter().find(|x| x.0 == *n).map(|x| (happiness(p.rules, x.1).0 - pct).abs() > 2.0).unwrap_or(true)).count();
            if miss < best.0 {
                best = (miss, k);
            }
        }
        if best.1 > 0 {
            out.insert(t, best.1);
        }
    }
    out
}

/// --genome recorded | record | record_wide | base | <file.json> (a search result's "plan", or a bare plan)
fn genome_arg(p: &Problem, which: Option<&str>) -> HashMap<Key, Choice> {
    match which {
        // (made with the recorded tiles, as the play it is replayed in: user_play_setup)
        Some("record") => record_genome_with(p, false, true, None),
        Some("record_wide") => record_genome_with(p, true, true, None),
        Some("record_search") => record_genome_search(p),
        None | Some("recorded") => genome_from(&p.v["recorded_genome"]),
        Some("recorded_tiles") => genome_from(&p.v["recorded_genome_tiles"]),
        Some("base") => HashMap::new(),
        Some(f) => {
            let v: Value = json::parse(&std::fs::read_to_string(f).unwrap()).unwrap();
            genome_from(if v.get("plan").is_some() { &v["plan"] } else { &v })
        }
    }
}

/// civ6opt build <export prefix> <rules.sqlite> <out.json> [--player 0] [--start 2] [--horizon 60] [--builder-candidates 4]
///     [--text data/rules/text_en.sqlite] [--unit-gift 14:UNIT_SLINGER,15:UNIT_TRADER]
/// (--unit-gift: units the record shows without a city making them, given to the plan on that turn, for those the
/// record does not explain, such as units bought with gold an ally gave the same turn)
/// Writes the game data file of a recorded game (gamedata.rs): the export rows <prefix>_p<player>.jsonl, the map
/// <prefix>_map_T<start>.json and the rules database, with the recorded game replayed as plans.
fn build(args: &[String]) {
    let t0 = Instant::now();
    let num = |name: &str, default: i64| arg(args, name).map(|x| x.parse().unwrap()).unwrap_or(default);
    let (prefix, rules, out) = (&args[2], &args[3], &args[4]);
    let text = arg(args, "--text").unwrap_or("data/rules/text_en.sqlite".into());
    let mut v = civ6opt::gamedata::assemble(prefix, rules, &text, num("--player", 0), num("--start", 2), num("--horizon", 60),
        num("--builder-candidates", 4));
    if let Some(gifts) = arg(args, "--unit-gift") {
        let Value::Obj(top) = &mut v else { unreachable!() };
        let (_, Value::Obj(sc)) = top.iter_mut().find(|(k, _)| k == "scenario").unwrap() else { unreachable!() };
        let (_, Value::Obj(ug)) = sc.iter_mut().find(|(k, _)| k == "unit_gifts").unwrap() else { unreachable!() };
        for g in gifts.split(',') {
            let (t, u) = g.split_once(':').expect("--unit-gift turn:UNIT_TYPE,...");
            match ug.iter_mut().find(|(k, _)| k == t) {
                Some((_, Value::Arr(a))) => a.push(Value::from(u)),
                _ => ug.push((t.to_string(), Value::Arr(vec![Value::from(u)]))),
            }
        }
    }
    let rules_ref: &'static Rules = Box::leak(Box::new(Rules::load(&v["rules"])));
    let mut p = Problem { v: Value::Null, rules: rules_ref };
    std::mem::swap(&mut p.v, &mut v);
    // the recorded game as plans, played with every builder action offered (as the replay does)
    let as_list = |g: &HashMap<Key, Choice>| {
        let mut items: Vec<(Value, Value)> = g.iter().map(|(k, c)| (key_json(k), choice_json(c))).collect();
        items.sort_by_key(|(k, _)| k.to_string());
        Value::Arr(items.into_iter().map(|(k, c)| Value::Arr(vec![k, c])).collect())
    };
    // played to the planner's horizon, past the record's last turn if it is later (the record's chooser then
    // falls back to the planner's own choices)
    let until = Some(p.v["settings"]["horizon"].as_i64().unwrap());
    let g = record_genome_with(&p, true, false, until);
    let g_tiles = record_genome_with(&p, true, true, until);
    let mut builder: Vec<(Key, Choice)> = g.iter().filter(|(k, c)| k.kind == sym("builder") && !matches!(choice_json(c), Value::Arr(ref a) if a.first().and_then(|x| x.as_str()) == Some("wait")))
        .map(|(k, c)| (k.clone(), c.clone())).collect();
    builder.sort_by_key(|(k, _)| key_json(k).to_string());
    let (genome, genome_tiles) = (as_list(&g), as_list(&g_tiles));
    let Value::Obj(top) = &mut p.v else { unreachable!() };
    for (k, val) in top.iter_mut() {
        match k.as_str() {
            "recorded_genome" => *val = genome.clone(),
            "recorded_genome_tiles" => *val = genome_tiles.clone(),
            "settings" => if let Value::Obj(s) = val {
                if let Some((_, eb)) = s.iter_mut().find(|(k, _)| k == "extra_builder") {
                    *eb = Value::Arr(builder.iter().map(|(_, c)| choice_json(c)).collect());
                }
            },
            _ => {}
        }
    }
    if let Some(dir) = std::path::Path::new(out).parent() {
        std::fs::create_dir_all(dir).unwrap();
    }
    std::fs::write(out, p.v.to_string()).unwrap_or_else(|e| panic!("{out}: {e}"));
    eprintln!("wrote {out} in {:.1}s", t0.elapsed().as_secs_f64());
}

/// Follows a genome (base policy elsewhere) and records every decision with its turn and options.
struct Recorder<'g> {
    genome: &'g HashMap<Key, Choice>,
    log: Vec<(i64, Key, Choice, Vec<Choice>)>,
}

impl civ6opt::planner::Chooser for Recorder<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        let c = GenomeChooser { genome: self.genome }.choose(p, st, key, options)?;
        self.log.push((st.turn, key.clone(), c.clone(), options.to_vec()));
        Some(c)
    }
}

fn snapshot(st: &State) -> Value {
    Value::obj(vec![
        ("turn", Value::from(st.turn - 1)),
        ("research", Value::from(st.research.map(|x| x.as_string()))),
        ("civic", Value::from(st.civic.map(|x| x.as_string()))),
        ("civic_progress", Value::from(st.civic.and_then(|c| st.acc.civic_progress.get(&c).copied()).unwrap_or(0.0))),
        ("research_progress", Value::from(st.research.and_then(|c| st.acc.tech_progress.get(&c).copied()).unwrap_or(0.0))),
        ("gold", Value::from(st.acc.gold)),
        ("faith", Value::from(st.acc.faith)),
        ("output", Value::from(st.last_output.to_vec())),
        ("cities", Value::Arr(st.cities.iter().map(|c| Value::obj(vec![
            ("name", Value::from(c.name.as_string())),
            ("xy", Value::from(vec![c.center.0, c.center.1])),
            ("pop", Value::from(st.cacc(c.name).population)),
            ("food_store", Value::from(st.cacc(c.name).food_store)),
            ("item", Value::from(c.current.map(|x| x.as_string()))),
            ("worked", Value::Arr(c.worked.iter().map(|w| Value::from(vec![w.x, w.y])).collect())),
            ("yields", Value::from(st.city_output.iter().find(|(n, _)| *n == c.name).map(|(_, y)| y.to_vec()).unwrap_or_default())),
            ("buildings", Value::Arr(c.buildings.iter().map(|b| Value::from(b.as_string())).collect())),
            ("districts", Value::Arr(c.districts.iter().filter(|d| d.dtype != sym("DISTRICT_CITY_CENTER"))
                .map(|d| Value::from(format!("{}{}@{},{}{}", d.dtype.as_string(), if d.complete { "" } else { "*" }, d.x, d.y,
                    d.plot.wonder.map(|w| format!(":{}", w.as_string())).unwrap_or_default()))).collect())),
            ("improvements", Value::Arr(c.owned.iter().filter_map(|p| p.improvement).map(|i| Value::from(i.as_string())).collect())),
            ("owned", Value::from(c.owned.len() as i64)),
        ])).collect())),
        ("techs", Value::from(st.acc.techs.len() as i64)),
        ("civics", Value::from(st.acc.civics.len() as i64)),
        ("government", Value::from(st.player.government.map(|x| x.as_string()))),
        ("gp_points", Value::Arr(st.gp.0.iter().map(|(k, v)| Value::from(format!("{}:{:.0}", k.as_string(), v))).collect())),
        ("gp_ours", Value::Arr(st.gp_ours.iter().map(|g| Value::from(g.as_string())).collect())),
        ("policies", Value::Arr(st.player.policies.iter().map(|x| Value::from(x.as_string())).collect())),
        ("governors", Value::Arr(st.player.governors.iter().flatten().map(|g| Value::from(format!("{}:{}:{}", g.gtype.as_string(), g.city_id.unwrap_or(-1), g.promotions.len()))).collect())),
        ("traders", Value::Arr(st.traders.iter().map(|t| Value::from(format!("{}->{}", t.origin.as_string(), t.dest.map(|d| d.as_string()).unwrap_or_default()))).collect())),
        ("builders", Value::from(st.builders.len() as i64)),
        ("units_trained", Value::Arr(st.acc.units_trained.0.iter().map(|(k, v)| Value::from(format!("{}:{}", k.as_string(), v))).collect())),
    ])
}

/// Steam's folder where it is installed by default: on Windows, on macOS, and on Windows seen from WSL (Linux).
fn steam_dir() -> String {
    if cfg!(windows) {
        r"C:\Program Files (x86)\Steam".into()
    } else if cfg!(target_os = "macos") {
        format!("{}/Library/Application Support/Steam", std::env::var("HOME").expect("HOME"))
    } else {
        "/mnt/c/Program Files (x86)/Steam".into()
    }
}

/// Diagnostics for aborts (glibc heap checks): on SIGABRT, the raw call stack goes to stderr without allocating
/// (glibc backtrace), with main's address to undo the load offset (resolve with addr2line).
#[cfg(all(target_os = "linux", target_env = "gnu"))]
mod abort_trace {
    extern "C" {
        fn signal(sig: i32, handler: usize) -> usize;
        fn backtrace(buf: *mut *mut u8, size: i32) -> i32;
        fn backtrace_symbols_fd(buf: *const *mut u8, size: i32, fd: i32);
        fn write(fd: i32, buf: *const u8, n: usize) -> isize;
    }
    extern "C" fn on_abort(_: i32) {
        let mut frames = [std::ptr::null_mut::<u8>(); 64];
        unsafe {
            let msg = b"ABORT TRACE\n";
            write(2, msg.as_ptr(), msg.len());
            let n = backtrace(frames.as_mut_ptr(), 64);
            backtrace_symbols_fd(frames.as_ptr(), n, 2);
        }
    }
    pub fn install() {
        unsafe {
            signal(6, on_abort as usize);
        }
        eprintln!("main at {:p}", super::main as fn());
    }
}

fn main() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    if std::env::var_os("CIV6OPT_ABORT_TRACE").is_some() {
        abort_trace::install();
    }
    let args: Vec<String> = std::env::args().collect();
    let cmd = args.get(1).map(|s| s.as_str()).unwrap_or("");
    if cmd == "build" {
        build(&args);
        return;
    }
    // civ6opt export <save dir> <player id> <rules.sqlite>: one JSON line per save (the export rows), in file order;
    // a save that cannot be read is skipped with a message
    // civ6opt map <save> <rules.sqlite>: the map of one save as JSON
    if cmd == "export" || cmd == "map" {
        let rules = civ6opt::gamedata::open(if cmd == "export" { &args[4] } else { &args[3] });
        let dict = civ6opt::save::Dict::new(&civ6opt::save::export::rules_types(&rules));
        let names = civ6opt::save::export::Names::load(&rules);
        if cmd == "map" {
            let g = civ6opt::save::parse(&std::fs::read(&args[2]).unwrap_or_else(|e| panic!("{}: {e}", args[2])), &dict);
            println!("{}", civ6opt::save::export::map_json(&g, &names));
            return;
        }
        let pid: i64 = args[3].parse().expect("player id");
        let mut files: Vec<String> = std::fs::read_dir(&args[2]).unwrap_or_else(|e| panic!("{}: {e}", args[2])).flatten()
            .map(|e| e.file_name().to_string_lossy().into_owned()).filter(|f| f.ends_with(".Civ6Save")).collect();
        files.sort();
        for f in files {
            let raw = std::fs::read(std::path::Path::new(&args[2]).join(&f)).unwrap();
            // e.g. a save written while the game was mid-turn
            match std::panic::catch_unwind(|| civ6opt::save::parse(&raw, &dict)) {
                Ok(g) => println!("{}", civ6opt::save::export::row_json(&g, &dict, &names, &f, pid)),
                Err(e) => eprintln!("skipping {f}: {}", e.downcast_ref::<String>().cloned().or(e.downcast_ref::<&str>().map(|s| s.to_string())).unwrap_or_default()),
            }
        }
        return;
    }
    // civ6opt text <out.sqlite> [--game <the game's folder>] [--workshop <the workshop content folder 289070>]
    if cmd == "text" {
        let steam = steam_dir();
        let game = arg(&args, "--game").unwrap_or_else(|| format!("{steam}/steamapps/common/Sid Meier's Civilization VI"));
        let workshop = arg(&args, "--workshop").unwrap_or_else(|| format!("{steam}/steamapps/workshop/content/289070"));
        civ6opt::gamedata::build_text_db(&args[2], &game, &workshop);
        return;
    }
    let path = args.get(2).expect("problem file");
    let p = load(path);
    let turn: i64 = arg(&args, "--turn").map(|x| x.parse().unwrap()).unwrap_or(30);
    let ext: i64 = arg(&args, "--ext").map(|x| x.parse().unwrap()).unwrap_or(0);
    setup_milestones(&p, &args, turn);
    let _ = ALLOW_MILITARY.set(args.iter().any(|a| a == "--allow-military"));
    // --team: what allies trade (as recorded): the gold given to them (--gold-out) and luxuries from them
    let team = args.iter().any(|a| a == "--team");
    let _ = GOLD_OUT.set(team || args.iter().any(|a| a == "--gold-out"));
    let _ = TEAM_LUX.set(team);
    let _ = ALLIES.set(arg(&args, "--allies").map(|x| x.split(',').map(|y| y.parse().unwrap()).collect()));
    let _ = NO_HARVEST_BEFORE.set(arg(&args, "--no-harvest-before").map(|x| x.parse().unwrap()).unwrap_or(0));
    // --seed: varies a search's random numbers (0: as before)
    let seed: u64 = arg(&args, "--seed").map(|x| x.parse().unwrap()).unwrap_or(0);
    let ext_w: f64 = arg(&args, "--ext-weight").map(|x| x.parse().unwrap()).unwrap_or(1.0);
    let horizon = opt::Horizon { turn, ext, w: ext_w, free: None, general: args.iter().any(|a| a == "--general") };
    // --value model.json: decisions the plan does not fix take the option with the best learned value --look turns
    // (default 3) after their own (crate::value; the model from civ6opt learn)
    if let Some(f) = arg(&args, "--value") {
        let model = civ6opt::value::ValueModel::from_json(&json::parse(&std::fs::read_to_string(&f).unwrap()).unwrap());
        let look: i64 = arg(&args, "--look").map(|x| x.parse().unwrap()).unwrap_or(3);
        let _ = civ6opt::value::LOOKAHEAD.set(civ6opt::value::Lookahead { model, turns: look });
    }
    match cmd {
        "collect" => {
            // rows for crate::learn: each --plans plan (comma separated) is played once to record its decisions; then
            // --plays times a random decision of it (turn <= --turn, more than one option) is given up to --alts other
            // options, each played to the end with the plan's choices elsewhere. A row is one option: the decision's and
            // the option's features, and the score minus the plan's (0 for the plan's own choice). Tab separated to --out.
            let plans = arg(&args, "--plans").expect("--plans a.json,b.json");
            let plays: usize = arg(&args, "--plays").map(|x| x.parse().unwrap()).unwrap_or(2000);
            let alts: usize = arg(&args, "--alts").map(|x| x.parse().unwrap()).unwrap_or(3);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(8);
            let out = arg(&args, "--out").expect("--out rows.tsv");
            use civ6opt::learn;
            struct Feats<'g> {
                genome: &'g HashMap<Key, Choice>,
                log: Vec<(i64, Key, Choice, Vec<Choice>, Vec<f64>, Vec<[f64; learn::N_OPTION]>)>,
            }
            impl civ6opt::planner::Chooser for Feats<'_> {
                fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
                    let c = GenomeChooser { genome: self.genome }.choose(p, st, key, options)?;
                    if options.len() > 1 {
                        let base = p.base(st, key, options);
                        let eng = p.engine(&p.player_env(st), &st.cities);
                        let of = options.iter().map(|o| learn::option_features(p, st, key, base.as_ref(), o, &eng)).collect();
                        self.log.push((st.turn, key.clone(), c.clone(), options.to_vec(), learn::decision_features(p, st, key, options, &eng), of));
                    }
                    Some(c)
                }
            }
            let mut w = std::io::BufWriter::new(std::fs::File::create(&out).unwrap());
            use std::io::Write as _;
            writeln!(w, "plan\tdecision\tkind\tname\t{}\tchosen\tdelta\tdelta_objective\tdelta_research", learn::NUMERIC.join("\t")).unwrap();
            let t0 = Instant::now();
            for (pi, path) in plans.split(',').enumerate() {
                let g = genome_arg(&p, Some(path));
                let (mut pl, st0) = planner(&p);
                pl.s.horizon = turn + ext;
                let mut rec = Feats { genome: &g, log: vec![] };
                let (base_score, _, base_end) = opt::play(&pl, &st0, &mut rec, horizon);
                let base_parts = learn::score_parts(&pl, base_score, &base_end);
                let log: Vec<_> = rec.log.into_iter().filter(|d| d.0 <= turn).collect();
                eprintln!("{path}: {base_score:.1}, {} decisions with options", log.len());
                let rows = std::sync::Mutex::new(Vec::<String>::new());
                std::thread::scope(|sc| {
                    for wi in 0..threads {
                        let (p, log, g, rows, base_parts) = (&p, &log, &g, &rows, &base_parts);
                        sc.spawn(move || {
                            let (mut wpl, wst0) = planner(p);
                            wpl.s.horizon = turn + ext;
                            let mut rng = opt::Rng::new(5000 + 97 * pi as u64 + wi as u64 + seed);
                            let mut i = wi;
                            while i < plays {
                                let di = (rng.next_u64() % log.len() as u64) as usize;
                                let (_, key, ch, opts, df, of) = &log[di];
                                let mut idx: Vec<usize> = (0..opts.len()).filter(|&j| opts[j] != *ch).collect();
                                for j in 0..idx.len() {
                                    let k = j + (rng.next_u64() % (idx.len() - j) as u64) as usize;
                                    idx.swap(j, k);
                                }
                                idx.truncate(alts);
                                let row = |j: usize, delta: f64, parts: (f64, f64)| {
                                    let nums: Vec<String> = df.iter().chain(of[j].iter()).map(|x| format!("{x}")).collect();
                                    format!("{pi}\t{di}\t{}\t{}\t{}\t{}\t{delta}\t{}\t{}", key.kind.as_string(), learn::option_name(&opts[j]), nums.join("\t"),
                                            (opts[j] == *ch) as i64, parts.0 - base_parts.0, parts.1 - base_parts.1)
                                };
                                let mut mine = vec![row(opts.iter().position(|o| o == ch).unwrap(), 0.0, *base_parts)];
                                for j in idx {
                                    let mut k = g.clone();
                                    k.insert(key.clone(), opts[j].clone());
                                    let (s, _, end) = opt::play(&wpl, &wst0, &mut GenomeChooser { genome: &k }, horizon);
                                    mine.push(row(j, s - base_score, learn::score_parts(&wpl, s, &end)));
                                }
                                rows.lock().unwrap().extend(mine);
                                i += threads;
                            }
                        });
                    }
                });
                for r in rows.into_inner().unwrap() {
                    writeln!(w, "{r}").unwrap();
                }
                eprintln!("{path}: {plays} decisions sampled ({:.0}s)", t0.elapsed().as_secs_f64());
            }
        }
        "learn" => {
            // fits the value model (crate::value) to plays around the --init plan: each play but the first changes one
            // of its decisions (turn T) to another option at random and plays to the end, the plan's choices elsewhere;
            // the state at the start of every turn after T with the play's score is one row of that turn (the value of
            // a state when the plan carries on from it). --plays (default 2000) on --threads
            // (default 8). Every fifth play is held out first to report the fit (R2: the share of the scores' variance
            // the value explains), then the model is fitted on all and saved to --out.
            let plays: usize = arg(&args, "--plays").map(|x| x.parse().unwrap()).unwrap_or(2000);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(8);
            let ridge: f64 = arg(&args, "--ridge").map(|x| x.parse().unwrap()).unwrap_or(0.01);
            let out = arg(&args, "--out").expect("--out model.json");
            let g0 = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let mut rec = Recorder { genome: &g0, log: vec![] };
            opt::play(&pl, &st0, &mut rec, opt::Horizon::plain(turn));
            let cands: Vec<(i64, Key, Vec<Choice>)> = rec.log.iter().filter(|d| d.3.len() > 1).map(|d| (d.0, d.1.clone(), d.3.clone())).collect();
            type Row = (usize, i64, [f64; civ6opt::value::N_FEATURES], f64);
            let rows = std::sync::Mutex::new(Vec::<Row>::new());
            let t0 = Instant::now();
            std::thread::scope(|sc| {
                for w in 0..threads {
                    let (p, cands, g0, rows) = (&p, &cands, &g0, &rows);
                    sc.spawn(move || {
                        let (mut wpl, wst0) = planner(p);
                        wpl.s.horizon = turn + ext;
                        let mut rng = opt::Rng::new(1000 + w as u64 + seed);
                        let mut i = w;
                        while i < plays {
                            let mut g = g0.clone();
                            let mut after = 0;
                            if i > 0 {
                                let (t, k, o) = &cands[(rng.next_u64() % cands.len() as u64) as usize];
                                g.insert(k.clone(), o[(rng.next_u64() % o.len() as u64) as usize].clone());
                                after = *t;
                            }
                            let mut st = wst0.clone();
                            let mut feats = vec![];
                            while st.turn <= turn {
                                feats.push((st.turn, civ6opt::value::features(&wpl, &st)));
                                wpl.advance(&mut st, &mut GenomeChooser { genome: &g });
                            }
                            let score = opt::play(&wpl, &wst0, &mut GenomeChooser { genome: &g }, horizon).0;
                            rows.lock().unwrap().extend(feats.into_iter().filter(|x| x.0 > after).map(|(t, f)| (i, t, f, score)));
                            i += threads;
                        }
                    });
                }
            });
            let rows = rows.into_inner().unwrap();
            let mut by_turn: std::collections::BTreeMap<i64, Vec<&Row>> = Default::default();
            for r in &rows {
                by_turn.entry(r.1).or_default().push(r);
            }
            let mut model = civ6opt::value::ValueModel { turns: HashMap::new() };
            let mut report = vec![];
            for (t, rs) in &by_turn {
                let (train, test): (Vec<&&Row>, Vec<&&Row>) = rs.iter().partition(|r| r.0 % 5 != 4);
                let fit = civ6opt::value::ValueModel::fit_turn(&train.iter().map(|r| r.2).collect::<Vec<_>>(), &train.iter().map(|r| r.3).collect::<Vec<_>>(), ridge);
                let one = civ6opt::value::ValueModel { turns: [(*t, fit)].into_iter().collect() };
                let pred: Vec<f64> = test.iter().map(|r| {
                    let (w, m, s) = &one.turns[t];
                    w[0] + (1..civ6opt::value::N_FEATURES).map(|i| w[i] * (r.2[i] - m[i]) / s[i]).sum::<f64>()
                }).collect();
                let y: Vec<f64> = test.iter().map(|r| r.3).collect();
                let my = y.iter().sum::<f64>() / y.len().max(1) as f64;
                let sst: f64 = y.iter().map(|v| (v - my).powi(2)).sum();
                let sse: f64 = y.iter().zip(&pred).map(|(a, b)| (a - b).powi(2)).sum();
                report.push((*t, if sst > 0.0 { 1.0 - sse / sst } else { 0.0 }));
                model.turns.insert(*t, civ6opt::value::ValueModel::fit_turn(&rs.iter().map(|r| r.2).collect::<Vec<_>>(), &rs.iter().map(|r| r.3).collect::<Vec<_>>(), ridge));
            }
            std::fs::write(&out, model.to_json().to_string()).unwrap();
            let r2 = Value::Arr(report.iter().map(|(t, v)| Value::Arr(vec![Value::from(*t), Value::from((v * 1000.0).round() / 1000.0)])).collect());
            println!("{}", Value::obj(vec![("plays", Value::from(plays as i64)), ("rows", Value::from(rows.len() as i64)), ("held_out_r2", r2),
                ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
            eprintln!("saved {out}");
        }
        "run" => {
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext; // the builders' returns count to the end of the evaluation
            let g = genome_arg(&p, arg(&args, "--genome").as_deref());
            if arg(&args, "--genome").as_deref() == Some("recorded_tiles") {
                pl.s.recorded_tiles = Some(civ6opt::scenario::recorded_tiles_from(&p.v["settings"]["recorded_tiles"]));
            }
            user_play_setup(&p, &mut pl, arg(&args, "--genome").as_deref());
            let mut ch = GenomeChooser { genome: &g };
            let mut st = st0.clone();
            let mut total = 0.0;
            let t = Instant::now();
            while st.turn <= turn {
                pl.advance(&mut st, &mut ch);
                total += opt::objective(&st.last_output);
                println!("{}", snapshot(&st));
                if args.iter().any(|a| a == "--costs") {
                    // each city's current item: its cost now (locked once started) and progress
                    let env = pl.player_env(&st);
                    let rows: Vec<Value> = st.cities.iter().filter_map(|c| {
                        let it = c.current?;
                        let cost = civ6opt::sim::current_cost(p.rules, &st.acc, &st.cities, &env, c.name, it, pl.speed)?;
                        let prog = st.acc.cities.get(&c.name).and_then(|ca| ca.progress.get(&it).copied()).unwrap_or(0.0);
                        Some(Value::Arr(vec![Value::from(c.name.as_string()), Value::from(it.as_string()), Value::from(cost), Value::from(prog)]))
                    }).collect();
                    println!("{}", Value::obj(vec![("costs_turn", Value::from(st.turn - 1)), ("costs", Value::Arr(rows))]));
                }
            }
            let floor = pl.milestones.as_ref().map(|m| m.penalty(&st)).unwrap_or(0.0);
            if let Some(m) = pl.milestones.as_ref() {
                for line in m.report(&st) {
                    eprintln!("{line}");
                }
                for x in &m.list {
                    let (need, have) = x.balance(&st);
                    eprintln!("milestone {} by T{}: needs {need:.0} more, expects {have:.0} -> penalty {:.0}", x.target.as_string(), x.deadline,
                              m.weight * (need - have).max(0.0));
                }
            }
            let mut ext_total = 0.0;
            let mut xc = opt::ExtChooser;
            while st.turn <= turn + ext {
                pl.advance(&mut st, &mut xc);
                ext_total += opt::objective(&st.last_output);
                println!("{}", snapshot(&st));
            }
            if std::env::var("CIV6OPT_PROFILE").is_ok() {
                eprintln!("{}", civ6opt::prof::report());
            }
            let idle = opt::idle_units(&st0, &st);
            println!("{}", Value::obj(vec![("total", Value::from(total)), ("ext", Value::from(ext_total)), ("idle_units", Value::from(idle)),
                ("floor_penalty", Value::from(floor)),
                ("score", Value::from(total + ext_w * ext_total - opt::IDLE_UNIT_PENALTY * idle - floor)), ("seconds", Value::from(t.elapsed().as_secs_f64())),
                ("engines", Value::from(civ6opt::engine::ENGINES.load(std::sync::atomic::Ordering::Relaxed)))]));
        }
        "search" => {
            let budget: usize = arg(&args, "--budget").map(|x| x.parse().unwrap()).unwrap_or(20000);
            let runs: usize = arg(&args, "--runs").map(|x| x.parse().unwrap()).unwrap_or(12);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(12);
            let init = arg(&args, "--init").unwrap_or("recorded".into());
            let out = arg(&args, "--out").unwrap_or("data/search/rust_search.json".into());
            let rounds: usize = arg(&args, "--rounds").map(|x| x.parse().unwrap()).unwrap_or(1);
            let t0 = Instant::now();
            // rounds: every run starts from the best plan of all runs so far (the first round from --init)
            let mut init_g: Option<HashMap<Key, Choice>> = if init == "base" { None } else { Some(genome_arg(&p, Some(init.as_str()))) };
            let mut results: Vec<(u64, f64, opt::Seq, Vec<(usize, f64)>)> = vec![];
            for round in 0..rounds {
                let next = std::sync::Mutex::new(0usize);
                let collected = std::sync::Mutex::new(vec![]);
                let init_r = init_g.as_ref();
                let per = budget / rounds;
                std::thread::scope(|sc| {
                    for _ in 0..threads.min(runs) {
                        sc.spawn(|| {
                            let (mut pl, st0) = planner(&p);
                            pl.s.horizon = turn + ext; // the builders' returns count to the end of the evaluation
                            loop {
                                let seed = { let mut n = next.lock().unwrap(); if *n >= runs { break; } *n += 1; (round * runs + *n - 1) as u64 } + seed * 1_000_000;
                                let t = Instant::now();
                                let s = opt::nrpa(&pl, &st0, horizon, per, 2, 1.0, 3.0, seed, init_r, None);
                                eprintln!("round {round} run {seed}: best {:.1} after {} rollouts ({:.0}s); reuse: repeats {}, turns simulated {} skipped {}",
                                          s.best, s.n, t.elapsed().as_secs_f64(), s.reuse.stats[1], s.reuse.stats[2], s.reuse.stats[3]);
                                collected.lock().unwrap().push((seed, s.best, s.best_seq, s.curve));
                            }
                        });
                    }
                });
                results.extend(collected.into_inner().unwrap());
                let best = results.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
                eprintln!("after round {round}: best {:.1} ({:.0}s)", best.1, t0.elapsed().as_secs_f64());
                init_g = Some(opt::genome_of(&best.2));
            }
            let best = results.iter().max_by(|a, b| a.1.partial_cmp(&b.1).unwrap()).unwrap();
            println!("{}", Value::obj(vec![("runs", Value::from(results.iter().map(|r| r.1).collect::<Vec<_>>())), ("best", Value::from(best.1)),
                                           ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
            let plan: Vec<Value> = best.2.iter().map(|(k, c, _, _, _)| Value::Arr(vec![key_json(k), choice_json(c)])).collect();
            let runs_v: Vec<Value> = results.iter().map(|r| Value::obj(vec![("seed", Value::from(r.0 as i64)), ("best", Value::from(r.1)),
                ("curve", Value::Arr(r.3.iter().map(|(n, b)| Value::Arr(vec![Value::from(*n), Value::from(*b)])).collect()))])).collect();
            let doc = Value::obj(vec![("turn", Value::from(turn)), ("best", Value::from(best.1)), ("runs", Value::Arr(runs_v)), ("plan", Value::Arr(plan))]);
            std::fs::write(&out, doc.to_string()).unwrap();
            eprintln!("saved {out}");
            if std::env::var("CIV6OPT_PROFILE").is_ok() {
                eprintln!("{} engines {} lookups {}", civ6opt::prof::report(), civ6opt::engine::ENGINES.load(std::sync::atomic::Ordering::Relaxed),
                          civ6opt::engine::LOOKUPS.load(std::sync::atomic::Ordering::Relaxed));
            }
        }
        "lns" => {
            // large neighbourhood search (fn lns)
            let window: i64 = arg(&args, "--window").map(|x| x.parse().unwrap()).unwrap_or(5);
            let budget: usize = arg(&args, "--budget").map(|x| x.parse().unwrap()).unwrap_or(150);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(12);
            let passes: usize = arg(&args, "--passes").map(|x| x.parse().unwrap()).unwrap_or(2);
            let out = arg(&args, "--out").unwrap_or("data/search/rust_lns.json".into());
            let g = genome_arg(&p, arg(&args, "--init").as_deref());
            let t0 = Instant::now();
            let (best, g, hist) = lns_span(&p, g, turn, ext, horizon, window, budget, threads, passes, "", (0, turn), seed);
            save_plan(&out, turn, best, &g, vec![("history", Value::Arr(hist))]);
            println!("{}", Value::obj(vec![("best", Value::from(best)), ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
            eprintln!("saved {out}");
        }
        "travel" => {
            // turns a unit needs between two plots at --turn (state from that turn's save): --from x,y --to x,y [--unit]
            let xy = |a: &str| { let v: Vec<i32> = a.split(',').map(|x| x.parse().unwrap()).collect(); (v[0], v[1]) };
            let (a, b) = (xy(&arg(&args, "--from").unwrap()), xy(&arg(&args, "--to").unwrap()));
            let unit = arg(&args, "--unit").unwrap_or("UNIT_SETTLER".into());
            let (pl, _) = planner(&p);
            let row = p.v["rows"].as_array().unwrap().iter().find(|r| r["turn"].as_i64() == Some(turn)).unwrap();
            let st = pl.state_at(row);
            println!("{:?}", pl.travel(&st, a, b, &unit));
        }
        "ils" => {
            // iterated local search that learns from its trials. Each round, --threads kicks of the incumbent (one
            // decision changed to another of its options, every decision from a turn on dropped, or an idle unit
            // replaced), each repaired by an LNS of the turns after it (15; 30 for a site or a drop); then the improving
            // kicks are laid over the incumbent one by one, kept while the score rises. Every trial updates the value of
            // its change (times tried, mean gain over the incumbent of its round) and of its kind (tries, gains); kicks
            // are drawn by kind in proportion to the kind's success and, within a kind, by UCB on the changes' values
            // (untried first). Annealing: the incumbent may get worse by up to --temp x the score, shrinking to 0.
            let minutes: f64 = arg(&args, "--minutes").map(|x| x.parse().unwrap()).unwrap_or(10.0);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(12);
            let budget: usize = arg(&args, "--budget").map(|x| x.parse().unwrap()).unwrap_or(70);
            let temp: f64 = arg(&args, "--temp").map(|x| x.parse().unwrap()).unwrap_or(0.005);
            let out = arg(&args, "--out").unwrap_or("data/search/rust_ils.json".into());
            let mut g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let eval = |g: &HashMap<Key, Choice>| opt::play(&pl, &st0, &mut GenomeChooser { genome: g }, horizon).0;
            let mut cur = eval(&g);
            let mut best = cur;
            let mut best_g = g.clone();
            eprintln!("start {best:.1}");
            let t0 = Instant::now();
            let mut rng = opt::Rng::new(4242 ^ seed.wrapping_mul(0x9E3779B97F4A7C15));
            let mut hist: Vec<Value> = vec![];
            let mut round = 0u64;
            let kinds = ["settle", "prod", "research", "civic", "governor", "trade", "builder", "buy", "drop", "unit"];
            // the values learned: per change (tries, mean gain), per kind (tries, gains)
            let mut value: HashMap<String, (u32, f64)> = HashMap::new();
            let mut kind_stat: HashMap<&str, (u32, u32)> = HashMap::new();
            let mut kind_gain: HashMap<&str, f64> = HashMap::new();
            let idle = |c: &Choice| match c {
                Choice::S(u) => opt::idle_unit(*u),
                Choice::BuyItem(_, u, _, _) => opt::idle_unit(*u),
                _ => false,
            };
            while t0.elapsed().as_secs_f64() < minutes * 60.0 {
                round += 1;
                // the incumbent's decisions and their options
                let mut rec = Recorder { genome: &g, log: vec![] };
                opt::play(&pl, &st0, &mut rec, opt::Horizon::plain(turn));
                let log = rec.log;
                // every change on offer: (kind, id, turn, decision key and new choice, or a drop turn)
                let mut offer: HashMap<&str, Vec<(String, i64, Option<(Key, Choice)>)>> = HashMap::new();
                for d in &log {
                    let k = d.1.kind.as_string();
                    let kind = if idle(&d.2) { "unit" } else { kinds.iter().copied().find(|x| *x == k).unwrap_or("") };
                    if kind.is_empty() {
                        continue;
                    }
                    for o in &d.3 {
                        if *o == d.2 || (kind == "unit" && idle(o)) {
                            continue;
                        }
                        let id = format!("{kind} {} -> {}", key_json(&d.1), choice_json(o));
                        offer.entry(kind).or_default().push((id, d.0, Some((d.1.clone(), o.clone()))));
                    }
                }
                for t in st0.turn..=turn {
                    offer.entry("drop").or_default().push((format!("drop from T{t}"), t, None));
                }
                let total: f64 = value.values().map(|v| v.0 as f64).sum::<f64>().max(1.0);
                // upper bound of a change's gain: its mean, or untried its kind's mean, plus the exploration bonus
                let ucb = |id: &str, kind: &str, rng: &mut opt::Rng| -> f64 {
                    let jitter = (rng.next_u64() % 1000) as f64 * 1e-6;
                    let (n, m) = value.get(id).copied().unwrap_or_else(|| {
                        let kn = kind_stat.get(kind).map(|x| x.0).unwrap_or(0);
                        (1, if kn > 0 { kind_gain.get(kind).copied().unwrap_or(0.0) / kn as f64 } else { 0.0 })
                    });
                    m + 30.0 * ((total + 1.0).ln() / n as f64).sqrt() + jitter
                };
                let mut kicks: Vec<(String, &str, i64, HashMap<Key, Choice>)> = vec![];
                let mut used: std::collections::HashSet<String> = std::collections::HashSet::new();
                let present: Vec<&str> = kinds.iter().copied().filter(|k| offer.get(k).map(|v| !v.is_empty()).unwrap_or(false)).collect();
                let mut guard = 0;
                while kicks.len() < threads && guard < 1000 && !present.is_empty() {
                    guard += 1;
                    // a kind, in proportion to its success so far ((gains + 1) / (tries + 2))
                    let w: Vec<f64> = present.iter().map(|k| { let (n, s) = kind_stat.get(k).copied().unwrap_or((0, 0)); (s as f64 + 1.0) / (n as f64 + 2.0) }).collect();
                    let mut x = rng.f64() * w.iter().sum::<f64>();
                    let mut kind = present[present.len() - 1];
                    for (k, wk) in present.iter().zip(&w) {
                        if x < *wk {
                            kind = k;
                            break;
                        }
                        x -= wk;
                    }
                    // within it, the change with the best upper bound
                    let cand = &offer[kind];
                    let Some(c) = cand.iter().filter(|c| !used.contains(&c.0)).map(|c| (ucb(&c.0, kind, &mut rng), c)).max_by(|a, b| a.0.partial_cmp(&b.0).unwrap()).map(|x| x.1) else { continue };
                    used.insert(c.0.clone());
                    let k = match &c.2 {
                        Some((key, ch)) => {
                            let mut k = g.clone();
                            k.insert(key.clone(), ch.clone());
                            k
                        }
                        None => {
                            let drop: std::collections::HashSet<Key> = log.iter().filter(|d| d.0 >= c.1).map(|d| d.1.clone()).collect();
                            g.iter().filter(|(k, _)| !drop.contains(*k)).map(|(k, c)| (k.clone(), c.clone())).collect()
                        }
                    };
                    kicks.push((c.0.clone(), kind, c.1, k));
                }
                let found = std::sync::Mutex::new(vec![]);
                std::thread::scope(|sc| {
                    for (i, (id, kind, t, k)) in kicks.into_iter().enumerate() {
                        let (p, found) = (&p, &found);
                        sc.spawn(move || {
                            let big = kind == "settle" || kind == "drop";
                            let (w, span) = if big { (10, (t, t + 29)) } else { (5, (t, t + 14)) };
                            let (b, gk, _) = lns_span(p, k, turn, ext, horizon, w, budget, 1, 1, "-", span, round * 100 + i as u64);
                            found.lock().unwrap().push((b, gk, id, kind));
                        });
                    }
                });
                let mut found = found.into_inner().unwrap();
                found.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.2.cmp(&b.2)));
                // learn from every trial
                for (b, _, id, kind) in &found {
                    let gain = b - cur;
                    let e = value.entry(id.clone()).or_insert((0, 0.0));
                    e.1 = (e.1 * e.0 as f64 + gain) / (e.0 + 1) as f64;
                    e.0 += 1;
                    let ks = kind_stat.entry(kind).or_insert((0, 0));
                    ks.0 += 1;
                    *kind_gain.entry(kind).or_insert(0.0) += gain;
                    if gain > 1e-9 {
                        ks.1 += 1;
                    }
                }
                // the improving kicks laid over one another (each one's changes from the incumbent), kept while better
                let prev = cur;
                let base_g = g.clone();
                let improving: Vec<&(f64, HashMap<Key, Choice>, String, &str)> = found.iter().filter(|f| f.0 > cur + 1e-9).collect();
                let mut merged = 0;
                if let Some(first) = improving.first() {
                    g = first.1.clone();
                    cur = first.0;
                    merged = 1;
                    for f in improving.iter().skip(1) {
                        let mut cand = g.clone();
                        for (k, c) in f.1.iter() {
                            if base_g.get(k) != Some(c) {
                                cand.insert(k.clone(), c.clone());
                            }
                        }
                        let sc = eval(&cand);
                        if sc > cur + 1e-9 {
                            cur = sc;
                            g = cand;
                            merged += 1;
                        }
                    }
                } else if let Some(top) = found.first() {
                    // annealing
                    let allow = temp * best * (1.0 - t0.elapsed().as_secs_f64() / (minutes * 60.0)).max(0.0);
                    if top.0 >= cur - allow {
                        cur = top.0;
                        g = top.1.clone();
                    }
                }
                if cur > best + 1e-9 {
                    best = cur;
                    best_g = g.clone();
                }
                let rate: Vec<String> = kinds.iter().filter_map(|k| kind_stat.get(k).map(|(n, s)| format!("{k} {s}/{n}"))).collect();
                eprintln!("round {round}: {} kicks, {} improving, {merged} merged: incumbent {prev:.1} -> {cur:.1}, best {best:.1} ({:.0}s) | {}",
                          found.len(), improving.len(), t0.elapsed().as_secs_f64(), rate.join(", "));
                if let Some(top) = found.first() {
                    eprintln!("   top: {:+.1} {}", top.0 - prev, top.2);
                }
                hist.push(Value::Arr(vec![Value::from(round as i64), Value::from(improving.len() as i64), Value::from(merged as i64), Value::from(cur), Value::from(best)]));
                save_plan(&out, turn, best, &best_g, vec![("history", Value::Arr(hist.clone()))]);
            }
            // the changes learned to be best
            let mut vs: Vec<(&String, &(u32, f64))> = value.iter().collect();
            vs.sort_by(|a, b| b.1 .1.partial_cmp(&a.1 .1).unwrap());
            for (id, (n, m)) in vs.iter().take(10) {
                eprintln!("value {m:+.1} x{n} {id}");
            }
            save_plan(&out, turn, best, &best_g, vec![("history", Value::Arr(hist))]);
            println!("{}", Value::obj(vec![("best", Value::from(best)), ("rounds", Value::from(round as i64)), ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
            eprintln!("saved {out}");
        }
        "ma" => {
            // memetic search: a population of plans (--pop, from --init and the base policy, made diverse by a few
            // random changes); each generation every slot breeds a child: two parents by tournament, the first's
            // decisions before a turn and the second's from it on (one point crossover by turn), one random change,
            // and an LNS of the 15 turns from the crossover point; the --pop best of parents and children (distinct
            // scores) go on. --minutes in all; the best plan is saved every generation.
            let minutes: f64 = arg(&args, "--minutes").map(|x| x.parse().unwrap()).unwrap_or(10.0);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(10);
            let budget: usize = arg(&args, "--budget").map(|x| x.parse().unwrap()).unwrap_or(70);
            let popn: usize = arg(&args, "--pop").map(|x| x.parse().unwrap()).unwrap_or(threads);
            // --stall M: stop once the best has not risen by more than 0.1% in the last M minutes (0: off)
            let stall: f64 = arg(&args, "--stall").map(|x| x.parse().unwrap()).unwrap_or(0.0);
            let mut best_at: Vec<(f64, f64)> = vec![];
            let out = arg(&args, "--out").unwrap_or("data/search/rust_ma.json".into());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            // an individual: (score, plan, its decisions by turn with their options)
            type Log = Vec<(i64, Key, Choice, Vec<Choice>)>;
            let describe = |g: &HashMap<Key, Choice>| -> (f64, Log) {
                let mut rec = Recorder { genome: g, log: vec![] };
                let sc = opt::play(&pl, &st0, &mut rec, horizon).0;
                (sc, rec.log.into_iter().filter(|d| d.0 <= turn).collect())
            };
            let mutate = |g: &HashMap<Key, Choice>, log: &Log, rng: &mut opt::Rng| -> (HashMap<Key, Choice>, i64) {
                let mut k = g.clone();
                if rng.next_u64() % 8 == 0 {
                    let t = st0.turn + (rng.next_u64() % (turn - st0.turn + 1) as u64) as i64;
                    let drop: std::collections::HashSet<&Key> = log.iter().filter(|d| d.0 >= t).map(|d| &d.1).collect();
                    k.retain(|key, _| !drop.contains(key));
                    return (k, t);
                }
                let cand: Vec<&(i64, Key, Choice, Vec<Choice>)> = log.iter().filter(|d| d.3.len() > 1).collect();
                if cand.is_empty() {
                    return (k, st0.turn);
                }
                let d = cand[(rng.next_u64() % cand.len() as u64) as usize];
                let alts: Vec<&Choice> = d.3.iter().filter(|o| **o != d.2).collect();
                k.insert(d.1.clone(), alts[(rng.next_u64() % alts.len() as u64) as usize].clone());
                (k, d.0)
            };
            let t0 = Instant::now();
            let mut rng = opt::Rng::new(777 ^ seed.wrapping_mul(0x9E3779B97F4A7C15));
            let inits = [genome_arg(&p, arg(&args, "--init").as_deref()), HashMap::new()];
            let mut pop: Vec<(f64, HashMap<Key, Choice>, Log)> = vec![];
            for i in 0..popn {
                let mut g = inits[i % 2].clone();
                if i >= 2 {
                    for _ in 0..3 {
                        let (_, log) = describe(&g);
                        g = mutate(&g, &log, &mut rng).0;
                    }
                }
                let (sc, log) = describe(&g);
                pop.push((sc, g, log));
            }
            pop.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
            eprintln!("start: {}", pop.iter().map(|x| format!("{:.0}", x.0)).collect::<Vec<_>>().join(" "));
            let mut gen = 0u64;
            let mut hist: Vec<Value> = vec![];
            while t0.elapsed().as_secs_f64() < minutes * 60.0 {
                gen += 1;
                // children: (plan, repair from turn)
                let mut kids: Vec<(HashMap<Key, Choice>, i64, String)> = vec![];
                for _ in 0..threads {
                    let tour = |rng: &mut opt::Rng| -> usize {
                        let a = (rng.next_u64() % pop.len() as u64) as usize;
                        let b = (rng.next_u64() % pop.len() as u64) as usize;
                        if pop[a].0 >= pop[b].0 { a } else { b }
                    };
                    let (a, b) = (tour(&mut rng), tour(&mut rng));
                    let c = st0.turn + 1 + (rng.next_u64() % (turn - st0.turn) as u64) as i64;
                    let mut child: HashMap<Key, Choice> = HashMap::new();
                    for d in &pop[a].2 {
                        if d.0 < c {
                            child.insert(d.1.clone(), d.2.clone());
                        }
                    }
                    for d in &pop[b].2 {
                        if d.0 >= c {
                            child.insert(d.1.clone(), d.2.clone());
                        }
                    }
                    let (_, clog) = describe(&child);
                    let (child, mt) = mutate(&child, &clog, &mut rng);
                    kids.push((child, c.min(mt), format!("{a}x{b}@T{c}")));
                }
                let found = std::sync::Mutex::new(vec![]);
                std::thread::scope(|sc| {
                    for (i, (k, t, what)) in kids.into_iter().enumerate() {
                        let (p, found) = (&p, &found);
                        sc.spawn(move || {
                            let (b, gk, _) = lns_span(p, k, turn, ext, horizon, 5, budget, 1, 1, "-", (t, t + 14), gen * 100 + i as u64);
                            found.lock().unwrap().push((b, gk, what));
                        });
                    }
                });
                let mut found = found.into_inner().unwrap();
                found.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap().then(a.2.cmp(&b.2)));
                let kid_best = found.first().map(|f| f.0).unwrap_or(f64::MIN);
                for (b, gk, _) in found {
                    if pop.iter().any(|x| (x.0 - b).abs() < 1e-6) {
                        continue;
                    }
                    let (_, log) = describe(&gk);
                    pop.push((b, gk, log));
                }
                pop.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap());
                pop.truncate(popn);
                eprintln!("gen {gen}: best child {kid_best:.1}, population {} ({:.0}s)", pop.iter().map(|x| format!("{:.0}", x.0)).collect::<Vec<_>>().join(" "), t0.elapsed().as_secs_f64());
                hist.push(Value::Arr(vec![Value::from(gen as i64), Value::from(kid_best), Value::from(pop[0].0)]));
                save_plan(&out, turn, pop[0].0, &pop[0].1, vec![("history", Value::Arr(hist.clone()))]);
                let now = t0.elapsed().as_secs_f64();
                best_at.push((now, pop[0].0));
                if stall > 0.0 && now > stall * 60.0 {
                    let then = best_at.iter().rev().find(|x| x.0 <= now - stall * 60.0).map(|x| x.1).unwrap_or(f64::MIN);
                    if pop[0].0 <= then * 1.001 {
                        eprintln!("stalled: best {:.1} was {:.1} {stall} minutes ago", pop[0].0, then);
                        break;
                    }
                }
            }
            println!("{}", Value::obj(vec![("best", Value::from(pop[0].0)), ("generations", Value::from(gen as i64)), ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
            eprintln!("saved {out}");
        }
        "rollout" => {
            // rollout policy improvement of the --init plan (Rollout::improve); the best plan is saved at every improvement
            let minutes: f64 = arg(&args, "--minutes").map(|x| x.parse().unwrap()).unwrap_or(60.0);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(10);
            let passes: usize = arg(&args, "--passes").map(|x| x.parse().unwrap()).unwrap_or(20);
            let out = arg(&args, "--out").unwrap_or("data/search/rust_rollout.json".into());
            // --span a,b: only the decisions of turns a..=b are tried (a local repair)
            let span: (i64, i64) = arg(&args, "--span").map(|x| { let v: Vec<i64> = x.split(',').map(|y| y.parse().unwrap()).collect(); (v[0], v[1]) }).unwrap_or((0, i64::MAX));
            let quiet = args.iter().any(|a| a == "--quiet");
            // --freeze '[key, ...]': decisions left as the plan has them
            let frozen: Vec<Key> = arg(&args, "--freeze").map(|x| json::parse(&x).unwrap().as_array().unwrap().iter().map(key_from).collect()).unwrap_or_default();
            // --keep 'DISTRICT_CAMPUS,DISTRICT_ENCAMPMENT,military': a decision whose choice is one of these (military: a unit
            // that does nothing for the objective) is left as it is
            let keep: Vec<String> = arg(&args, "--keep").map(|x| x.split(',').map(|y| y.to_string()).collect()).unwrap_or_default();
            let kept = |c: &Choice| match c.sym() {
                Some(x) => keep.iter().any(|k| *k == x.as_string() || (k == "military" && opt::idle_unit(x))),
                None => false,
            };
            let mut g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let t0 = Instant::now();
            std::thread::scope(|sc| {
                let (job_tx, res_rx) = rollout_workers(sc, &p, threads, turn, ext, horizon);
                let mut ro = Rollout { pl: &pl, st0: &st0, turn, horizon, job_tx, res_rx, tried: 0, learning: Learning::from_args(&args) };
                let mut hist: Vec<Value> = vec![];
                eprintln!("start {:.1}", ro.describe(&g).0);
                let deadline = t0 + std::time::Duration::from_secs_f64(minutes * 60.0);
                let best = ro.improve(&mut g, span, &frozen, &kept, passes, deadline, quiet, &mut |pass, t, best, g| {
                    hist.push(Value::Arr(vec![Value::from(pass as i64), Value::from(t), Value::from(best), Value::from(t0.elapsed().as_secs_f64())]));
                    save_plan(&out, turn, best, g, vec![("history", Value::Arr(hist.clone()))]);
                });
                let tried = ro.tried;
                drop(ro);
                save_plan(&out, turn, best, &g, vec![("history", Value::Arr(hist))]);
                println!("{}", Value::obj(vec![("best", Value::from(best)), ("tried", Value::from(tried as i64)), ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
                eprintln!("saved {out}");
            });
        }
        "irollout" => {
            // iterated rollout: a kick (one decision of the incumbent changed to another option, at random), then a
            // rollout repair of the decisions of the --window turns from the kick on (the kicked decision kept); the
            // result replaces the incumbent if better. Single changes alone (the rollout) cannot leave a plan whose
            // later decisions are fitted to it; the repair refits them before the change is judged.
            // --kick-to T: kicks only in turns <= T (default: the evaluation turn); saved at every improvement.
            let minutes: f64 = arg(&args, "--minutes").map(|x| x.parse().unwrap()).unwrap_or(60.0);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(10);
            let window: i64 = arg(&args, "--window").map(|x| x.parse().unwrap()).unwrap_or(10);
            let rpasses: usize = arg(&args, "--passes").map(|x| x.parse().unwrap()).unwrap_or(1);
            let kick_to: i64 = arg(&args, "--kick-to").map(|x| x.parse().unwrap()).unwrap_or(turn);
            // --freeze '[keys]': never kicked nor changed by the repair (as in rollout)
            let frozen: Vec<Key> = arg(&args, "--freeze").map(|x| json::parse(&x).unwrap().as_array().unwrap().iter().map(key_from).collect()).unwrap_or_default();
            let out = arg(&args, "--out").unwrap_or("data/search/rust_irollout.json".into());
            let mut g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let t0 = Instant::now();
            let deadline = t0 + std::time::Duration::from_secs_f64(minutes * 60.0);
            let mut rng = opt::Rng::new(91 + seed);
            std::thread::scope(|sc| {
                let (job_tx, res_rx) = rollout_workers(sc, &p, threads, turn, ext, horizon);
                let mut ro = Rollout { pl: &pl, st0: &st0, turn, horizon, job_tx, res_rx, tried: 0, learning: Learning::from_args(&args) };
                let (mut best, mut log, _) = ro.describe(&g);
                eprintln!("start {best:.1}, {} decisions", log.len());
                let mut hist: Vec<Value> = vec![];
                let (mut kicks, mut accepted) = (0usize, 0usize);
                while Instant::now() < deadline {
                    let cands: Vec<usize> = (0..log.len()).filter(|&i| log[i].0 <= kick_to && !frozen.contains(&log[i].1) && log[i].3.iter().any(|o| *o != log[i].2)).collect();
                    if cands.is_empty() {
                        break;
                    }
                    let d = log[cands[(rng.next_u64() % cands.len() as u64) as usize]].clone();
                    let (t, key, ch) = (d.0, d.1.clone(), d.2.clone());
                    // with a model, half the kicks take its best option (Learning::alternatives orders them), half any
                    let alts: Vec<Choice> = if ro.learning.model.is_some() && rng.next_u64() % 2 == 0 {
                        ro.learning.alternatives(&d).into_iter().take(1).collect()
                    } else {
                        d.3.iter().filter(|o| **o != ch).cloned().collect()
                    };
                    let a = alts[(rng.next_u64() % alts.len() as u64) as usize].clone();
                    let mut cand = g.clone();
                    cand.insert(key.clone(), a.clone());
                    let mut keep = frozen.clone();
                    keep.push(key.clone());
                    let s = ro.improve(&mut cand, (t, t + window - 1), &keep, &|_| false, rpasses, deadline, true, &mut |_, _, _, _| {});
                    kicks += 1;
                    let ok = s > best + 1e-9;
                    eprintln!("kick {kicks} T{t} {} {} -> {}: {s:.1} vs {best:.1}{} ({:.0}s, {} tried)", key.kind.as_string(), choice_json(&ch), choice_json(&a),
                              if ok { " accepted" } else { "" }, t0.elapsed().as_secs_f64(), ro.tried);
                    if ok && Instant::now() < deadline {
                        accepted += 1;
                        g = cand;
                        let d = ro.describe(&g);
                        best = d.0;
                        log = d.1;
                        hist.push(Value::Arr(vec![Value::from(kicks as i64), Value::from(t), Value::from(best), Value::from(t0.elapsed().as_secs_f64())]));
                        save_plan(&out, turn, best, &g, vec![("history", Value::Arr(hist.clone()))]);
                    }
                }
                save_plan(&out, turn, best, &g, vec![("history", Value::Arr(hist))]);
                println!("{}", Value::obj(vec![("best", Value::from(best)), ("kicks", Value::from(kicks as i64)), ("accepted", Value::from(accepted as i64)),
                    ("tried", Value::from(ro.tried as i64)), ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
                eprintln!("saved {out}");
            });
        }
        "policy_from" => {
            // the plan --init played as it is to each of the turns --from (comma separated), then by the option model
            // (--model) to the end: how good the model is at finishing the plan's game, against the plan's own finish
            let model = civ6opt::learn::Trees::load(&arg(&args, "--model").expect("--model trees.json"));
            // --span N: the model plays N turns from each start, the plan the rest (default: to the end)
            let span: i64 = arg(&args, "--span").map(|x| x.parse().unwrap()).unwrap_or(i64::MAX / 2);
            // --research trees.json [--slack N]: options the research model predicts to lose more than N research are left out
            let research = arg(&args, "--research").map(|x| civ6opt::learn::Trees::load(&x));
            let slack: f64 = arg(&args, "--slack").map(|x| x.parse().unwrap()).unwrap_or(0.0);
            let g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            println!("plan {:.1}", opt::play(&pl, &st0, &mut GenomeChooser { genome: &g }, horizon).0);
            for from in arg(&args, "--from").expect("--from 0,20,40").split(',').map(|x| x.parse::<i64>().unwrap()) {
                let mut ch = civ6opt::learn::PolicyChooser { genome: &g, model: &model, research: research.as_ref(), slack, from, to: from + span, taken: HashMap::new() };
                let s = opt::play(&pl, &st0, &mut ch, horizon).0;
                println!("model from T{} {s:.1}", from + 1);
                // --out dir: each game's choices as a plan, for a look at what the model chose
                if let Some(dir) = arg(&args, "--out") {
                    save_plan(&format!("{dir}/from_T{}.json", from + 1), turn, s, &ch.taken, vec![]);
                }
            }
        }
        "diffplay" => {
            // where the loss of a single change comes from: --samples random changes of decisions of --kind (default
            // prod) in the plan --init, each played beside the plan: the score and the objective change, the decisions
            // the plan's choice could not be taken in (not among the options: the base policy chose), and the cities
            // whose summed food and production changed most
            let samples: usize = arg(&args, "--samples").map(|x| x.parse().unwrap()).unwrap_or(10);
            // --kind all: any kind of decision
            let kind = arg(&args, "--kind").unwrap_or("prod".into());
            let g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            struct Watch<'a> { g: &'a HashMap<Key, Choice>, missed: Vec<(i64, Key, Choice, Choice)> }
            impl civ6opt::planner::Chooser for Watch<'_> {
                fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
                    let c = GenomeChooser { genome: self.g }.choose(p, st, key, options)?;
                    if let Some(want) = self.g.get(key) {
                        if *want != c {
                            self.missed.push((st.turn, key.clone(), want.clone(), c.clone()));
                        }
                    }
                    Some(c)
                }
            }
            let city_fp = |gen: &HashMap<Key, Choice>, w: &mut Watch| -> (f64, f64, HashMap<Sym, f64>) {
                let mut st = st0.clone();
                let mut by_city: HashMap<Sym, f64> = HashMap::new();
                while st.turn <= turn {
                    pl.advance(&mut st, w);
                    for (c, y) in &st.city_output {
                        *by_city.entry(*c).or_default() += y[0] + y[1];
                    }
                }
                let (s, total, _) = opt::play(&pl, &st0, &mut GenomeChooser { genome: gen }, horizon);
                (s, total, by_city)
            };
            let mut w0 = Watch { g: &g, missed: vec![] };
            let (s0, t0, c0) = city_fp(&g, &mut w0);
            println!("plan {s0:.1}, objective {t0:.1}, plan choices not taken {}", w0.missed.len());
            let mut rec = Recorder { genome: &g, log: vec![] };
            opt::play(&pl, &st0, &mut rec, horizon);
            let cands: Vec<_> = rec.log.iter().filter(|d| d.0 <= turn && d.3.len() > 1 && (kind == "all" || d.1.kind.as_string() == kind)).collect();
            let mut rng = opt::Rng::new(11 + seed);
            for _ in 0..samples {
                let (t, key, ch, opts) = cands[(rng.next_u64() % cands.len() as u64) as usize];
                let alts: Vec<&Choice> = opts.iter().filter(|o| *o != ch).collect();
                let a = alts[(rng.next_u64() % alts.len() as u64) as usize];
                let mut k = g.clone();
                k.insert(key.clone(), a.clone());
                let mut w = Watch { g: &k, missed: vec![] };
                let (s, total, c) = city_fp(&k, &mut w);
                let mut cities: Vec<(Sym, f64)> = c0.keys().chain(c.keys()).collect::<std::collections::BTreeSet<_>>().into_iter()
                    .map(|n| (*n, c.get(n).copied().unwrap_or(0.0) - c0.get(n).copied().unwrap_or(0.0))).collect();
                cities.sort_by(|x, y| x.1.abs().partial_cmp(&y.1.abs()).unwrap().reverse());
                let first: Vec<String> = w.missed.iter().filter(|m| m.1 != *key).take(4)
                    .map(|m| format!("T{} {}:{} {} -> {}", m.0, m.1.kind.as_string(), key_json(&m.1), choice_json(&m.2), choice_json(&m.3))).collect();
                println!("T{t} {} {} -> {}: score {:+.0}, objective {:+.0}, not taken {} (plan {}); cities {}",
                         key_json(key), choice_json(ch), choice_json(a), s - s0, total - t0, w.missed.len(), w0.missed.len(),
                         cities.iter().take(3).map(|(n, d)| format!("{} {d:+.0}", n.as_string())).collect::<Vec<_>>().join(", "));
                for f in first {
                    println!("    {f}");
                }
            }
        }
        "keycheck" => {
            // how much of what one changed decision does comes from the plan's later choices shifting: --samples
            // random single changes of the plan --init, each played with the plan's other choices looked up as now
            // (by the decision's count: the n-th production of a city) and by turn (the j-th decision of its kind
            // and entity in that turn, as the plan made them; one the plan has not: the base policy)
            let samples: usize = arg(&args, "--samples").map(|x| x.parse().unwrap()).unwrap_or(200);
            let g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let mut rec = Recorder { genome: &g, log: vec![] };
            let base_score = opt::play(&pl, &st0, &mut rec, horizon).0;
            let mut by_turn: HashMap<(Sym, Ent, i64, usize), Choice> = HashMap::new();
            for (t, k, c, _) in &rec.log {
                let j = by_turn.keys().filter(|x| x.0 == k.kind && x.1 == k.ent && x.2 == *t).count();
                by_turn.insert((k.kind, k.ent.clone(), *t, j), c.clone());
            }
            struct ByTurn<'a> { plan: &'a HashMap<(Sym, Ent, i64, usize), Choice>, changed: &'a Key, to: &'a Choice, seen: HashMap<(Sym, Ent, i64), usize> }
            impl civ6opt::planner::Chooser for ByTurn<'_> {
                fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
                    let j = self.seen.entry((key.kind, key.ent.clone(), st.turn)).or_insert(0);
                    let k = (key.kind, key.ent.clone(), st.turn, *j);
                    *j += 1;
                    if key == self.changed && options.contains(self.to) {
                        return Some(self.to.clone());
                    }
                    match self.plan.get(&k) {
                        Some(c) if options.contains(c) => Some(c.clone()),
                        _ => p.base(st, key, options),
                    }
                }
            }
            {
                let none = Key { kind: sym("none"), ent: Ent::I(-1), n: -1 };
                let s = opt::play(&pl, &st0, &mut ByTurn { plan: &by_turn, changed: &none, to: &Choice::Nothing, seen: HashMap::new() }, horizon).0;
                println!("plan {base_score:.1}, replayed by turn {s:.1}");
            }
            let cands: Vec<_> = rec.log.iter().filter(|d| d.0 <= turn && d.3.len() > 1).collect();
            let mut rng = opt::Rng::new(5 + seed);
            let mut stats: std::collections::BTreeMap<String, [usize; 3]> = Default::default();
            for _ in 0..samples {
                let (_, key, ch, opts) = cands[(rng.next_u64() % cands.len() as u64) as usize];
                let alts: Vec<&Choice> = opts.iter().filter(|o| *o != ch).collect();
                let a = alts[(rng.next_u64() % alts.len() as u64) as usize];
                let mut k = g.clone();
                k.insert(key.clone(), a.clone());
                let by_count = opt::play(&pl, &st0, &mut GenomeChooser { genome: &k }, horizon).0;
                let by_t = opt::play(&pl, &st0, &mut ByTurn { plan: &by_turn, changed: key, to: a, seen: HashMap::new() }, horizon).0;
                let e = stats.entry(key.kind.as_string()).or_default();
                e[0] += 1;
                e[1] += (by_count < base_score - 1000.0) as usize;
                e[2] += (by_t < base_score - 1000.0) as usize;
            }
            println!("kind | changes | score down by more than 1000: by count | by turn");
            for (k, e) in stats {
                println!("{k} | {} | {} | {}", e[0], e[1], e[2]);
            }
        }
        "psearch" => {
            // policy search with the option model (crate::learn, --model): a candidate changes one decision of the
            // incumbent (turn <= --kick-to; half the time to the model's first option, else to any) and plays the
            // decisions of later turns by the model (PolicyChooser), so the plan after the change is fitted to it;
            // each round plays --threads candidates side by side and the best, if better, becomes the incumbent (every
            // choice of its game). Saves at every improvement.
            let minutes: f64 = arg(&args, "--minutes").map(|x| x.parse().unwrap()).unwrap_or(30.0);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(8);
            let kick_to: i64 = arg(&args, "--kick-to").map(|x| x.parse().unwrap()).unwrap_or(turn);
            let out = arg(&args, "--out").expect("--out plan.json");
            let model = civ6opt::learn::Trees::load(&arg(&args, "--model").expect("--model trees.json"));
            // --span N: the model plays the N turns after the changed decision's, the plan the rest (default: to the end)
            let span: i64 = arg(&args, "--span").map(|x| x.parse().unwrap()).unwrap_or(i64::MAX / 2);
            // --research trees.json [--slack N]: options the research model predicts to lose more than N research are left out
            let research = arg(&args, "--research").map(|x| civ6opt::learn::Trees::load(&x));
            let slack: f64 = arg(&args, "--slack").map(|x| x.parse().unwrap()).unwrap_or(0.0);
            let mut g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let describe = |g: &HashMap<Key, Choice>| {
                let mut rec = Recorder { genome: g, log: vec![] };
                let s = opt::play(&pl, &st0, &mut rec, horizon).0;
                (s, rec.log.into_iter().filter(|d| d.0 <= kick_to && d.3.len() > 1).collect::<Vec<_>>())
            };
            let (mut best, mut log) = describe(&g);
            eprintln!("start {best:.1}, {} decisions", log.len());
            let t0 = Instant::now();
            let deadline = t0 + std::time::Duration::from_secs_f64(minutes * 60.0);
            let mut rng = opt::Rng::new(77 + seed);
            let (mut rounds, mut played) = (0usize, 0usize);
            let mut hist: Vec<Value> = vec![];
            type Job = (usize, i64, HashMap<Key, Choice>);
            let (job_tx, job_rx) = std::sync::mpsc::channel::<Job>();
            let job_rx = std::sync::Arc::new(std::sync::Mutex::new(job_rx));
            let (res_tx, res_rx) = std::sync::mpsc::channel::<(usize, f64, HashMap<Key, Choice>)>();
            std::thread::scope(|sc| {
                for _ in 0..threads {
                    let (job_rx, res_tx, p, model, research) = (job_rx.clone(), res_tx.clone(), &p, &model, &research);
                    sc.spawn(move || {
                        let (mut wpl, wst0) = planner(p);
                        wpl.s.horizon = turn + ext;
                        loop {
                            let job = { job_rx.lock().unwrap().recv() };
                            let Ok((id, from, cand)) = job else { break };
                            let mut ch = civ6opt::learn::PolicyChooser { genome: &cand, model, research: research.as_ref(), slack, from, to: from + span, taken: HashMap::new() };
                            let s = opt::play(&wpl, &wst0, &mut ch, horizon).0;
                            if res_tx.send((id, s, ch.taken)).is_err() {
                                break;
                            }
                        }
                    });
                }
                while Instant::now() < deadline {
                    let mut descr = vec![];
                    for id in 0..threads {
                        let (t, key, ch, opts) = log[(rng.next_u64() % log.len() as u64) as usize].clone();
                        let a = if rng.next_u64() % 2 == 0 {
                            // the model's first option other than the plan's choice, at the decision's state
                            model_first(&pl, &st0, &g, &model, &key, &ch).unwrap_or_else(|| opts.iter().find(|o| **o != ch).unwrap().clone())
                        } else {
                            let alts: Vec<&Choice> = opts.iter().filter(|o| **o != ch).collect();
                            alts[(rng.next_u64() % alts.len() as u64) as usize].clone()
                        };
                        let mut cand = g.clone();
                        cand.insert(key.clone(), a.clone());
                        job_tx.send((id, t, cand)).unwrap();
                        descr.push((t, key, ch, a));
                    }
                    let mut res: Vec<(usize, f64, HashMap<Key, Choice>)> = (0..threads).map(|_| res_rx.recv().unwrap()).collect();
                    rounds += 1;
                    played += threads;
                    res.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
                    let (id, s, taken) = res.swap_remove(0);
                    if s > best + 1e-9 {
                        let (t, key, ch, a) = &descr[id];
                        let (replayed, nlog) = describe(&taken);
                        if (replayed - s).abs() > 1e-6 {
                            eprintln!("warning: the kept plan replays to {replayed:.3}, its play {s:.3}");
                        }
                        eprintln!("round {rounds} T{t} {} {} -> {}: {best:.1} -> {replayed:.1} ({:.0}s, {played} played)", key.kind.as_string(),
                                  choice_json(ch), choice_json(a), t0.elapsed().as_secs_f64());
                        g = taken;
                        best = replayed;
                        log = nlog;
                        hist.push(Value::Arr(vec![Value::from(rounds as i64), Value::from(*t), Value::from(best), Value::from(t0.elapsed().as_secs_f64())]));
                        save_plan(&out, turn, best, &g, vec![("history", Value::Arr(hist.clone()))]);
                    }
                }
                drop(job_tx);
            });
            save_plan(&out, turn, best, &g, vec![("history", Value::Arr(hist))]);
            println!("{}", Value::obj(vec![("best", Value::from(best)), ("rounds", Value::from(rounds as i64)), ("played", Value::from(played as i64)),
                ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
        }
        "variants" => {
            // strategy variants of a plan: each decision of --kind (default settle) changed to each other option, the rest
            // played as the plan; writes every variant's plan to --dir and prints (index, turn, key, option, score)
            let kind = sym(&arg(&args, "--kind").unwrap_or("settle".into()));
            let dir = arg(&args, "--dir").unwrap_or("data/search/variants".into());
            std::fs::create_dir_all(&dir).unwrap();
            let g = genome_arg(&p, arg(&args, "--init").as_deref());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let mut rec = Recorder { genome: &g, log: vec![] };
            let base_score = opt::play(&pl, &st0, &mut rec, horizon).0;
            println!("{}", Value::obj(vec![("index", Value::from(-1)), ("score", Value::from(base_score))]));
            let mut n = 0;
            for (t, key, ch, opts) in rec.log.iter().filter(|d| d.1.kind == kind && d.0 <= turn) {
                for o in opts.iter().filter(|o| *o != ch) {
                    let mut k = g.clone();
                    k.insert(key.clone(), o.clone());
                    let sc = opt::play(&pl, &st0, &mut GenomeChooser { genome: &k }, horizon).0;
                    save_plan(&format!("{dir}/v{n}.json"), turn, sc, &k, vec![]);
                    println!("{}", Value::obj(vec![("index", Value::from(n as i64)), ("turn", Value::from(*t)), ("key", key_json(key)),
                        ("from", choice_json(ch)), ("to", choice_json(o)), ("score", Value::from(sc))]));
                    n += 1;
                }
            }
        }
        "skelplay" => {
            // the recorded player's skeleton played turn by turn (snapshots as run prints them), then its score
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let rec = civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]);
            let sk = civ6opt::skeleton::from_record(&rec, turn + ext);
            eprintln!("{}", civ6opt::skeleton::to_json(&sk));
            let mut ch = civ6opt::skeleton::SkeletonChooser::new(&sk);
            let mut st = st0.clone();
            while st.turn <= turn {
                pl.advance(&mut st, &mut ch);
                println!("{}", snapshot(&st));
            }
            println!("{}", opt::play(&pl, &st0, &mut civ6opt::skeleton::SkeletonChooser::new(&sk), horizon).0);
        }
        "skel" => {
            // skeleton search: chains of random changes to a plan's skeleton (crate::skeleton), each keeping a change
            // that does not lower the score, from the recorded player's skeleton, the base policy's and random ones;
            // --minutes per chain, --threads chains (a third from each start)
            use civ6opt::skeleton::*;
            let minutes: f64 = arg(&args, "--minutes").map(|x| x.parse().unwrap()).unwrap_or(5.0);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(12);
            let out = arg(&args, "--out").unwrap_or("data/search/rust_skel.json".into());
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let rec = civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]);
            let user = from_record(&rec, turn + ext);
            // the base policy's skeleton, and what the games offered
            struct Took { districts: Vec<Sym>, techs: Vec<Sym>, civics: Vec<Sym>, govs: Vec<GovPick>, sites: Vec<XY> }
            struct Base<'t> { took: &'t mut Took }
            impl civ6opt::planner::Chooser for Base<'_> {
                fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
                    let c = p.base(st, key, options)?;
                    let k = key.kind;
                    match &c {
                        Choice::S(x) if k == sym("prod") && civ6opt::production::item_kind(*x) == civ6opt::production::Kind::District => self.took.districts.push(*x),
                        Choice::S(x) if k == sym("research") => self.took.techs.push(*x),
                        Choice::S(x) if k == sym("civic") => self.took.civics.push(*x),
                        Choice::Xy(x, y) if k == sym("settle") => self.took.sites.push((*x, *y)),
                        Choice::Appoint(g, cn) => self.took.govs.push(GovPick::Appoint(*g, st.cities.iter().position(|x| x.name == *cn).unwrap_or(0))),
                        Choice::Promote(g, pr) => self.took.govs.push(GovPick::Promote(*g, *pr)),
                        _ => {}
                    }
                    Some(c)
                }
            }
            let mut took = Took { districts: vec![], techs: vec![], civics: vec![], govs: vec![], sites: vec![] };
            let mut pools = Pools::default();
            let (_, _, base_end) = {
                let mut b = Base { took: &mut took };
                let mut w = Watch { inner: &mut b, pools: Pools::default() };
                let r = opt::play(&pl, &st0, &mut w, horizon);
                pools.merge(&w.pools);
                r
            };
            let base = Skeleton { districts: took.districts, techs: took.techs, civics: took.civics, govs: took.govs, sites: took.sites,
                n_cities: base_end.cities.len(), settler_pop: 2, builders_per_city: 1.0, settler_slots: 1, buy_units: true, buy_tiles: true, gold_reserve: 0.0 };
            {
                let mut sc = SkeletonChooser::new(&user);
                let mut w = Watch { inner: &mut sc, pools: Pools::default() };
                opt::play(&pl, &st0, &mut w, horizon);
                pools.merge(&w.pools);
            }
            // specialty districts only (the others are left to the rules)
            let spec = |v: &mut Vec<Sym>| v.retain(|d| p.rules.district_requires_population(*d));
            let mut user = user;
            let mut base = base;
            spec(&mut user.districts);
            spec(&mut base.districts);
            spec(&mut pools.districts);
            let mut rng0 = opt::Rng::new(7);
            let starts: Vec<(&str, Skeleton)> = vec![("user", user.clone()), ("base", base.clone()), ("random", mutate(&base, &pools, &mut rng0, 30))];
            let t0 = Instant::now();
            let results = std::sync::Mutex::new(vec![]);
            std::thread::scope(|sc| {
                for j in 0..threads {
                    let (p, pools, starts, results) = (&p, &pools, &starts, &results);
                    sc.spawn(move || {
                        let (mut pl, st0) = planner(p);
                        pl.s.horizon = turn + ext;
                        let (name, init) = &starts[j % starts.len()];
                        let mut rng = opt::Rng::new(1000 + j as u64);
                        let mut init = init.clone();
                        if *name == "random" {
                            init = mutate(&init, pools, &mut rng, 30);
                        }
                        let eval = |sk: &Skeleton| opt::play(&pl, &st0, &mut SkeletonChooser::new(sk), horizon).0;
                        let start = eval(&init);
                        let (mut cur, mut cur_s, mut n) = (init, start, 0usize);
                        while t0.elapsed().as_secs_f64() < minutes * 60.0 {
                            let k = 1 + (rng.next_u64() % 2) as usize;
                            let m = mutate(&cur, pools, &mut rng, k);
                            let s = eval(&m);
                            n += 1;
                            if s >= cur_s {
                                cur = m;
                                cur_s = s;
                            }
                        }
                        eprintln!("chain {j} ({name}): {start:.1} -> {cur_s:.1} after {n} plays");
                        results.lock().unwrap().push((j, name.to_string(), start, cur_s, cur));
                    });
                }
            });
            let mut results = results.into_inner().unwrap();
            results.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
            let best = &results[0];
            // a skeleton's decisions, as a plan (for the LNS)
            let decisions = |sk: &Skeleton| -> HashMap<Key, Choice> {
                struct Keep<'a, 'b> { inner: SkeletonChooser<'b>, g: &'a mut HashMap<Key, Choice> }
                impl civ6opt::planner::Chooser for Keep<'_, '_> {
                    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
                        let c = self.inner.choose(p, st, key, options)?;
                        self.g.insert(key.clone(), c.clone());
                        Some(c)
                    }
                }
                let mut g = HashMap::new();
                let mut k = Keep { inner: SkeletonChooser::new(sk), g: &mut g };
                opt::play(&pl, &st0, &mut k, horizon);
                g
            };
            let g = decisions(&best.4);
            let plan: Vec<Value> = g.iter().map(|(k, c)| Value::Arr(vec![key_json(k), choice_json(c)])).collect();
            let chains: Vec<Value> = results.iter().map(|r| Value::obj(vec![("chain", Value::from(r.0 as i64)), ("start", Value::from(r.1.as_str())),
                ("from", Value::from(r.2)), ("best", Value::from(r.3)), ("skeleton", to_json(&r.4))])).collect();
            let doc = Value::obj(vec![("turn", Value::from(turn)), ("best", Value::from(best.3)), ("chains", Value::Arr(chains)), ("plan", Value::Arr(plan))]);
            std::fs::write(&out, doc.to_string()).unwrap();
            println!("{}", Value::obj(vec![("best", Value::from(best.3)), ("start", Value::from(best.1.as_str())), ("seconds", Value::from(t0.elapsed().as_secs_f64()))]));
            eprintln!("saved {out}");
            // then the LNS (--lns-passes > 0) from each start's best skeleton and from the recorded play, side by side
            let lns_passes: usize = arg(&args, "--lns-passes").map(|x| x.parse().unwrap()).unwrap_or(0);
            if lns_passes > 0 {
                let lns_budget: usize = arg(&args, "--lns-budget").map(|x| x.parse().unwrap()).unwrap_or(70);
                let mut jobs: Vec<(String, HashMap<Key, Choice>)> = vec![("play".into(), genome_arg(&p, Some("recorded")))];
                for name in ["user", "base", "random"] {
                    if let Some(r) = results.iter().find(|r| r.1 == name) {
                        jobs.push((format!("skel_{name}"), decisions(&r.4)));
                    }
                }
                let per = (threads / jobs.len()).max(1);
                let t1 = Instant::now();
                let done = std::sync::Mutex::new(vec![]);
                std::thread::scope(|sc| {
                    for (tag, g) in jobs {
                        let (p, done, out) = (&p, &done, &out);
                        sc.spawn(move || {
                            let (b, g, hist) = lns(p, g, turn, ext, horizon, 5, lns_budget, per, lns_passes, &format!("[{tag}] "));
                            let f = out.replace(".json", &format!("_lns_{tag}.json"));
                            save_plan(&f, turn, b, &g, vec![("history", Value::Arr(hist))]);
                            done.lock().unwrap().push((tag, b));
                        });
                    }
                });
                for (tag, b) in done.into_inner().unwrap() {
                    println!("{}", Value::obj(vec![("lns_from", Value::from(tag.as_str())), ("best", Value::from(b))]));
                }
                eprintln!("lns {:.0}s", t1.elapsed().as_secs_f64());
            }
        }
        "decisions" => {
            // every decision of a plan up to --turn: turn, key, choice, number of options
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let g = genome_arg(&p, arg(&args, "--genome").as_deref());
            if arg(&args, "--genome").as_deref() == Some("recorded_tiles") {
                pl.s.recorded_tiles = Some(civ6opt::scenario::recorded_tiles_from(&p.v["settings"]["recorded_tiles"]));
            }
            user_play_setup(&p, &mut pl, arg(&args, "--genome").as_deref());
            let mut rec = Recorder { genome: &g, log: vec![] };
            opt::play(&pl, &st0, &mut rec, opt::Horizon::plain(turn));
            for (t, k, c, o) in &rec.log {
                let opts = if args.iter().any(|a| a == "--options") { Value::Arr(o.iter().map(choice_json).collect()) } else { Value::from(o.len()) };
                println!("{}", Value::obj(vec![("turn", Value::from(*t)), ("key", key_json(k)), ("choice", choice_json(c)), ("options", opts)]));
            }
        }
        "compare" => {
            // one decision of a plan, every option: fix it, search the rest again (NRPA from the plan), best score each
            let key = key_from(&json::parse(&arg(&args, "--key").expect("--key '[kind, entity, n]'")).unwrap());
            let budget: usize = arg(&args, "--budget").map(|x| x.parse().unwrap()).unwrap_or(5000);
            let runs: usize = arg(&args, "--runs").map(|x| x.parse().unwrap()).unwrap_or(2);
            let threads: usize = arg(&args, "--threads").map(|x| x.parse().unwrap()).unwrap_or(12);
            let max_opts: usize = arg(&args, "--max-options").map(|x| x.parse().unwrap()).unwrap_or(12);
            let (mut pl0, st00) = planner(&p);
            pl0.s.horizon = turn + ext;
            let g = genome_arg(&p, arg(&args, "--genome").as_deref());
            let mut rec = Recorder { genome: &g, log: vec![] };
            let (plan_score, _, _) = opt::play(&pl0, &st00, &mut rec, horizon);
            let Some((t, _, chosen, options)) = rec.log.iter().find(|x| x.1 == key).cloned() else {
                eprintln!("the plan never makes decision {}", key_json(&key));
                return;
            };
            eprintln!("decision {} at T{t}: plan chose {} ({} options); plan scores {plan_score:.1}", key_json(&key), choice_json(&chosen), options.len());
            // the plan's own choice first, then the others in the planner's order, at most max_opts
            let mut opts = vec![chosen.clone()];
            opts.extend(options.iter().filter(|o| **o != chosen).cloned());
            opts.truncate(max_opts);
            let jobs: Vec<(usize, usize)> = (0..opts.len()).flat_map(|i| (0..runs).map(move |r| (i, r))).collect();
            let next = std::sync::Mutex::new(0usize);
            let collected = std::sync::Mutex::new(vec![]);
            std::thread::scope(|sc| {
                for _ in 0..threads.min(jobs.len()) {
                    sc.spawn(|| {
                        let (mut pl, st0) = planner(&p);
                        pl.s.horizon = turn + ext;
                        loop {
                            let j = { let mut n = next.lock().unwrap(); if *n >= jobs.len() { break; } *n += 1; jobs[*n - 1] };
                            let fixed: HashMap<Key, Choice> = [(key.clone(), opts[j.0].clone())].into_iter().collect();
                            let s = opt::nrpa(&pl, &st0, horizon, budget, 2, 1.0, 3.0, j.1 as u64, Some(&g), Some(&fixed));
                            collected.lock().unwrap().push((j.0, j.1, s.best));
                        }
                    });
                }
            });
            let got = collected.into_inner().unwrap();
            let best_of = |i: usize| got.iter().filter(|x| x.0 == i).map(|x| x.2).fold(f64::MIN, f64::max);
            let top = (0..opts.len()).map(best_of).fold(f64::MIN, f64::max);
            for (i, o) in opts.iter().enumerate() {
                let b = best_of(i);
                println!("{}", Value::obj(vec![("choice", choice_json(o)), ("plan", Value::from(i == 0)), ("best", Value::from(b)), ("loss", Value::from(top - b))]));
            }
        }
        "replay" => {
            // the recorded game replayed (crate::record), compared with the record turn by turn
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext;
            let rec = std::rc::Rc::new(civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]));
            pl.s.recorded_tiles = Some(rec.tiles.clone());
            pl.s.builder_candidates = None;
            pl.s.fixed.clear(); // the search's fixed choices (the opening scout) are not the record's
            pl.rec = Some(rec.clone());
            let verbose = args.iter().any(|a| a == "--verbose");
            let detail: Option<i64> = arg(&args, "--detail").map(|x| x.parse().unwrap());
            let mut ch = civ6opt::record::RecordedChooser::new(&rec);
            let mut st = st0.clone();
            let (mut cum_s, mut cum_g) = (0.0, 0.0);
            while st.turn <= turn {
                let t = st.turn;
                pl.advance(&mut st, &mut ch);
                let out = opt::objective(&st.last_output);
                // a save's city yields are those of the turn before it (made from the previous save's state)
                let game = |tt: i64| rec.turn(tt + 1).map(|r| r.player_food_prod).unwrap_or(0.0);
                cum_s += out;
                cum_g += game(t);
                let Some(r) = rec.turn(t + 1) else { continue };
                let mut diffs: Vec<String> = vec![];
                for c in &st.cities {
                    let Some(rc) = r.city(c.name) else { diffs.push(format!("{} not in record", c.name)); continue };
                    let ca = st.cacc(c.name);
                    if ca.population as i64 != rc.pop {
                        diffs.push(format!("{} pop {}/{}", c.name, ca.population, rc.pop));
                    }
                    if (ca.food_store - rc.food).abs() > 1.0 {
                        diffs.push(format!("{} food {:.1}/{:.1}", c.name, ca.food_store, rc.food));
                    }
                    if c.current != rc.current {
                        diffs.push(format!("{} item {}/{}", c.name, c.current.map(|x| x.as_string()).unwrap_or_default(), rc.current.map(|x| x.as_string()).unwrap_or_default()));
                    }
                    let missing: Vec<String> = rc.buildings.iter().filter(|b| !c.buildings.contains(b)).map(|b| b.as_string()).collect();
                    if !missing.is_empty() {
                        diffs.push(format!("{} lacks {:?}", c.name, missing));
                    }
                    let extra: Vec<String> = c.buildings.iter().filter(|b| !rc.buildings.contains(b)).map(|b| b.as_string()).collect();
                    if !extra.is_empty() {
                        diffs.push(format!("{} extra {:?}", c.name, extra));
                    }
                    let dsim: Vec<(Sym, XY, bool)> = c.districts.iter().filter(|d| d.dtype != s!("DISTRICT_CITY_CENTER") && d.dtype != s!("DISTRICT_WONDER")).map(|d| (d.dtype, (d.x, d.y), d.complete)).collect();
                    for d in &rc.districts {
                        if d.0 == s!("DISTRICT_CITY_CENTER") || d.0 == s!("DISTRICT_WONDER") {
                            continue;
                        }
                        if !dsim.iter().any(|x| x.0 == d.0 && x.1 == d.1 && x.2 == d.2) {
                            diffs.push(format!("{} district {} {:?} done={} differs", c.name, d.0, d.1, d.2));
                        }
                    }
                }
                for rc in &r.cities {
                    if !st.cities.iter().any(|c| c.name == rc.name) {
                        diffs.push(format!("{} missing", rc.name));
                    }
                }
                // the turn's yields of each city against the game's (record of turn t)
                if let Some(r0) = rec.turn(t) {
                    for (n, y) in &st.city_output {
                        if let Some(c0) = r0.city(*n) {
                            if (y[0] - c0.food_yield).abs() >= 1.5 || (y[1] - c0.prod_yield).abs() >= 1.5 {
                                diffs.push(format!("{n} yields F{:.0}/{:.0} P{:.0}/{:.0}", y[0], c0.food_yield, y[1], c0.prod_yield));
                            }
                        }
                    }
                }
                for u in ["UNIT_SETTLER", "UNIT_BUILDER", "UNIT_TRADER"] {
                    let a = st.acc.units_trained.get(&sym(u)).copied().unwrap_or(0);
                    let b = r.units_trained.get(&sym(u)).copied().unwrap_or(0);
                    if a != b {
                        diffs.push(format!("{u} trained {a}/{b}"));
                    }
                }
                if st.acc.techs.len() != r.techs.len() {
                    diffs.push(format!("techs {}/{}", st.acc.techs.len(), r.techs.len()));
                }
                if st.acc.civics.len() != r.civics.len() {
                    diffs.push(format!("civics {}/{}", st.acc.civics.len(), r.civics.len()));
                }
                if detail == Some(t) {
                    let tp: Vec<String> = st.acc.tech_progress.iter().map(|(k, v)| format!("{k} {v:.1}")).collect();
                    let cp: Vec<String> = st.acc.civic_progress.iter().map(|(k, v)| format!("{k} {v:.1}")).collect();
                    println!("  sim pantheon {:?} policies {:?} gov {:?}", st.player.pantheon.map(|x| x.as_string()), st.player.policies.iter().map(|x| x.as_string()).collect::<Vec<_>>(), st.player.government.map(|x| x.as_string()));
                    println!("  sim research {:?} civic {:?} techs {:?} progress {:?} civics {:?} progress {:?} gold {:.1} faith {:.1} output {:?}",
                             st.research.map(|x| x.as_string()), st.civic.map(|x| x.as_string()), st.acc.techs.iter().map(|x| x.as_string()).collect::<Vec<_>>(), tp,
                             st.acc.civics.iter().map(|x| x.as_string()).collect::<Vec<_>>(), cp, st.acc.gold, st.acc.faith, st.last_output);
                    println!("  rec research {:?} civic {:?} techs {:?} progress {:?} civics {:?} progress {:?}", rec.research.get(&t).map(|x| x.as_string()),
                             rec.civic.get(&t).map(|x| x.as_string()), r.techs.iter().map(|x| x.as_string()).collect::<Vec<_>>(),
                             r.tech_progress.iter().map(|(k, v)| format!("{k} {v:.1}")).collect::<Vec<_>>(), r.civics.iter().map(|x| x.as_string()).collect::<Vec<_>>(),
                             r.civic_progress.iter().map(|(k, v)| format!("{k} {v:.1}")).collect::<Vec<_>>());
                    println!("  sim trade capacity {} traders {} routes {:?}", pl.trade_capacity(&st), st.traders.len(),
                             st.traders.iter().map(|t| (t.origin.as_string(), t.dest.map(|d| d.as_string()))).collect::<Vec<_>>());
                    {
                        let env = pl.player_env(&st);
                        let sims: Vec<City> = st.cities.iter().map(|c| { let mut x = c.clone(); x.population = st.cacc(c.name).population; x }).collect();
                        let es = pl.engine(&env, &sims);
                        println!("  sim amenities {:?}", civ6opt::amenities::amenity_scores(p.rules, &sims, &es, 0).iter().map(|(n, v)| (n.as_string(), *v)).collect::<Vec<_>>());
                    }
                    for c in &st.cities {
                        println!("  sim {} districts {:?}", c.name, c.districts.iter().map(|d| (d.dtype.as_string(), d.x, d.y, d.complete)).collect::<Vec<_>>());
                    }
                    for c in &st.cities {
                        let y = st.city_output.iter().find(|x| x.0 == c.name).map(|x| x.1).unwrap_or_default();
                        println!("  sim {} pop {} food {:.1} F {:.1} P {:.1} item {:?} worked {:?}", c.name, st.cacc(c.name).population, st.cacc(c.name).food_store,
                                 y[0], y[1], c.current.map(|x| x.as_string()), c.worked.iter().map(|w| w.xy()).collect::<Vec<_>>());
                        if verbose {
                            let env = pl.player_env(&st);
                            let e = pl.engine(&env, &st.cities);
                            let mods = e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]);
                            let amods = civ6opt::yields::appeal_modifiers(&e, c.name);
                            let full = civ6opt::yields::city_yields(p.rules, &e, c, &[]);
                            let mut tiles = [0.0; 6];
                            let ys = ["YIELD_FOOD", "YIELD_PRODUCTION", "YIELD_GOLD", "YIELD_SCIENCE", "YIELD_CULTURE", "YIELD_FAITH"];
                            for w in &c.worked {
                                let mut wp = w.clone();
                                let center = w.xy() == c.center;
                                wp.district_type = if center { Some(sym("DISTRICT_CITY_CENTER")) } else { None };
                                let y = civ6opt::yields::tile_yields_bbcc(p.rules, &wp, &st.acc.techs, &st.acc.civics, center, center && c.capital, mods, Some(&e.plots), Some(c), Some(&amods));
                                for (k, v) in &y {
                                    if let Some(i) = ys.iter().position(|x| *x == k.as_string()) {
                                        tiles[i] += v;
                                    }
                                }
                            }
                            println!("      city yields without routes {:?}; tiles (bbcc) {:?}; rest {:?}", full, tiles, (0..6).map(|i| full[i] - tiles[i]).collect::<Vec<_>>());
                            for w in &c.worked {
                                let y = civ6opt::yields::tile_yields(p.rules, w, &st.acc.techs, &st.acc.civics, w.xy() == c.center, mods, Some(&e.plots), Some(c), Some(&amods));
                                println!("      tile {:?} {} {:?} {:?} {:?} -> {:?}", w.xy(), w.terrain, w.feature.map(|x| x.as_string()), w.resource.map(|x| x.as_string()),
                                         w.improvement.map(|x| x.as_string()), y.iter().map(|(k, v)| format!("{}{v}", &k.as_string()[6..7])).collect::<Vec<_>>());
                            }
                        }
                        if let Some(rc) = r.city(c.name) {
                            let g = rc.food_yield;
                            let g = (g, rc.prod_yield);
                            println!("  rec {} pop {} food {:.1} F {:.1} P {:.1} item {:?} worked {:?}", rc.name, rc.pop, rc.food, g.0, g.1,
                                     rc.current.map(|x| x.as_string()), rc.worked);
                        }
                    }
                }
                if let Some(kinds) = arg(&args, "--log") {
                    for (k, tt, c) in st.log.iter().filter(|x| x.1 == t) {
                        if kinds.split(',').any(|x| x == k.kind.as_string()) {
                            println!("  T{tt} {} -> {}", key_json(k), c.as_ref().map(choice_json).unwrap_or(Value::Null));
                        }
                    }
                }
                for (nt, n) in pl.rec_notes.borrow().iter().filter(|x| x.0 == t) {
                    println!("  note T{nt}: {n}");
                }
                println!("T{t} F+P sim {out:.1} game {:.1} | cum {cum_s:.0}/{cum_g:.0} | {} differences{}", game(t), diffs.len(),
                         if verbose || diffs.len() <= 6 { format!(": {}", diffs.join("; ")) } else { format!(": {} ...", diffs[..6].join("; ")) });
            }
        }
        "validate" => {
            // each recorded turn on its own: the save's state, the recorded choices, one turn, compared with the next
            // save (no difference carries over to the next turn)
            let (mut pl, _) = planner(&p);
            pl.s.horizon = turn + ext;
            let rec = std::rc::Rc::new(civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]));
            pl.s.recorded_tiles = Some(rec.tiles.clone());
            pl.s.builder_candidates = None;
            pl.s.fixed.clear(); // the search's fixed choices (the opening scout) are not the record's
            pl.rec = Some(rec.clone());
            let from: i64 = arg(&args, "--from").map(|x| x.parse().unwrap()).unwrap_or(2);
            let rows: HashMap<i64, &Value> = p.v["rows"].as_array().unwrap().iter().map(|d| (d["turn"].as_i64().unwrap(), d)).collect();
            let (mut exact, mut n) = (0, 0);
            let mut kinds: HashMap<&'static str, i64> = HashMap::new();
            for t in from..turn {
                let (Some(d), Some(r)) = (rows.get(&t), rec.turn(t + 1)) else { continue };
                if d["cities"].as_array().map(|a| a.is_empty()).unwrap_or(true) {
                    continue;
                }
                let mut st = pl.state_at(d);
                let mut ch = civ6opt::record::RecordedChooser::new(&rec);
                let before = (st.research, st.civic);
                pl.advance(&mut st, &mut ch);
                if args.iter().any(|a| a == "--verbose") {
                    println!("  research {:?} -> {:?} civic {:?} -> {:?} science {:.2} culture {:.2} tech progress {:?}", before.0.map(|x| x.as_string()), st.research.map(|x| x.as_string()),
                             before.1.map(|x| x.as_string()), st.civic.map(|x| x.as_string()), st.last_output[3], st.last_output[4],
                             st.acc.tech_progress.iter().map(|(k, v)| format!("{k} {v:.1}")).collect::<Vec<_>>());
                }
                if let Some(kinds) = arg(&args, "--log") {
                    for (k, tt, c) in st.log.iter() {
                        if kinds.split(',').any(|x| x == k.kind.as_string()) {
                            println!("  T{tt} {} -> {}", key_json(k), c.as_ref().map(choice_json).unwrap_or(Value::Null));
                        }
                    }
                }
                let mut diffs: Vec<String> = vec![];
                // differences the saves cannot settle (they lack the data) are marked "(save)" and not counted as errors
                let mut note = |k: &'static str, s: String, diffs: &mut Vec<String>| {
                    *kinds.entry(k).or_insert(0) += 1;
                    diffs.push(if k.contains("not in the save") || k.contains("same total") { format!("(save) {s}") } else { s });
                };
                // the turn's food and production against the next save's player yields (those of this turn)
                let gy = &rows.get(&(t + 1)).map(|x| &x["player"]["yields"]);
                let (gf, gp) = gy.map(|y| (y["YIELD_FOOD"].as_f64().unwrap_or(0.0), y["YIELD_PRODUCTION"].as_f64().unwrap_or(0.0))).unwrap_or((0.0, 0.0));
                if (st.last_output[0] - gf).abs() > 0.05 || (st.last_output[1] - gp).abs() > 0.05 {
                    // (the saves show the yields as of the next turn's start, with the item's percent bonus: changes made
                    // there, such as a new government, show in them; the cities' food and progress are checked below)
                    note("yields (shown at the next turn's start, not in the save)", format!("F {:.1}/{gf} P {:.1}/{gp}", st.last_output[0], st.last_output[1]), &mut diffs);
                }
                for c in &st.cities {
                    let Some(rc) = r.city(c.name) else { continue };
                    let ca = st.cacc(c.name);
                    if ca.population as i64 != rc.pop {
                        note("pop", format!("{} pop {}/{}", c.name, ca.population, rc.pop), &mut diffs);
                    }
                    if (ca.food_store - rc.food).abs() > 0.5 {
                        note("food", format!("{} food {:.1}/{:.1}", c.name, ca.food_store, rc.food), &mut diffs);
                    }
                    let gprog = rows.get(&(t + 1)).and_then(|x| x["cities"].as_array().and_then(|a| a.iter().find(|cc| osym(&cc["name"]) == Some(c.name)).cloned()));
                    // the save holds the overflow not yet in the item (whole units): the item's progress and the overflow
                    // together, against the model's
                    let held = gprog.as_ref().and_then(|x| x["production"]["heldOverflow"].as_f64());
                    if let (Some(item), Some(gc), Some(h)) = (rc.current, gprog.as_ref(), held) {
                        let gt = gc["production"]["progress"][item.as_string().as_str()].as_f64().unwrap_or(0.0) + h;
                        let mt = ca.progress.get(&item).copied().unwrap_or(0.0) + ca.overflow + ca.raw;
                        // the overflow the turn began with is held in whole units, so the model's start lacks its fraction
                        // (under 1, then times the item's percent bonus; 10/18 Nantes T29: 4.5 held as 4)
                        let start_held = d["cities"].as_array().and_then(|a| a.iter().find(|cc| osym(&cc["name"]) == Some(c.name)))
                            .and_then(|cc| cc["production"]["heldOverflow"].as_f64()).unwrap_or(0.0);
                        if mt - gt < -0.5 || mt - gt >= 1.0 {
                            if start_held > 0.0 && gt > mt && gt - mt < 2.0 {
                                note("progress (overflow fraction, not in the save)", format!("{} {} progress {:.1}/{:.1}", c.name, item, mt, gt), &mut diffs);
                            } else {
                                note("progress", format!("{} {} progress {:.1}/{:.1}", c.name, item, mt, gt), &mut diffs);
                            }
                        }
                    }
                    if let (Some(item), Some(gc), None) = (rc.current, gprog, held) {
                        let g = gc["production"]["progress"][item.as_string().as_str()].as_f64().unwrap_or(0.0);
                        // the model holds a finished item's overflow apart until the next item; the game puts it in the next one
                        // (a city the record left without an item this turn keeps its production apart too: not in the save)
                        let idle_now = d["cities"].as_array().and_then(|a| a.iter().find(|cc| osym(&cc["name"]) == Some(c.name)))
                            .map(|cc| cc["production"]["current"].is_null()).unwrap_or(false);
                        let sv = ca.progress.get(&item).copied().unwrap_or(0.0) + if !idle_now && (c.current.is_none() || c.current == Some(item) && ca.progress.get(&item).copied().unwrap_or(0.0) == 0.0) { ca.overflow } else { 0.0 };
                        let started = d["cities"].as_array().and_then(|a| a.iter().find(|cc| osym(&cc["name"]) == Some(c.name)))
                            .map(|cc| cc["production"]["progress"][item.as_string().as_str()].as_f64().unwrap_or(0.0) == 0.0).unwrap_or(true);
                        // the overflow the model holds may already be in the game's next item or still held apart
                        let sv_held = ca.progress.get(&item).copied().unwrap_or(0.0);
                        if (g - sv).abs() > 0.5 && (g - sv_held).abs() > 0.5 && g > 0.0 {
                            // an item begun this turn gets the overflow of the last one, which the saves do not hold
                            if started && sv < g {
                                note("progress (overflow, not in the save)", format!("{} {} progress {:.1}/{:.1}", c.name, item, sv, g), &mut diffs);
                            } else {
                                note("progress", format!("{} {} progress {:.1}/{:.1}", c.name, item, sv, g), &mut diffs);
                            }
                        }
                    }
                    let missing: Vec<String> = rc.buildings.iter().filter(|b| !c.buildings.contains(b)).map(|b| b.as_string()).collect();
                    let extra: Vec<String> = c.buildings.iter().filter(|b| !rc.buildings.contains(b)).map(|b| b.as_string()).collect();
                    if !missing.is_empty() || !extra.is_empty() {
                        note("buildings", format!("{} buildings -{:?} +{:?}", c.name, missing, extra), &mut diffs);
                    }
                }
                if st.acc.techs.len() != r.techs.len() || st.acc.civics.len() != r.civics.len() {
                    // one behind after a completion last turn: its overflow (not in the save) finished the next one
                    let n_of = |tt: i64, k: &str| rows.get(&tt).map(|x| x["player"][k].as_array().map(|a| a.len()).unwrap_or(0)).unwrap_or(0);
                    let carried = (st.acc.techs.len() + 1 == r.techs.len() && n_of(t - 1, "techs") < n_of(t, "techs"))
                        || (st.acc.civics.len() + 1 == r.civics.len() && n_of(t - 1, "civics") < n_of(t, "civics"));
                    let msg = format!("techs {}/{} civics {}/{}", st.acc.techs.len(), r.techs.len(), st.acc.civics.len(), r.civics.len());
                    if carried {
                        note("research (overflow, not in the save)", msg, &mut diffs);
                    } else {
                        note("research", msg, &mut diffs);
                    }
                }
                for (k, v) in &r.tech_progress {
                    let sv = st.acc.tech_progress.get(k).copied().unwrap_or(0.0);
                    if (sv - v).abs() > 0.5 && !st.acc.techs.contains(k) {
                        let fresh = d["player"]["techProgress"][k.as_string().as_str()].as_f64().unwrap_or(0.0) == 0.0;
                        // one finished last turn left an overflow the save does not hold
                        let finished_last = rows.get(&(t - 1)).map(|x| x["player"]["techs"].as_array().map(|a| a.len()).unwrap_or(0) < d["player"]["techs"].as_array().map(|a| a.len()).unwrap_or(0)).unwrap_or(false);
                        // one finished this turn: the game puts its overflow into the queued one now, the model next turn
                        let pending = st.acc.tech_overflow;
                        if pending > 0.0 && (sv + pending - v).abs() <= 0.5 {
                            note("research (overflow next turn, same total)", format!("{k} {sv:.1}+{pending:.1}/{v:.1}"), &mut diffs);
                        } else if (fresh || finished_last) && sv < *v {
                            note("research (overflow, not in the save)", format!("{k} {sv:.1}/{v:.1}"), &mut diffs);
                        } else {
                            note("research", format!("{k} {sv:.1}/{v:.1}"), &mut diffs);
                        }
                    }
                }
                for (k, v) in &r.civic_progress {
                    let sv = st.acc.civic_progress.get(k).copied().unwrap_or(0.0);
                    if (sv - v).abs() > 0.5 && !st.acc.civics.contains(k) {
                        let fresh = d["player"]["civicProgress"][k.as_string().as_str()].as_f64().unwrap_or(0.0) == 0.0;
                        // one finished last turn left an overflow the save does not hold
                        let finished_last = rows.get(&(t - 1)).map(|x| x["player"]["civics"].as_array().map(|a| a.len()).unwrap_or(0) < d["player"]["civics"].as_array().map(|a| a.len()).unwrap_or(0)).unwrap_or(false);
                        // one finished this turn: the game puts its overflow into the queued one now, the model next turn
                        let pending = st.acc.civic_overflow;
                        if pending > 0.0 && (sv + pending - v).abs() <= 0.5 {
                            note("research (overflow next turn, same total)", format!("{k} {sv:.1}+{pending:.1}/{v:.1}"), &mut diffs);
                        } else if (fresh || finished_last) && sv < *v {
                            note("research (overflow, not in the save)", format!("{k} {sv:.1}/{v:.1}"), &mut diffs);
                        } else {
                            note("research", format!("{k} {sv:.1}/{v:.1}"), &mut diffs);
                        }
                    }
                }
                for (_, nt) in pl.rec_notes.borrow().iter().filter(|x| x.0 == t) {
                    if nt.contains("done by the record") {
                        note("done by the record (bought or carried over, not in the save)", nt.clone(), &mut diffs);
                    }
                }
                // the boost rules on the next save's state (the recorded actions already done): each boost recorded this
                // turn should follow from them, and none they give should be missing from the record. Boosts by events
                // the model does not play (meeting, exploring, combat, religion, alliances; tribal villages) are the record's.
                if let Some(dn) = rows.get(&(t + 1)) {
                    let sn = pl.state_at(dn);
                    let by_rules: Vec<Sym> = pl.endogenous_boosts(&sn);
                    let before = rec.turn(t).map(|x| x.boosted.clone()).unwrap_or_default();
                    let cls_of = |item: Sym| {
                        let col = if item.starts_with("TECH_") { "TechnologyType" } else { "CivicType" };
                        p.rules.lookup_sym("Boosts", col, item, "BoostClass").map(|c| c.as_string()).unwrap_or_default()
                    };
                    for b in r.boosted.iter().filter(|b| !before.contains(b)) {
                        let cls = cls_of(*b);
                        if !cls.is_empty() && !EXOGENOUS_BOOSTS.contains(&cls.as_str()) && !by_rules.contains(b) {
                            // its condition does not hold in the save: another source (village, city-state, ...), given by the record
                            note("boost from another source (given by the record, not in the save)", format!("boost {b} recorded, not by the rules ({cls})"), &mut diffs);
                        }
                    }
                    let done: Vec<Sym> = r.techs.iter().chain(r.civics.iter()).copied().collect();
                    for b in by_rules.iter().filter(|b| !r.boosted.contains(b) && !done.contains(b)) {
                        note("boost", format!("boost {b} by the rules, not recorded ({})", cls_of(*b)), &mut diffs);
                    }
                }
                n += 1;
                if diffs.iter().all(|d| d.starts_with("(save)")) {
                    exact += 1;
                }
                println!("T{t} {}", if diffs.is_empty() { "ok".to_string() } else { diffs.join("; ") });
            }
            let mut ks: Vec<_> = kinds.into_iter().collect();
            ks.sort();
            println!("turns without model errors {exact}/{n}; differences by kind {ks:?}");
        }
        "step" => {
            // one recorded turn from its save with the recorded choices (as validate): the state after it, as JSON
            // (rules tests: tests/cases.json)
            let (mut pl, _) = planner(&p);
            let rec = std::rc::Rc::new(civ6opt::record::Record::load(&p.v["rows"], &pl.sc.unit_gifts, &p.v["map"]));
            pl.s.recorded_tiles = Some(rec.tiles.clone());
            pl.s.builder_candidates = None;
            pl.s.fixed.clear();
            pl.rec = Some(rec.clone());
            let d = p.v["rows"].as_array().unwrap().iter().find(|d| d["turn"].as_i64() == Some(turn)).expect("no such turn");
            let mut st = pl.state_at(d);
            let mut ch = civ6opt::record::RecordedChooser::new(&rec);
            pl.advance(&mut st, &mut ch);
            let env = pl.player_env(&st);
            let e = pl.engine(&env, &st.cities);
            let am = civ6opt::amenities::amenity_scores(p.rules, &st.cities, &e, env.extra_luxuries);
            let cities: Vec<Value> = st.cities.iter().map(|c| {
                let y = st.city_output.iter().find(|x| x.0 == c.name).map(|x| x.1).unwrap_or_default();
                let mods = e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]);
                let tiles: Vec<Value> = c.owned.iter().map(|w| {
                    let ty = civ6opt::yields::tile_yields(p.rules, w, &st.acc.techs, &st.acc.civics, w.xy() == c.center, mods, Some(&e.plots), Some(c), None);
                    Value::obj(vec![("xy", Value::from(vec![w.x, w.y])), ("yields", Value::obj(ty.iter().map(|(k, v)| (k.as_string(), Value::from(*v))).collect::<Vec<_>>()
                        .iter().map(|(k, v)| (k.as_str(), v.clone())).collect()))])
                }).collect();
                Value::obj(vec![("name", Value::from(c.name.as_string())), ("pop", Value::from(st.cacc(c.name).population as i64)),
                    ("food", Value::from(st.cacc(c.name).food_store)), ("F", Value::from(y[0])), ("P", Value::from(y[1])),
                    ("housing", Value::from(civ6opt::growth::housing(p.rules, c, Some(&e)))),
                    ("amenity", Value::from(am.iter().find(|x| x.0 == c.name).map(|x| x.1).unwrap_or(0.0))),
                    ("progress", Value::Obj(st.cacc(c.name).progress.iter().map(|(k, v)| (k.as_string(), Value::from(*v))).collect())),
                    ("overflow", Value::from(st.cacc(c.name).overflow)),
                    ("buildings", Value::Arr(c.buildings.iter().map(|b| Value::from(b.as_string())).collect())), ("tiles", Value::Arr(tiles))])
            }).collect();
            println!("{}", Value::obj(vec![("turn", Value::from(turn)), ("F", Value::from(st.last_output[0])), ("P", Value::from(st.last_output[1])),
                ("techs", Value::from(st.acc.techs.len())), ("civics", Value::from(st.acc.civics.len())),
                ("policies", Value::Arr(st.player.policies.iter().map(|x| Value::from(x.as_string())).collect())),
                ("units_trained", Value::Obj(st.acc.units_trained.iter().map(|(k, v)| (k.as_string(), Value::from(*v as f64))).collect())),
                ("exo_boosts", Value::Arr(pl.sc.exo_boosts.get(&turn).cloned().unwrap_or_default().iter().map(|x| Value::from(x.as_string())).collect())),
                ("cities", Value::Arr(cities))]));
        }
        "debug" => {
            let (mut pl, st0) = planner(&p);
            pl.s.horizon = turn + ext; // the builders' returns count to the end of the evaluation
            let env = pl.player_env(&st0);
            let e = pl.engine(&env, &st0.cities);
            let c = &st0.cities[0];
            println!("city {} pop {} buildings {:?}", c.name, c.population, c.buildings.iter().map(|b| b.as_string()).collect::<Vec<_>>());
            for b in &c.buildings {
                println!("  {} -> {:?}", b, p.rules.yield_rows("Building_YieldChanges", "BuildingType", *b, "YieldType", "YieldChange").iter().map(|(k, v)| (k.as_string(), *v)).collect::<Vec<_>>());
            }
            for w in c.owned.iter().take(8) {
                let y = civ6opt::yields::tile_yields(p.rules, w, &env.techs, &env.civics, w.xy() == c.center, e.plot_effects.get(&c.name).map(|v| v.as_slice()).unwrap_or(&[]), Some(&e.plots), Some(c), None);
                println!("  tile {:?} {} {:?} {:?} -> {:?}", w.xy(), w.terrain, w.feature.map(|x| x.as_string()), w.resource.map(|x| x.as_string()), y.iter().map(|(k, v)| (k.as_string(), *v)).collect::<Vec<_>>());
            }
            println!("effects: {:?}", e.effects.list.iter().map(|(k, v)| (k.as_string(), v.len())).collect::<Vec<_>>());
            println!("total city yields {:?}", civ6opt::yields::city_yields(p.rules, &e, c, &[]));
        }
        _ => eprintln!("usage: civ6opt run|search <problem.json> ..."),
    }
}
