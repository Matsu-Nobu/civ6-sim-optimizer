//! Interned strings: every type name (TECH_..., UNIT_..., city names) becomes a small copyable id.
use std::collections::HashMap;
use std::sync::{OnceLock, RwLock};

#[derive(Copy, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Debug, Default)]
pub struct Sym(pub u32);

struct Interner {
    ids: HashMap<String, u32>,
    names: Vec<String>,
}

fn interner() -> &'static RwLock<Interner> {
    static I: OnceLock<RwLock<Interner>> = OnceLock::new();
    I.get_or_init(|| RwLock::new(Interner { ids: HashMap::from([(String::new(), 0)]), names: vec![String::new()] }))
}

thread_local! {
    // per-thread copies, so threads don't contend on the global lock once names are known
    static LOCAL_IDS: std::cell::RefCell<HashMap<String, u32>> = std::cell::RefCell::new(HashMap::new());
    static LOCAL_NAMES: std::cell::RefCell<Vec<std::sync::Arc<str>>> = std::cell::RefCell::new(Vec::new());
}

/// Sym(0) is the empty string.
pub fn sym(s: &str) -> Sym {
    if let Some(i) = LOCAL_IDS.with(|m| m.borrow().get(s).copied()) {
        return Sym(i);
    }
    let i = {
        let r = interner().read().unwrap();
        r.ids.get(s).copied()
    };
    let i = match i {
        Some(i) => i,
        None => {
            let mut w = interner().write().unwrap();
            if let Some(&i) = w.ids.get(s) {
                i
            } else {
                let i = w.names.len() as u32;
                w.names.push(s.to_string());
                w.ids.insert(s.to_string(), i);
                i
            }
        }
    };
    LOCAL_IDS.with(|m| m.borrow_mut().insert(s.to_string(), i));
    Sym(i)
}

fn local_name(i: u32) -> std::sync::Arc<str> {
    LOCAL_NAMES.with(|v| {
        let mut v = v.borrow_mut();
        if (i as usize) >= v.len() {
            let r = interner().read().unwrap();
            for n in &r.names[v.len()..] {
                v.push(std::sync::Arc::from(n.as_str()));
            }
        }
        v[i as usize].clone()
    })
}

impl Sym {
    pub fn as_string(self) -> String {
        local_name(self.0).to_string()
    }
    pub fn with<R>(self, f: impl FnOnce(&str) -> R) -> R {
        let n = local_name(self.0);
        f(&n)
    }
    pub fn starts_with(self, p: &str) -> bool {
        self.with(|s| s.starts_with(p))
    }
    pub fn ends_with(self, p: &str) -> bool {
        self.with(|s| s.ends_with(p))
    }
    pub fn contains(self, p: &str) -> bool {
        self.with(|s| s.contains(p))
    }
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }
}

impl std::fmt::Display for Sym {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.with(|s| f.write_str(s))
    }
}

/// The Sym of a string literal, interned once per call site.
#[macro_export]
macro_rules! s {
    ($x:literal) => {{
        static C: std::sync::OnceLock<$crate::sym::Sym> = std::sync::OnceLock::new();
        *C.get_or_init(|| $crate::sym::sym($x))
    }};
}

thread_local! {
    static STATIC_CACHE: std::cell::RefCell<HashMap<usize, Sym>> = std::cell::RefCell::new(HashMap::new());
}

/// The Sym of a &'static str, cached by its address (no hashing of the text after the first time).
pub fn sym_static(x: &'static str) -> Sym {
    let key = x.as_ptr() as usize ^ (x.len() << 48);
    STATIC_CACHE.with(|c| {
        if let Some(&s) = c.borrow().get(&key) {
            return s;
        }
        let s = sym(x);
        c.borrow_mut().insert(key, s);
        s
    })
}
