//! `when` clauses: a small boolean expression language evaluated against a [`Context`]
//! snapshot.
//!
//! ```text
//! expr   := or
//! or     := and ( "||" and )*
//! and    := not ( "&&" not )*
//! not    := "!" not | atom
//! atom   := "(" expr ")" | key | key ("==" | "!=") value
//! key    := [a-zA-Z_][a-zA-Z0-9_.-]*
//! value  := 'quoted' | true | false | number
//! ```

use std::collections::BTreeMap;
use std::fmt;

use thiserror::Error;

/// A context value. Bare keys are truthy for `Bool(true)`, non-empty strings and non-zero
/// numbers.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Str(String),
    Num(f64),
}

impl Value {
    fn truthy(&self) -> bool {
        match self {
            Value::Bool(b) => *b,
            Value::Str(s) => !s.is_empty(),
            Value::Num(n) => *n != 0.0,
        }
    }
}

impl From<bool> for Value {
    fn from(value: bool) -> Self {
        Value::Bool(value)
    }
}

impl From<&str> for Value {
    fn from(value: &str) -> Self {
        Value::Str(value.to_string())
    }
}

impl From<String> for Value {
    fn from(value: String) -> Self {
        Value::Str(value)
    }
}

impl From<f64> for Value {
    fn from(value: f64) -> Self {
        Value::Num(value)
    }
}

impl From<i64> for Value {
    fn from(value: i64) -> Self {
        Value::Num(value as f64)
    }
}

impl fmt::Display for Value {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bool(b) => write!(f, "{b}"),
            Value::Num(n) => {
                if n.fract() == 0.0 && n.abs() < 1e15 {
                    write!(f, "{}", *n as i64)
                } else {
                    write!(f, "{n}")
                }
            }
            Value::Str(s) => {
                f.write_str("'")?;
                for c in s.chars() {
                    if c == '\'' || c == '\\' {
                        write!(f, "\\{c}")?;
                    } else {
                        write!(f, "{c}")?;
                    }
                }
                f.write_str("'")
            }
        }
    }
}

/// Snapshot of context keys. Unknown keys read as `false`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Context {
    values: BTreeMap<String, Value>,
}

impl Context {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<Value>) {
        self.values.insert(key.into(), value.into());
    }

    pub fn get(&self, key: &str) -> Option<&Value> {
        self.values.get(key)
    }

    pub fn clear(&mut self, key: &str) -> Option<Value> {
        self.values.remove(key)
    }

    pub fn iter(&self) -> impl Iterator<Item = (&str, &Value)> {
        self.values.iter().map(|(k, v)| (k.as_str(), v))
    }
}

/// Parsed `when` clause.
#[derive(Clone, Debug, PartialEq)]
pub enum When {
    Key(String),
    Eq(String, Value),
    Ne(String, Value),
    Not(Box<When>),
    /// Two or more conjuncts.
    And(Vec<When>),
    /// Two or more disjuncts.
    Or(Vec<When>),
}

#[derive(Clone, Debug, PartialEq, Eq, Error)]
pub enum WhenErrorKind {
    #[error("unexpected end of clause")]
    UnexpectedEnd,
    #[error("unexpected `{0}`")]
    Unexpected(char),
    #[error("expected a value after the comparison")]
    ExpectedValue,
    #[error("expected `)`")]
    ExpectedCloseParen,
    #[error("unterminated string")]
    UnterminatedString,
    #[error("invalid number")]
    InvalidNumber,
}

/// Parse failure with the byte offset it was detected at.
#[derive(Clone, Debug, PartialEq, Eq, Error)]
#[error("{kind} at offset {pos}")]
pub struct WhenError {
    pub pos: usize,
    pub kind: WhenErrorKind,
}

impl When {
    pub fn parse(source: &str) -> Result<When, WhenError> {
        let mut parser = Parser {
            src: source,
            pos: 0,
        };
        parser.skip_ws();
        let expr = parser.or()?;
        parser.skip_ws();
        match parser.peek() {
            None => Ok(expr),
            Some(c) => Err(parser.error(WhenErrorKind::Unexpected(c))),
        }
    }

    pub fn eval(&self, ctx: &Context) -> bool {
        match self {
            When::Key(key) => ctx.get(key).is_some_and(Value::truthy),
            When::Eq(key, value) => ctx.get(key).is_some_and(|actual| actual == value),
            When::Ne(key, value) => ctx.get(key).is_some_and(|actual| actual != value),
            When::Not(inner) => !inner.eval(ctx),
            When::And(parts) => parts.iter().all(|part| part.eval(ctx)),
            When::Or(parts) => parts.iter().any(|part| part.eval(ctx)),
        }
    }

    /// Conservative overlap test: `false` only when the two clauses provably cannot hold in
    /// the same context (a top-level conjunct of one contradicts a conjunct of the other,
    /// e.g. `k == 'a'` against `k == 'b'`, `k` against `!k`).
    pub fn compatible(&self, other: &When) -> bool {
        let mut mine = Vec::new();
        self.facts(&mut mine);
        let mut theirs = Vec::new();
        other.facts(&mut theirs);
        !mine.iter().any(|a| theirs.iter().any(|b| a.contradicts(b)))
    }

    fn facts<'a>(&'a self, out: &mut Vec<Fact<'a>>) {
        match self {
            When::And(parts) => parts.iter().for_each(|part| part.facts(out)),
            When::Key(key) => out.push(Fact::Truthy(key)),
            When::Eq(key, value) => out.push(Fact::Eq(key, value)),
            When::Ne(key, value) => out.push(Fact::Ne(key, value)),
            When::Not(inner) => match inner.as_ref() {
                When::Key(key) => out.push(Fact::Falsy(key)),
                When::Eq(key, value) => out.push(Fact::Ne(key, value)),
                When::Ne(key, value) => out.push(Fact::Eq(key, value)),
                _ => {}
            },
            When::Or(_) => {}
        }
    }

    fn fmt_prec(&self, f: &mut fmt::Formatter<'_>, prec: u8) -> fmt::Result {
        let (level, parts, separator) = match self {
            When::Or(parts) => (0, parts, " || "),
            When::And(parts) => (1, parts, " && "),
            When::Not(inner) => {
                f.write_str("!")?;
                return inner.fmt_prec(f, 2);
            }
            When::Key(key) => return f.write_str(key),
            When::Eq(key, value) => return write!(f, "{key} == {value}"),
            When::Ne(key, value) => return write!(f, "{key} != {value}"),
        };
        let wrap = prec > level;
        if wrap {
            f.write_str("(")?;
        }
        for (index, part) in parts.iter().enumerate() {
            if index > 0 {
                f.write_str(separator)?;
            }
            part.fmt_prec(f, level + 1)?;
        }
        if wrap {
            f.write_str(")")?;
        }
        Ok(())
    }
}

impl fmt::Display for When {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.fmt_prec(f, 0)
    }
}

enum Fact<'a> {
    Truthy(&'a str),
    Falsy(&'a str),
    Eq(&'a str, &'a Value),
    Ne(&'a str, &'a Value),
}

impl Fact<'_> {
    fn contradicts(&self, other: &Fact<'_>) -> bool {
        match (self, other) {
            (Fact::Truthy(a), Fact::Falsy(b)) | (Fact::Falsy(a), Fact::Truthy(b)) => a == b,
            (Fact::Eq(a, x), Fact::Eq(b, y)) => a == b && x != y,
            (Fact::Eq(a, x), Fact::Ne(b, y)) | (Fact::Ne(a, x), Fact::Eq(b, y)) => a == b && x == y,
            _ => false,
        }
    }
}

struct Parser<'a> {
    src: &'a str,
    pos: usize,
}

impl Parser<'_> {
    fn error(&self, kind: WhenErrorKind) -> WhenError {
        WhenError {
            pos: self.pos,
            kind,
        }
    }

    fn peek(&self) -> Option<char> {
        self.src[self.pos..].chars().next()
    }

    fn rest(&self) -> &str {
        &self.src[self.pos..]
    }

    fn skip_ws(&mut self) {
        while let Some(c) = self.peek().filter(|c| c.is_whitespace()) {
            self.pos += c.len_utf8();
        }
    }

    fn eat(&mut self, token: &str) -> bool {
        if self.rest().starts_with(token) {
            self.pos += token.len();
            true
        } else {
            false
        }
    }

    fn or(&mut self) -> Result<When, WhenError> {
        let mut parts = vec![self.and()?];
        loop {
            self.skip_ws();
            if !self.eat("||") {
                break;
            }
            self.skip_ws();
            parts.push(self.and()?);
        }
        Ok(flatten(parts, When::Or))
    }

    fn and(&mut self) -> Result<When, WhenError> {
        let mut parts = vec![self.not()?];
        loop {
            self.skip_ws();
            if !self.eat("&&") {
                break;
            }
            self.skip_ws();
            parts.push(self.not()?);
        }
        Ok(flatten(parts, When::And))
    }

    fn not(&mut self) -> Result<When, WhenError> {
        self.skip_ws();
        if self.eat("!") {
            return Ok(When::Not(Box::new(self.not()?)));
        }
        self.atom()
    }

    fn atom(&mut self) -> Result<When, WhenError> {
        self.skip_ws();
        if self.eat("(") {
            let inner = self.or()?;
            self.skip_ws();
            if !self.eat(")") {
                return Err(self.error(WhenErrorKind::ExpectedCloseParen));
            }
            return Ok(inner);
        }
        let key = self.key()?;
        self.skip_ws();
        if self.eat("==") {
            self.skip_ws();
            return Ok(When::Eq(key, self.value()?));
        }
        if self.eat("!=") {
            self.skip_ws();
            return Ok(When::Ne(key, self.value()?));
        }
        Ok(When::Key(key))
    }

    fn key(&mut self) -> Result<String, WhenError> {
        let start = self.pos;
        match self.peek() {
            None => return Err(self.error(WhenErrorKind::UnexpectedEnd)),
            Some(c) if c.is_ascii_alphabetic() || c == '_' => self.pos += 1,
            Some(c) => return Err(self.error(WhenErrorKind::Unexpected(c))),
        }
        while let Some(c) = self
            .peek()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '.' || *c == '-')
        {
            self.pos += c.len_utf8();
        }
        Ok(self.src[start..self.pos].to_string())
    }

    fn value(&mut self) -> Result<Value, WhenError> {
        if self.eat("'") {
            let mut text = String::new();
            loop {
                let Some(c) = self.peek() else {
                    return Err(self.error(WhenErrorKind::UnterminatedString));
                };
                self.pos += c.len_utf8();
                match c {
                    '\'' => return Ok(Value::Str(text)),
                    '\\' => {
                        let Some(escaped) = self.peek() else {
                            return Err(self.error(WhenErrorKind::UnterminatedString));
                        };
                        self.pos += escaped.len_utf8();
                        text.push(escaped);
                    }
                    other => text.push(other),
                }
            }
        }
        if self.eat_word("true") {
            return Ok(Value::Bool(true));
        }
        if self.eat_word("false") {
            return Ok(Value::Bool(false));
        }
        let start = self.pos;
        if self.peek() == Some('-') {
            self.pos += 1;
        }
        let mut digits = 0;
        while let Some(c) = self.peek().filter(|c| c.is_ascii_digit() || *c == '.') {
            self.pos += c.len_utf8();
            digits += 1;
        }
        if digits == 0 {
            self.pos = start;
            return Err(self.error(WhenErrorKind::ExpectedValue));
        }
        match self.src[start..self.pos].parse::<f64>() {
            Ok(number) => Ok(Value::Num(number)),
            Err(_) => {
                self.pos = start;
                Err(self.error(WhenErrorKind::InvalidNumber))
            }
        }
    }

    /// Consumes `word` only when it is not a prefix of a longer identifier.
    fn eat_word(&mut self, word: &str) -> bool {
        let rest = self.rest();
        if !rest.starts_with(word) {
            return false;
        }
        let follows_identifier = rest[word.len()..]
            .chars()
            .next()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '.' || c == '-');
        if follows_identifier {
            return false;
        }
        self.pos += word.len();
        true
    }
}

fn flatten(mut parts: Vec<When>, build: fn(Vec<When>) -> When) -> When {
    if parts.len() == 1 {
        parts.pop().expect("one part")
    } else {
        build(parts)
    }
}
