#[macro_use]
pub mod sym;
pub mod alloc;
pub mod amenities;
pub mod consumed;
pub mod engine;
pub mod gamedata;
pub mod geo;
pub mod gp;
pub mod growth;
pub mod json;
pub mod learn;
pub mod model;
pub mod opt;
pub mod placement;
pub mod planner;
pub mod production;
pub mod record;
pub mod rules;
pub mod save;
pub mod scenario;
pub mod sim;
pub mod strategic;
pub mod skeleton;
pub mod milestones;
pub mod value;
pub mod world;
pub mod yields;

/// Settings read from the environment once (reading it takes a process-wide lock, which threads contend for on
/// Windows): CIV6OPT_TRACE (trace output), CIV6OPT_ALL_EFFECTS (keep every modifier).
pub mod envcfg {
    pub fn trace() -> Option<&'static str> {
        static T: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();
        T.get_or_init(|| std::env::var("CIV6OPT_TRACE").ok()).as_deref()
    }
    /// The trace is for this city (CIV6OPT_TRACE=CITY:...).
    pub fn trace_city(city: &str) -> bool {
        trace().map(|v| v.len() > city.len() && v.starts_with(city) && v.as_bytes()[city.len()] == b':').unwrap_or(false)
    }
    pub fn all_effects() -> bool {
        static T: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *T.get_or_init(|| std::env::var_os("CIV6OPT_ALL_EFFECTS").is_some())
    }
}

/// Coarse timers for profiling (CIV6OPT_PROFILE=1 prints them after a run).
pub mod prof {
    use std::sync::atomic::{AtomicU64, Ordering};
    pub static T: [AtomicU64; 33] = [const { AtomicU64::new(0) }; 33];
    pub const NAMES: [&str; 33] = ["engine_new", "city_yields", "assign", "buildable", "builder_opts", "step", "amenity", "settle_sites", "route_opts", "site_possible", "grants", "other", "engine_key", "player_env",
        "learn_decision", "learn_option", "learn_predict", "advance",
        "turn_start", "turn_citizens", "turn_chops", "turn_prod_choice", "turn_builders", "turn_purchase", "turn_routes", "turn_step", "turn_end", "turn_borders",
        "b_idle_mark", "b_decide", "b_travel", "b_apply", "b_reassign"];
    pub struct Guard(usize, std::time::Instant);
    pub fn start(i: usize) -> Guard {
        Guard(i, std::time::Instant::now())
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            T[self.0].fetch_add(self.1.elapsed().as_nanos() as u64, Ordering::Relaxed);
        }
    }
    /// Adds the time since `t` to timer `i` and restarts `t` (the sections of one long function in turn).
    pub fn lap(t: &mut std::time::Instant, i: usize) {
        let now = std::time::Instant::now();
        T[i].fetch_add((now - *t).as_nanos() as u64, Ordering::Relaxed);
        *t = now;
    }
    pub fn report() -> String {
        NAMES.iter().enumerate().map(|(i, n)| format!("{n} {:.3}s", T[i].load(Ordering::Relaxed) as f64 / 1e9)).collect::<Vec<_>>().join(", ")
    }
}
