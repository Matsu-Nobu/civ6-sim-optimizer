#[macro_use]
pub mod sym;
/// FxHash (rustc's): a fast hash for keys the program makes itself (names of tables and columns, coordinates,
/// symbols); not for keys an outsider chooses, which could make every key collide.
pub mod fx {
    #[derive(Default, Clone, Copy)]
    pub struct FxHasher(u64);
    impl std::hash::Hasher for FxHasher {
        fn finish(&self) -> u64 {
            self.0
        }
        fn write(&mut self, bytes: &[u8]) {
            let mut chunks = bytes.chunks_exact(8);
            for c in &mut chunks {
                self.write_u64(u64::from_le_bytes(c.try_into().unwrap()));
            }
            for &b in chunks.remainder() {
                self.write_u64(b as u64);
            }
        }
        fn write_u8(&mut self, i: u8) {
            self.write_u64(i as u64)
        }
        fn write_u32(&mut self, i: u32) {
            self.write_u64(i as u64)
        }
        fn write_i32(&mut self, i: i32) {
            self.write_u64(i as u32 as u64)
        }
        fn write_i64(&mut self, i: i64) {
            self.write_u64(i as u64)
        }
        fn write_usize(&mut self, i: usize) {
            self.write_u64(i as u64)
        }
        fn write_u64(&mut self, i: u64) {
            self.0 = (self.0.rotate_left(5) ^ i).wrapping_mul(0x51_7c_c1_b7_27_22_0a_95);
        }
    }
    pub type FxBuild = std::hash::BuildHasherDefault<FxHasher>;
    pub type FxMap<K, V> = std::collections::HashMap<K, V, FxBuild>;
    pub type FxSet<K> = std::collections::HashSet<K, FxBuild>;
}
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
pub mod value_net;
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
    pub static T: [AtomicU64; 44] = [const { AtomicU64::new(0) }; 44];
    pub const NAMES: [&str; 44] = ["engine_new", "city_yields", "assign", "buildable", "builder_opts", "step", "amenity", "settle_sites", "route_opts", "site_possible", "grants", "other", "engine_key", "player_env",
        "learn_decision", "learn_option", "learn_predict", "advance",
        "turn_start", "turn_citizens", "turn_chops", "turn_prod_choice", "turn_builders", "turn_purchase", "turn_routes", "turn_step", "turn_end", "turn_borders",
        "b_idle_mark", "b_decide", "b_travel", "b_apply", "b_reassign",
        "a_borders", "a_choose", "a_builders", "a_settlers", "a_purchase", "p_items", "p_tiles", "x1", "x2", "x3", "x4"];
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
