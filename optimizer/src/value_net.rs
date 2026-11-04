//! The learned value of a state as a neural net that reads each city (trained by tools/value_net.py, saved with
//! --save): the predicted score of the play from the state on (the part already made plus what the base policy
//! makes from there).
//!
//! Every city's features (value::city_features) pass through one small net; the results' sum, mean and maximum over
//! the cities, the state's features (value::features and value::features_extra) and the turn pass through another,
//! which gives the score still to come, standardized within the turn.
use crate::json::Value;
use crate::model::State;
use crate::planner::Planner;

struct Layer {
    /// out × in, row by row
    w: Vec<f64>,
    b: Vec<f64>,
    inputs: usize,
}

impl Layer {
    fn read(v: &Value) -> Layer {
        let rows = v.get("w").and_then(|x| x.as_array()).expect("value net: a layer without w");
        let w: Vec<f64> = rows.iter().flat_map(|r| r.as_array().expect("value net: w row").iter().map(|x| x.as_f64().unwrap())).collect();
        let b: Vec<f64> = nums(v.get("b").expect("value net: a layer without b"));
        assert_eq!(w.len() % b.len(), 0, "value net: w does not fit b");
        Layer { inputs: w.len() / b.len(), w, b }
    }

    fn apply(&self, x: &[f64], relu: bool, out: &mut Vec<f64>) {
        assert_eq!(x.len(), self.inputs, "value net: input size");
        out.clear();
        for (o, b) in self.b.iter().enumerate() {
            let row = &self.w[o * self.inputs..(o + 1) * self.inputs];
            let v = b + row.iter().zip(x).map(|(a, b)| a * b).sum::<f64>();
            out.push(if relu { v.max(0.0) } else { v });
        }
    }
}

fn nums(v: &Value) -> Vec<f64> {
    v.as_array().expect("value net: a list of numbers").iter().map(|x| x.as_f64().unwrap()).collect()
}

pub struct ValueNet {
    made: usize,
    gx_mu: Vec<f64>,
    gx_sd: Vec<f64>,
    cx_mu: Vec<f64>,
    cx_sd: Vec<f64>,
    turn_mu: crate::fx::FxMap<i64, f64>,
    turn_sd: crate::fx::FxMap<i64, f64>,
    city: [Layer; 2],
    head: [Layer; 3],
}

impl ValueNet {
    pub fn load(path: &str) -> ValueNet {
        let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("value net {path}: {e}"));
        let v = crate::json::parse(&text).unwrap_or_else(|e| panic!("value net {path}: {e}"));
        let per_turn = |k: &str| -> crate::fx::FxMap<i64, f64> {
            v.get(k).and_then(|x| x.as_object()).expect("value net: per-turn statistics").iter()
                .map(|(t, x)| (t.parse().unwrap(), x.as_f64().unwrap())).collect()
        };
        let layers = |k: &str| -> Vec<Layer> { v.get(k).and_then(|x| x.as_array()).expect("value net: layers").iter().map(Layer::read).collect() };
        let mut city = layers("city").into_iter();
        let mut head = layers("head").into_iter();
        ValueNet {
            made: v.get("made").and_then(|x| x.as_u64()).expect("value net: made") as usize,
            gx_mu: nums(v.get("gx_mu").unwrap()),
            gx_sd: nums(v.get("gx_sd").unwrap()),
            cx_mu: nums(v.get("cx_mu").unwrap()),
            cx_sd: nums(v.get("cx_sd").unwrap()),
            turn_mu: per_turn("turn_mu"),
            turn_sd: per_turn("turn_sd"),
            city: [city.next().unwrap(), city.next().unwrap()],
            head: [head.next().unwrap(), head.next().unwrap(), head.next().unwrap()],
        }
    }

    /// The predicted score of the play from the state at the start of its turn.
    pub fn value(&self, p: &Planner, st: &State) -> f64 {
        let mut g: Vec<f64> = crate::value::features(p, st).to_vec();
        g.extend(crate::value::features_extra(p, st));
        self.value_of(st.turn, &g, &crate::value::city_features(p, st))
    }

    /// The value from the features as valuerows writes them (state's, then each city's).
    pub fn value_of(&self, turn: i64, g: &[f64], cities: &[Vec<f64>]) -> f64 {
        assert!(!cities.is_empty(), "value net: a state without cities");
        let width = self.city[1].b.len();
        let (mut sum, mut top) = (vec![0.0; width], vec![f64::MIN; width]);
        let (mut x, mut h1, mut h2) = (vec![], vec![], vec![]);
        for c in cities {
            x.clear();
            x.extend(c.iter().zip(self.cx_mu.iter().zip(&self.cx_sd)).map(|(v, (m, s))| (v - m) / s));
            self.city[0].apply(&x, true, &mut h1);
            self.city[1].apply(&h1, true, &mut h2);
            for i in 0..width {
                sum[i] += h2[i];
                top[i] = top[i].max(h2[i]);
            }
        }
        let n = cities.len() as f64;
        let mut v: Vec<f64> = sum.clone();
        v.extend(sum.iter().map(|s| s / n));
        v.extend(top);
        v.extend(g.iter().zip(self.gx_mu.iter().zip(&self.gx_sd)).map(|(v, (m, s))| (v - m) / s));
        v.push(turn as f64 / 65.0);
        self.head[0].apply(&v, true, &mut h1);
        self.head[1].apply(&h1, true, &mut h2);
        self.head[2].apply(&h2, false, &mut h1);
        let z = h1[0];
        z * self.turn_sd.get(&turn).copied().unwrap_or(1.0) + self.turn_mu.get(&turn).copied().unwrap_or(0.0) + g[self.made]
    }
}
