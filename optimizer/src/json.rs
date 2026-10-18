//! A small JSON value, parser and writer (no dependencies: the build needs no C toolchain). Objects keep their key
//! order, like the Python dicts they were written from.
use std::fmt::Write;

#[derive(Clone, Debug, PartialEq, Default)]
pub enum Value {
    #[default]
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Value>),
    Obj(Vec<(String, Value)>),
}

static NULL: Value = Value::Null;

impl Value {
    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self { Some(s) } else { None }
    }
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Value::Num(n) => Some(*n),
            Value::Bool(b) => Some(*b as i64 as f64),
            _ => None,
        }
    }
    pub fn as_i64(&self) -> Option<i64> {
        match self {
            Value::Num(n) if n.fract() == 0.0 => Some(*n as i64),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }
    pub fn as_u64(&self) -> Option<u64> {
        self.as_i64().filter(|x| *x >= 0).map(|x| x as u64)
    }
    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self { Some(*b) } else { None }
    }
    pub fn as_array(&self) -> Option<&Vec<Value>> {
        if let Value::Arr(a) = self { Some(a) } else { None }
    }
    pub fn as_object(&self) -> Option<&Vec<(String, Value)>> {
        if let Value::Obj(o) = self { Some(o) } else { None }
    }
    pub fn is_number(&self) -> bool {
        matches!(self, Value::Num(_))
    }
    pub fn is_object(&self) -> bool {
        matches!(self, Value::Obj(_))
    }
    pub fn is_null(&self) -> bool {
        matches!(self, Value::Null)
    }
    pub fn get(&self, k: &str) -> Option<&Value> {
        self.as_object().and_then(|o| o.iter().find(|(a, _)| a == k).map(|(_, v)| v))
    }
    pub fn obj(items: Vec<(&str, Value)>) -> Value {
        Value::Obj(items.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
}

impl std::ops::Index<&str> for Value {
    type Output = Value;
    fn index(&self, k: &str) -> &Value {
        self.get(k).unwrap_or(&NULL)
    }
}
impl std::ops::Index<usize> for Value {
    type Output = Value;
    fn index(&self, i: usize) -> &Value {
        self.as_array().and_then(|a| a.get(i)).unwrap_or(&NULL)
    }
}

impl From<f64> for Value {
    fn from(x: f64) -> Value {
        Value::Num(x)
    }
}
impl From<i64> for Value {
    fn from(x: i64) -> Value {
        Value::Num(x as f64)
    }
}
impl From<i32> for Value {
    fn from(x: i32) -> Value {
        Value::Num(x as f64)
    }
}
impl From<usize> for Value {
    fn from(x: usize) -> Value {
        Value::Num(x as f64)
    }
}
impl From<bool> for Value {
    fn from(x: bool) -> Value {
        Value::Bool(x)
    }
}
impl From<&str> for Value {
    fn from(x: &str) -> Value {
        Value::Str(x.to_string())
    }
}
impl From<String> for Value {
    fn from(x: String) -> Value {
        Value::Str(x)
    }
}
impl<T: Into<Value>> From<Vec<T>> for Value {
    fn from(x: Vec<T>) -> Value {
        Value::Arr(x.into_iter().map(Into::into).collect())
    }
}
impl<T: Into<Value>> From<Option<T>> for Value {
    fn from(x: Option<T>) -> Value {
        x.map(Into::into).unwrap_or(Value::Null)
    }
}

// ---------------------------------------------------------------- parse
struct P<'a> {
    b: &'a [u8],
    i: usize,
}

impl P<'_> {
    fn ws(&mut self) {
        while self.i < self.b.len() && matches!(self.b[self.i], b' ' | b'\n' | b'\r' | b'\t') {
            self.i += 1;
        }
    }
    fn value(&mut self) -> Result<Value, String> {
        self.ws();
        let c = *self.b.get(self.i).ok_or("unexpected end")?;
        match c {
            b'{' => {
                self.i += 1;
                let mut o = vec![];
                self.ws();
                if self.b[self.i] == b'}' {
                    self.i += 1;
                    return Ok(Value::Obj(o));
                }
                loop {
                    self.ws();
                    let k = self.string()?;
                    self.ws();
                    if self.b[self.i] != b':' {
                        return Err(format!("expected : at {}", self.i));
                    }
                    self.i += 1;
                    let v = self.value()?;
                    o.push((k, v));
                    self.ws();
                    match self.b[self.i] {
                        b',' => self.i += 1,
                        b'}' => {
                            self.i += 1;
                            return Ok(Value::Obj(o));
                        }
                        _ => return Err(format!("expected , or }} at {}", self.i)),
                    }
                }
            }
            b'[' => {
                self.i += 1;
                let mut a = vec![];
                self.ws();
                if self.b[self.i] == b']' {
                    self.i += 1;
                    return Ok(Value::Arr(a));
                }
                loop {
                    a.push(self.value()?);
                    self.ws();
                    match self.b[self.i] {
                        b',' => self.i += 1,
                        b']' => {
                            self.i += 1;
                            return Ok(Value::Arr(a));
                        }
                        _ => return Err(format!("expected , or ] at {}", self.i)),
                    }
                }
            }
            b'"' => Ok(Value::Str(self.string()?)),
            b't' => {
                self.i += 4;
                Ok(Value::Bool(true))
            }
            b'f' => {
                self.i += 5;
                Ok(Value::Bool(false))
            }
            b'n' => {
                self.i += 4;
                Ok(Value::Null)
            }
            b'N' => {
                self.i += 3; // NaN (Python's json writes it)
                Ok(Value::Num(f64::NAN))
            }
            b'I' => {
                self.i += 8;
                Ok(Value::Num(f64::INFINITY))
            }
            _ => {
                let s = self.i;
                if self.b[self.i] == b'-' && self.b.get(self.i + 1) == Some(&b'I') {
                    self.i += 9;
                    return Ok(Value::Num(f64::NEG_INFINITY));
                }
                while self.i < self.b.len() && matches!(self.b[self.i], b'0'..=b'9' | b'-' | b'+' | b'.' | b'e' | b'E') {
                    self.i += 1;
                }
                let t = std::str::from_utf8(&self.b[s..self.i]).unwrap();
                t.parse::<f64>().map(Value::Num).map_err(|e| format!("bad number {t}: {e}"))
            }
        }
    }
    fn string(&mut self) -> Result<String, String> {
        if self.b[self.i] != b'"' {
            return Err(format!("expected string at {}", self.i));
        }
        self.i += 1;
        let mut out = String::new();
        loop {
            let c = self.b[self.i];
            match c {
                b'"' => {
                    self.i += 1;
                    return Ok(out);
                }
                b'\\' => {
                    let e = self.b[self.i + 1];
                    self.i += 2;
                    match e {
                        b'n' => out.push('\n'),
                        b't' => out.push('\t'),
                        b'r' => out.push('\r'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'u' => {
                            let h = std::str::from_utf8(&self.b[self.i..self.i + 4]).unwrap();
                            let mut cp = u32::from_str_radix(h, 16).unwrap();
                            self.i += 4;
                            if (0xD800..0xDC00).contains(&cp) && self.b.get(self.i) == Some(&b'\\') {
                                let h2 = std::str::from_utf8(&self.b[self.i + 2..self.i + 6]).unwrap();
                                let lo = u32::from_str_radix(h2, 16).unwrap();
                                cp = 0x10000 + ((cp - 0xD800) << 10) + (lo - 0xDC00);
                                self.i += 6;
                            }
                            out.push(char::from_u32(cp).unwrap_or('?'));
                        }
                        other => out.push(other as char),
                    }
                }
                _ => {
                    // copy a run of plain bytes (UTF-8 kept as is)
                    let s = self.i;
                    while self.i < self.b.len() && self.b[self.i] != b'"' && self.b[self.i] != b'\\' {
                        self.i += 1;
                    }
                    out.push_str(std::str::from_utf8(&self.b[s..self.i]).unwrap());
                }
            }
        }
    }
}

pub fn parse(s: &str) -> Result<Value, String> {
    let mut p = P { b: s.as_bytes(), i: 0 };
    p.value()
}

// ---------------------------------------------------------------- write
fn esc(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

fn write_value(out: &mut String, v: &Value) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Num(n) => {
            if n.is_finite() && n.fract() == 0.0 && n.abs() < 1e15 {
                let _ = write!(out, "{}", *n as i64);
            } else if n.is_finite() {
                let _ = write!(out, "{}", n);
            } else {
                out.push_str("null");
            }
        }
        Value::Str(s) => esc(out, s),
        Value::Arr(a) => {
            out.push('[');
            for (i, x) in a.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_value(out, x);
            }
            out.push(']');
        }
        Value::Obj(o) => {
            out.push('{');
            for (i, (k, x)) in o.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                esc(out, k);
                out.push(':');
                write_value(out, x);
            }
            out.push('}');
        }
    }
}

impl std::fmt::Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut s = String::new();
        write_value(&mut s, self);
        f.write_str(&s)
    }
}
