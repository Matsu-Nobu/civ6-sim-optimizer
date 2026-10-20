//! Search for the best plan: NRPA with the incumbent plan as prior (civ6sim/opt.py), objective = per-turn food +
//! production of all cities summed over the turns up to `turn`.
use crate::model::*;
use crate::sym::Sym;
use crate::planner::{Chooser, Planner};
use std::collections::HashMap;

pub fn objective(out: &[f64; 6]) -> f64 {
    out[0] + out[1]
}

/// Units that do nothing for the objective (all but settlers, builders, traders and scouts, which unlock the
/// recorded exploration rewards) cost this much each when trained or bought by the end of the extension (one the plan
/// left half built counts; the extension policy never starts one): among plans that score the same, the one without
/// them wins, so plans stop carrying arbitrary military builds.
pub const IDLE_UNIT_PENALTY: f64 = 1.0;

const USEFUL_UNITS: [&str; 4] = ["UNIT_SETTLER", "UNIT_BUILDER", "UNIT_TRADER", "UNIT_SCOUT"];

pub fn idle_unit(u: Sym) -> bool {
    u.starts_with("UNIT_") && !USEFUL_UNITS.iter().any(|x| u.as_string() == *x)
}

pub fn idle_units(st0: &State, st: &State) -> f64 {
    st.acc.units_trained.0.iter().filter(|(u, _)| idle_unit(*u))
        .map(|(u, n)| (n - st0.acc.units_trained.get(u).copied().unwrap_or(0)).max(0) as f64).sum()
}

/// The evaluation: the objective summed over the turns up to `turn`, plus `w` x the objective of `ext` more turns
/// played by the fixed ExtChooser (not by the plan), so what the plan leaves at `turn` (population, improvements,
/// half-built items, gold) counts and end-of-horizon tricks do not pay.
#[derive(Clone, Copy, Debug)]
pub struct Horizon {
    pub turn: i64,
    pub ext: i64,
    pub w: f64,
    /// the turns whose decisions a rollout samples (LNS); elsewhere it takes the prior (incumbent, else base). None: all
    pub free: Option<(i64, i64)>,
    /// the policy's weights also on general codes of a move (move_codes), shared across decisions
    pub general: bool,
}

impl Horizon {
    pub fn plain(turn: i64) -> Horizon {
        Horizon { turn, ext: 0, w: 0.0, free: None, general: false }
    }
}

/// The policy of the extension turns: the base policy, but gold buys the cheapest tile on offer (the base policy
/// never spends, which would leave gold worth nothing) and no settlers or idle units are made.
pub struct ExtChooser;

impl Chooser for ExtChooser {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        if key.kind == s!("prod") {
            // no settlers either: one would not found a city within the extension, so it would only cost the city
            // production and a citizen and misprice the position the plan leaves
            let keep: Vec<Choice> = options.iter().filter(|o| !o.sym().map(|u| idle_unit(u) || u == s!("UNIT_SETTLER")).unwrap_or(false)).cloned().collect();
            if !keep.is_empty() {
                return p.base(st, key, &keep);
            }
        }
        if key.kind == s!("buy") && key.n > 0 {
            // one purchase a turn
            return Some(Choice::Nothing).filter(|c| options.contains(c)).or_else(|| p.base(st, key, options));
        }
        if key.kind == s!("buy") {
            let tile = options.iter().filter_map(|o| match o { Choice::BuyTile(_, g, _, _) => Some((f64::from_bits(*g), o)), _ => None })
                .min_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            if let Some((_, o)) = tile {
                return Some(o.clone());
            }
        }
        p.base(st, key, options)
    }
}

/// Play to the end of `h.turn` (then the extension); returns (score, objective up to h.turn, final state at h.turn).
pub fn play(p: &Planner, st0: &State, ch: &mut dyn Chooser, h: Horizon) -> (f64, f64, State) {
    let mut st = st0.clone();
    let mut total = 0.0;
    while st.turn <= h.turn {
        p.advance(&mut st, ch);
        total += objective(&st.last_output);
    }
    let floor = p.milestones.as_ref().map(|m| m.penalty(&st)).unwrap_or(0.0);
    let mut ext = 0.0;
    let mut xs = st.clone();
    if h.ext > 0 {
        let mut xc = ExtChooser;
        while xs.turn <= h.turn + h.ext {
            p.advance(&mut xs, &mut xc);
            ext += objective(&xs.last_output);
        }
    }
    let penalty = IDLE_UNIT_PENALTY * idle_units(st0, &xs);
    (total + h.w * ext - penalty - floor, total, st)
}

/// Like play, from a state of the same game at the start of a turn (`start`, with `prefix` the objective of the turns
/// before it): the score of the whole game (the idle-unit penalty counted from `st0`).
pub fn play_from(p: &Planner, st0: &State, start: &State, prefix: f64, ch: &mut dyn Chooser, h: Horizon) -> f64 {
    let mut st = start.clone();
    let mut total = prefix;
    while st.turn <= h.turn {
        p.advance(&mut st, ch);
        total += objective(&st.last_output);
    }
    let floor = p.milestones.as_ref().map(|m| m.penalty(&st)).unwrap_or(0.0);
    let mut ext = 0.0;
    if h.ext > 0 {
        let mut xc = ExtChooser;
        while st.turn <= h.turn + h.ext {
            p.advance(&mut st, &mut xc);
            ext += objective(&st.last_output);
        }
    }
    total + h.w * ext - IDLE_UNIT_PENALTY * idle_units(st0, &st) - floor
}

/// Follow a genome; elsewhere the lookahead with the learned value (crate::value) when one is set, else the base
/// policy.
pub struct GenomeChooser<'g> {
    pub genome: &'g HashMap<Key, Choice>,
}

impl Chooser for GenomeChooser<'_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        if let Some(c) = self.genome.get(key) {
            if options.contains(c) {
                return Some(c.clone());
            }
        }
        crate::value::choose(p, st, key, options, self.genome).or_else(|| p.base(st, key, options))
    }
}

/// A small fast PRNG (xorshift64*).
pub struct Rng(pub u64);
impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed.wrapping_mul(0x9E3779B97F4A7C15) | 1)
    }
    pub fn next_u64(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545F4914F6CDD1D)
    }
    pub fn f64(&mut self) -> f64 {
        (self.next_u64() >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// Policy weights by move code (move_codes).
pub type Policy = HashMap<u64, f64>;
/// A rollout's decisions: key, choice, options, sampling probabilities (None: not sampled), each option's codes.
pub type Seq = Vec<(Key, Choice, Vec<Choice>, Option<Vec<f64>>, Vec<Vec<u64>>)>;

fn hash_of<T: std::hash::Hash>(x: T) -> u64 {
    use std::hash::Hasher;
    let mut h = std::collections::hash_map::DefaultHasher::new();
    x.hash(&mut h);
    h.finish()
}

/// The codes a move's policy weight is the sum of (generalized NRPA, Cazenave 2020: the code of a move is the
/// domain's choice): the move at this decision; and with `general`, what the option is apart from where and when
/// (a building, an improvement, a tech, a site, a governor's use, a route's kind), the same in the turn's tenth of the
/// game, and the same in the kind of city it is for (capital or not, population band), so what is learned at one
/// decision carries to the others.
pub fn move_codes(st: &State, key: &Key, o: &Choice, general: bool) -> Vec<u64> {
    let mut v = vec![hash_of((0u8, key, o))];
    if !general {
        return v;
    }
    let k = key.kind;
    let big = |n: Sym| {
        let mut pops: Vec<(i32, Sym)> = st.cities.iter().map(|c| (st.acc.cities.get(&c.name).map(|a| a.population).unwrap_or(0), c.name)).collect();
        pops.sort_by(|a, b| b.cmp(a));
        pops.iter().take(2).any(|x| x.1 == n)
    };
    let (what, city): ((u8, Sym, Sym), Option<Sym>) = match o {
        Choice::S(x) => ((1, *x, Sym(0)), if let Ent::S(c) = key.ent { Some(c) } else { None }),
        Choice::Xy(x, y) => ((2, Sym(*x as u32), Sym(*y as u32)), None),
        Choice::Act(kind, c, _, _, w) => ((3, *kind, *w), Some(*c)),
        Choice::Wait => ((4, Sym(0), Sym(0)), None),
        Choice::Nothing => ((5, Sym(0), Sym(0)), None),
        Choice::BuyItem(c, it, _, cur) => ((6, *it, *cur), Some(*c)),
        Choice::BuyTile(c, ..) => ((7, Sym(0), Sym(0)), Some(*c)),
        Choice::Appoint(g, c) => ((8, *g, Sym(0)), Some(*c)),
        Choice::Promote(g, pr) => ((9, *g, *pr), None),
        Choice::Route(o, d) => ((10, Sym(big(*o) as u32), Sym(if st.city(*d).is_none() { 2 } else { big(*d) as u32 })), Some(*o)),
    };
    v.push(hash_of((1u8, k, what)));
    v.push(hash_of((2u8, k, what, st.turn / 10)));
    if let Some(c) = city {
        let pop = st.acc.cities.get(&c).map(|a| a.population).unwrap_or(0);
        let capital = st.cities.first().map(|x| x.name) == Some(c);
        v.push(hash_of((3u8, k, what, capital, (pop / 4).min(4))));
    }
    v
}

/// The sampling rule of a policy rollout: a fixed choice where the plan fixes one, else softmax over the options of
/// policy weight + `bias` on the prior (the incumbent's choice where valid, else the base policy's).
struct Sampler<'a> {
    policy: &'a Policy,
    bias: f64,
    incumbent: Option<&'a HashMap<Key, Choice>>,
    fixed: Option<&'a HashMap<Key, Choice>>,
    free: Option<(i64, i64)>,
    general: bool,
}

impl Sampler<'_> {
    fn prior(&self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        self.incumbent.and_then(|g| g.get(key)).filter(|c| options.contains(c)).cloned().or_else(|| p.base(st, key, options))
    }

    fn sample(&self, rng: &mut Rng, turn: i64, key: &Key, options: &[Choice], codes: &[Vec<u64>], prior: Option<&Choice>) -> (Choice, Option<Vec<f64>>) {
        if let Some(f) = self.fixed.and_then(|f| f.get(key)) {
            if options.contains(f) {
                return (f.clone(), None);
            }
        }
        if let (Some((a, b)), Some(pr)) = (self.free, prior) {
            if turn < a || turn > b {
                return (pr.clone(), None);
            }
        }
        let _ = key;
        let lg: Vec<f64> = options.iter().zip(codes).map(|(o, cs)| cs.iter().map(|c| self.policy.get(c).copied().unwrap_or(0.0)).sum::<f64>()
            + if Some(o) == prior { self.bias } else { 0.0 }).collect();
        let m = lg.iter().cloned().fold(f64::MIN, f64::max);
        let ws: Vec<f64> = lg.iter().map(|x| (x - m).exp()).collect();
        let tot: f64 = ws.iter().sum();
        let probs: Vec<f64> = ws.iter().map(|w| w / tot).collect();
        let x = rng.f64();
        let mut acc = 0.0;
        let mut c = options[options.len() - 1].clone();
        for (o, pr) in options.iter().zip(probs.iter()) {
            acc += pr;
            if x <= acc {
                c = o.clone();
                break;
            }
        }
        (c, Some(probs))
    }
}

/// One decision of a finished rollout, enough to sample it again without simulating: the same choices before it
/// mean the same state, so the same options and prior.
#[derive(Clone)]
struct Dec {
    turn: i64,
    codes: Vec<Vec<u64>>,
    key: Key,
    options: Vec<Choice>,
    prior: Option<Choice>,
    chosen: Choice,
}

/// A finished rollout kept for reuse: its decisions, the state at the start of every turn (with the number of
/// decisions and the objective summed before it) and its score.
struct Ref {
    decs: Vec<Dec>,
    snaps: Vec<(usize, f64, std::rc::Rc<State>)>,
    score: f64,
}

/// Recent rollouts. A new rollout first walks the decisions they share (sampling each as a full simulation would, so
/// the random numbers and results are exactly those of simulating), and simulates only from the start of the turn
/// where its choices leave all of them; when they never do, it is a repeat and takes the score.
pub struct Reuse {
    refs: std::collections::VecDeque<Ref>,
    cap: usize,
    /// rollouts, repeats, turns simulated (up to the horizon), turns skipped
    pub stats: [u64; 4],
}

impl Reuse {
    pub fn new(cap: usize) -> Reuse {
        Reuse { refs: Default::default(), cap, stats: [0; 4] }
    }
}

/// How many recent rollouts to keep for reuse (CIV6OPT_REUSE, default 16; 0: simulate every rollout in full).
fn reuse_cap() -> usize {
    std::env::var("CIV6OPT_REUSE").ok().and_then(|x| x.parse().ok()).unwrap_or(16)
}

struct PolicyChooser<'a, 'b> {
    sampler: &'b Sampler<'a>,
    rng: &'b mut Rng,
    /// choices already sampled in the walk, replayed first (from the restart snapshot up to the divergence)
    replay: std::collections::VecDeque<(Key, Choice)>,
    seq: Seq,
    decs: Vec<Dec>,
}

impl Chooser for PolicyChooser<'_, '_> {
    fn choose(&mut self, p: &Planner, st: &State, key: &Key, options: &[Choice]) -> Option<Choice> {
        if let Some((k, c)) = self.replay.pop_front() {
            assert!(&k == key && options.contains(&c), "replay diverged at {key:?}");
            return Some(c);
        }
        let prior = self.sampler.prior(p, st, key, options);
        let codes: Vec<Vec<u64>> = options.iter().map(|o| move_codes(st, key, o, self.sampler.general)).collect();
        let (c, probs) = self.sampler.sample(self.rng, st.turn, key, options, &codes, prior.as_ref());
        self.seq.push((key.clone(), c.clone(), options.to_vec(), probs, codes.clone()));
        self.decs.push(Dec { turn: st.turn, codes, key: key.clone(), options: options.to_vec(), prior, chosen: c.clone() });
        Some(c)
    }
}

#[allow(clippy::too_many_arguments)]
pub fn policy_rollout(p: &Planner, st0: &State, h: Horizon, policy: &Policy, bias: f64, rng: &mut Rng,
                      incumbent: Option<&HashMap<Key, Choice>>, fixed: Option<&HashMap<Key, Choice>>, reuse: &mut Reuse) -> (f64, Seq) {
    use std::rc::Rc;
    let sampler = Sampler { policy, bias, incumbent, fixed, free: h.free, general: h.general };
    let mut seq: Seq = vec![];
    let mut decs: Vec<Dec> = vec![];
    // walk the decisions shared with the kept rollouts
    let mut alive: Vec<usize> = (0..reuse.refs.len()).collect();
    let mut restart: Option<(usize, usize)> = None; // (ref, decision index of the divergence)
    let mut i = 0;
    while !alive.is_empty() {
        let r0 = &reuse.refs[alive[0]];
        if i == r0.decs.len() {
            // every choice as in r0: the same game
            let score = r0.score;
            let snaps = r0.snaps.clone();
            reuse.stats[0] += 1;
            reuse.stats[1] += 1;
            reuse_push(reuse, Ref { decs, snaps, score });
            return (score, seq);
        }
        let d = &r0.decs[i];
        let (c, probs) = sampler.sample(rng, d.turn, &d.key, &d.options, &d.codes, d.prior.as_ref());
        seq.push((d.key.clone(), c.clone(), d.options.clone(), probs, d.codes.clone()));
        decs.push(Dec { turn: d.turn, codes: d.codes.clone(), key: d.key.clone(), options: d.options.clone(), prior: d.prior.clone(), chosen: c.clone() });
        let next: Vec<usize> = alive.iter().copied().filter(|&r| reuse.refs[r].decs.get(i).map(|x| x.chosen == c).unwrap_or(false)).collect();
        if next.is_empty() {
            restart = Some((alive[0], i));
            break;
        }
        alive = next;
        i += 1;
    }
    // simulate from the start of the turn of the divergence (or from the start)
    let (mut st, mut snaps, replay) = match restart {
        Some((r, i)) => {
            let rf = &reuse.refs[r];
            let k = rf.snaps.iter().rposition(|(j, _, _)| *j <= i).unwrap();
            let j = rf.snaps[k].0;
            let replay = decs[j..=i].iter().map(|d| (d.key.clone(), d.chosen.clone())).collect();
            ((*rf.snaps[k].2).clone(), rf.snaps[..=k].to_vec(), replay)
        }
        None => (st0.clone(), vec![], Default::default()),
    };
    // the objective of the turns before the restart, summed in the same order as simulating them
    let mut total: f64 = snaps.last().map(|x| x.1).unwrap_or(0.0);
    let mut ch = PolicyChooser { sampler: &sampler, rng, replay, seq, decs };
    let mut first = restart.is_some();
    reuse.stats[0] += 1;
    reuse.stats[3] += (st.turn - st0.turn).max(0) as u64;
    reuse.stats[2] += (h.turn + 1 - st.turn).max(0) as u64;
    while st.turn <= h.turn {
        if !first {
            snaps.push((ch.seq.len(), total, Rc::new(st.clone())));
        }
        first = false;
        p.advance(&mut st, &mut ch);
        total += objective(&st.last_output);
    }
    assert!(ch.replay.is_empty());
    let floor = p.milestones.as_ref().map(|m| m.penalty(&st)).unwrap_or(0.0);
    let mut ext = 0.0;
    if h.ext > 0 {
        let mut xc = ExtChooser;
        while st.turn <= h.turn + h.ext {
            p.advance(&mut st, &mut xc);
            ext += objective(&st.last_output);
        }
    }
    let penalty = IDLE_UNIT_PENALTY * idle_units(st0, &st);
    let score = total + h.w * ext - penalty - floor;
    let PolicyChooser { seq, decs, .. } = ch;
    reuse_push(reuse, Ref { decs, snaps, score });
    (score, seq)
}

fn reuse_push(reuse: &mut Reuse, r: Ref) {
    if reuse.cap == 0 {
        return;
    }
    if reuse.refs.len() >= reuse.cap {
        reuse.refs.pop_front();
    }
    reuse.refs.push_back(r);
}

pub fn adapt(policy: &Policy, seq: &Seq, alpha: f64) -> Policy {
    let mut pol = policy.clone();
    // the gradient of each code, averaged over the decisions it appears in: a general code shared by many decisions
    // moves about as far per step as a move's own (with one code per move, plain NRPA)
    let mut grad: HashMap<u64, (f64, u32)> = HashMap::new();
    for (_key, c, options, probs, codes) in seq {
        let Some(probs) = probs else { continue };
        let Some(ci) = options.iter().position(|o| o == c) else { continue };
        let mut seen: std::collections::HashSet<u64> = std::collections::HashSet::new();
        for code in &codes[ci] {
            grad.entry(*code).or_insert((0.0, 0)).0 += 1.0;
            seen.insert(*code);
        }
        for (cs, pr) in codes.iter().zip(probs.iter()) {
            for code in cs {
                grad.entry(*code).or_insert((0.0, 0)).0 -= pr;
                seen.insert(*code);
            }
        }
        for code in seen {
            grad.get_mut(&code).unwrap().1 += 1;
        }
    }
    let mut g: Vec<(u64, (f64, u32))> = grad.into_iter().collect();
    g.sort_by_key(|x| x.0);
    for (code, (v, n)) in g {
        *pol.entry(code).or_insert(0.0) += alpha * v / n.max(1) as f64;
    }
    pol
}

pub fn genome_of(seq: &Seq) -> HashMap<Key, Choice> {
    seq.iter().map(|(k, c, _, _, _)| (k.clone(), c.clone())).collect()
}

pub struct Search {
    pub n: usize,
    pub budget: usize,
    pub best: f64,
    pub best_seq: Seq,
    pub curve: Vec<(usize, f64)>,
    pub reuse: Reuse,
}

impl Search {
    fn add(&mut self, score: f64, seq: &Seq) {
        self.n += 1;
        if score > self.best {
            self.best = score;
            self.best_seq = seq.clone();
            self.curve.push((self.n, score));
        }
    }
    fn done(&self) -> bool {
        self.n >= self.budget
    }
}

/// Level-`level` NRPA, one rollout per step, with the incumbent as prior.
#[allow(clippy::too_many_arguments)]
pub fn nrpa(p: &Planner, st0: &State, turn: Horizon, budget: usize, level: u32, alpha: f64, bias: f64, seed: u64,
            init: Option<&HashMap<Key, Choice>>, fixed: Option<&HashMap<Key, Choice>>) -> Search {
    let mut rng = Rng::new(seed);
    let mut s = Search { n: 0, budget, best: f64::MIN, best_seq: vec![], curve: vec![], reuse: Reuse::new(reuse_cap()) };
    let iters = ((budget as f64).powf(1.0 / level as f64)).round().max(2.0) as usize;
    fn run(p: &Planner, st0: &State, turn: Horizon, lv: u32, policy: Policy, iters: usize, alpha: f64, bias: f64, rng: &mut Rng,
           init: Option<&HashMap<Key, Choice>>, fixed: Option<&HashMap<Key, Choice>>, s: &mut Search) -> (f64, Seq) {
        if lv == 0 || s.done() {
            let (score, seq) = policy_rollout(p, st0, turn, &policy, bias, rng, init, fixed, &mut s.reuse);
            s.add(score, &seq);
            return (score, seq);
        }
        let mut best = (f64::MIN, vec![]);
        let mut policy = policy;
        for _ in 0..iters {
            if s.done() {
                break;
            }
            let r = run(p, st0, turn, lv - 1, policy.clone(), iters, alpha, bias, rng, init, fixed, s);
            if r.0 >= best.0 {
                best = r;
            }
            policy = adapt(&policy, &best.1, alpha);
        }
        best
    }
    while !s.done() {
        run(p, st0, turn, level, Policy::new(), iters, alpha, bias, &mut rng, init, fixed, &mut s);
    }
    s
}
