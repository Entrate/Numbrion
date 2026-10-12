//! A small JSON reader for `dex.json` and Showdown request JSON (the crate has no dependencies).
//!
//! Values follow Python's `json.loads`: an object keeps its keys in source order and a repeated key resolves to its
//! last value; [`Json::truthy`] is Python's `bool()` of the loaded value.

use std::borrow::Cow;

#[derive(Clone, Debug, PartialEq)]
pub enum Json<'a> {
    Null,
    Bool(bool),
    Num(f64),
    Str(Cow<'a, str>),
    Arr(Vec<Json<'a>>),
    Obj(Vec<(Cow<'a, str>, Json<'a>)>),
}

static NULL: Json<'static> = Json::Null;

impl<'a> Json<'a> {
    /// Borrow unescaped strings and object keys; only escape decoding allocates string storage.
    pub fn parse(text: &'a str) -> Result<Json<'a>, String> {
        let mut p = Parser { text, s: text.as_bytes(), i: 0, members: Vec::with_capacity(64), items: Vec::with_capacity(32) };
        p.ws();
        let v = p.value(0)?;
        p.ws();
        if p.i != p.s.len() {
            return Err(p.err("trailing characters"));
        }
        Ok(v)
    }

    /// Object member `key` (the last one if repeated); None for a missing key or a non-object.
    pub fn get(&self, key: &str) -> Option<&Json<'a>> {
        match self {
            Json::Obj(members) => members.iter().rev().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    /// `get(key)`, with a missing member read as `null` (Python's `d.get(key)`).
    pub fn at(&self, key: &str) -> &Json<'a> {
        self.get(key).unwrap_or(&NULL)
    }

    /// Python truthiness of the value: false, null, 0, "", [] and {} are false.
    pub fn truthy(&self) -> bool {
        match self {
            Json::Null => false,
            Json::Bool(b) => *b,
            Json::Num(n) => *n != 0.0,
            Json::Str(s) => !s.is_empty(),
            Json::Arr(a) => !a.is_empty(),
            Json::Obj(o) => !o.is_empty(),
        }
    }

    pub fn is_null(&self) -> bool {
        matches!(self, Json::Null)
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    /// The string, or "" for anything else (Python's `d.get(key, "")` on string fields).
    pub fn str_or_empty(&self) -> &str {
        self.as_str().unwrap_or("")
    }

    /// Numbers, with booleans as 0 / 1 like Python arithmetic.
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            Json::Bool(b) => Some(f64::from(u8::from(*b))),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Json::Bool(b) => Some(*b),
            _ => None,
        }
    }

    /// Array items; empty for anything else.
    pub fn items(&self) -> &[Json<'a>] {
        match self {
            Json::Arr(a) => a,
            _ => &[],
        }
    }

    /// Object members in source order (repeated keys included); empty for anything else.
    pub fn members(&self) -> &[(Cow<'a, str>, Json<'a>)] {
        match self {
            Json::Obj(o) => o,
            _ => &[],
        }
    }

    /// Object members with repeated keys resolved like a Python dict: the first position, the last value.
    pub fn dict(&self) -> Vec<(&str, &Json<'a>)> {
        let mut out: Vec<(&str, &Json<'a>)> = Vec::with_capacity(self.members().len());
        for (k, v) in self.members() {
            match out.iter_mut().find(|(key, _)| *key == k.as_ref()) {
                Some(slot) => slot.1 = v,
                None => out.push((k.as_ref(), v)),
            }
        }
        out
    }
}

const MAX_DEPTH: usize = 256;

struct Parser<'a> {
    text: &'a str,
    s: &'a [u8],
    i: usize,
    /// Stacks of the members / items of the containers being parsed: a finished container moves its own entries
    /// into one exactly sized vector (one allocation per container, however many entries it has).
    members: Vec<(Cow<'a, str>, Json<'a>)>,
    items: Vec<Json<'a>>,
}

impl<'a> Parser<'a> {
    fn err(&self, what: &str) -> String {
        format!("invalid JSON at byte {}: {what}", self.i)
    }

    fn ws(&mut self) {
        let rest = &self.s[self.i.min(self.s.len())..];
        self.i += rest.iter().position(|c| !matches!(c, b' ' | b'\t' | b'\n' | b'\r')).unwrap_or(rest.len());
    }

    fn eat(&mut self, lit: &[u8]) -> bool {
        if self.s[self.i..].starts_with(lit) {
            self.i += lit.len();
            true
        } else {
            false
        }
    }

    fn value(&mut self, depth: usize) -> Result<Json<'a>, String> {
        if depth > MAX_DEPTH {
            return Err(self.err("nested too deeply"));
        }
        match self.s.get(self.i) {
            None => Err(self.err("unexpected end")),
            Some(b'{') => {
                self.i += 1;
                self.ws();
                if self.eat(b"}") {
                    return Ok(Json::Obj(Vec::new()));
                }
                let start = self.members.len();
                loop {
                    self.ws();
                    if self.s.get(self.i) != Some(&b'"') {
                        return Err(self.err("expected a key"));
                    }
                    let key = self.string()?;
                    self.ws();
                    if !self.eat(b":") {
                        return Err(self.err("expected ':'"));
                    }
                    self.ws();
                    let v = self.value(depth + 1)?;
                    self.members.push((key, v));
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"}") {
                        return Ok(Json::Obj(self.members.drain(start..).collect()));
                    }
                    return Err(self.err("expected ',' or '}'"));
                }
            }
            Some(b'[') => {
                self.i += 1;
                self.ws();
                if self.eat(b"]") {
                    return Ok(Json::Arr(Vec::new()));
                }
                let start = self.items.len();
                loop {
                    self.ws();
                    let v = self.value(depth + 1)?;
                    self.items.push(v);
                    self.ws();
                    if self.eat(b",") {
                        continue;
                    }
                    if self.eat(b"]") {
                        return Ok(Json::Arr(self.items.drain(start..).collect()));
                    }
                    return Err(self.err("expected ',' or ']'"));
                }
            }
            Some(b'"') => Ok(Json::Str(self.string()?)),
            Some(b't') if self.eat(b"true") => Ok(Json::Bool(true)),
            Some(b'f') if self.eat(b"false") => Ok(Json::Bool(false)),
            Some(b'n') if self.eat(b"null") => Ok(Json::Null),
            // Python's json also reads these non-standard constants.
            Some(b'N') if self.eat(b"NaN") => Ok(Json::Num(f64::NAN)),
            Some(b'I') if self.eat(b"Infinity") => Ok(Json::Num(f64::INFINITY)),
            Some(b'-') if self.s[self.i..].starts_with(b"-Infinity") => {
                self.i += 9;
                Ok(Json::Num(f64::NEG_INFINITY))
            }
            Some(c) if *c == b'-' || c.is_ascii_digit() => self.number(),
            Some(_) => Err(self.err("unexpected character")),
        }
    }

    fn number(&mut self) -> Result<Json<'a>, String> {
        let start = self.i;
        while let Some(&c) = self.s.get(self.i) {
            if c.is_ascii_digit() || matches!(c, b'-' | b'+' | b'.' | b'e' | b'E') {
                self.i += 1;
            } else {
                break;
            }
        }
        let bytes = &self.s[start..self.i];
        // Plain integers below 10^15 are exact in f64: the same value `parse` gives (including -0.0).
        let (negative, digits) = match bytes.split_first() {
            Some((b'-', rest)) => (true, rest),
            _ => (false, bytes),
        };
        if !digits.is_empty() && digits.len() <= 15 && digits.iter().all(u8::is_ascii_digit) {
            let v = digits.iter().fold(0u64, |v, &d| v * 10 + u64::from(d - b'0')) as f64;
            return Ok(Json::Num(if negative { -v } else { v }));
        }
        let text = std::str::from_utf8(bytes).map_err(|_| self.err("bad number"))?;
        text.parse::<f64>().map(Json::Num).map_err(|_| self.err("bad number"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let digits = self.s.get(self.i..self.i + 4).ok_or_else(|| self.err("short \\u escape"))?;
        let text = std::str::from_utf8(digits).map_err(|_| self.err("bad \\u escape"))?;
        let v = u32::from_str_radix(text, 16).map_err(|_| self.err("bad \\u escape"))?;
        self.i += 4;
        Ok(v)
    }

    /// A string starting at the opening quote.
    fn string(&mut self) -> Result<Cow<'a, str>, String> {
        self.i += 1;
        let string_start = self.i;
        let mut decoded: Option<String> = None;
        loop {
            // ASCII quotes/backslashes cannot occur inside a UTF-8 code point. Both ends of each
            // run are therefore character boundaries in the original &str, even for non-ASCII names.
            let start = self.i;
            let rest = &self.s[start.min(self.s.len())..];
            self.i += rest.iter().position(|&c| c == b'"' || c == b'\\').unwrap_or(rest.len());
            if let Some(out) = &mut decoded {
                out.push_str(&self.text[start..self.i]);
            }
            match self.s.get(self.i) {
                None => return Err(self.err("unterminated string")),
                Some(b'"') => {
                    let text = &self.text[string_start..self.i];
                    self.i += 1;
                    return Ok(match decoded {
                        Some(out) => Cow::Owned(out),
                        None => Cow::Borrowed(text),
                    });
                }
                Some(_) => {
                    let out = decoded.get_or_insert_with(|| self.text[string_start..self.i].to_owned());
                    self.i += 1;
                    let Some(&e) = self.s.get(self.i) else { return Err(self.err("unterminated escape")) };
                    self.i += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            let mut code = self.hex4()?;
                            if (0xD800..0xDC00).contains(&code) && self.s[self.i..].starts_with(b"\\u") {
                                let save = self.i;
                                self.i += 2;
                                let low = self.hex4()?;
                                if (0xDC00..0xE000).contains(&low) {
                                    code = 0x10000 + ((code - 0xD800) << 10) + (low - 0xDC00);
                                } else {
                                    self.i = save;
                                }
                            }
                            out.push(char::from_u32(code).unwrap_or('\u{FFFD}'));
                        }
                        _ => return Err(self.err("bad escape")),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Json;
    use std::borrow::Cow;

    #[test]
    fn borrows_unescaped_keys_and_unicode_strings_from_input() {
        let text = String::from(r#"{"Pokémon😀": "Flabébé雪", "empty": ""}"#);
        let v = Json::parse(&text).unwrap();
        let (key, value) = &v.members()[0];
        assert!(matches!(key, Cow::Borrowed(_)));
        let Json::Str(Cow::Borrowed(s)) = value else { panic!("unescaped string was copied") };
        assert_eq!(*s, "Flabébé雪");
        let start = text.find(s).unwrap();
        assert_eq!(s.as_ptr(), text[start..].as_ptr());
        assert!(matches!(v.at("empty"), Json::Str(Cow::Borrowed(""))));
    }

    #[test]
    fn owns_only_escaped_strings_and_preserves_escape_decoding() {
        let v = Json::parse(r#"{"\u006b": "é\"\\\/\b\f\n\r\t\u96ea\uD83D\uDE00end", "plain": "雪"}"#).unwrap();
        assert!(matches!(&v.members()[0].0, Cow::Owned(_)));
        assert!(matches!(v.at("k"), Json::Str(Cow::Owned(_))));
        assert_eq!(v.at("k").as_str(), Some("é\"\\/\u{8}\u{c}\n\r\t雪😀end"));
        assert!(matches!(v.at("plain"), Json::Str(Cow::Borrowed(_))));
        // Preserve the existing replacement-character treatment of unpaired surrogates.
        let lone = Json::parse(r#"["\uD800", "\uDC00", "\uD800\u0061"]"#).unwrap();
        assert_eq!(lone.items().iter().map(Json::str_or_empty).collect::<Vec<_>>(), ["�", "�", "�a"]);
    }

    #[test]
    fn malformed_input_retains_clear_byte_errors() {
        for (text, error) in [
            ("", "invalid JSON at byte 0: unexpected end"),
            (r#""abc"#, "invalid JSON at byte 4: unterminated string"),
            ("\"a\\", "invalid JSON at byte 3: unterminated escape"),
            (r#""\x""#, "invalid JSON at byte 3: bad escape"),
            (r#""\u12""#, "invalid JSON at byte 3: short \\u escape"),
            (r#""\uZZZZ""#, "invalid JSON at byte 3: bad \\u escape"),
            (r#"{"a":}"#, "invalid JSON at byte 5: unexpected character"),
            ("[1,]", "invalid JSON at byte 3: unexpected character"),
            ("1e+", "invalid JSON at byte 3: bad number"),
            ("[1] x", "invalid JSON at byte 4: trailing characters"),
        ] {
            assert_eq!(Json::parse(text).unwrap_err(), error, "{text:?}");
        }
        let text = r#"{"雪😀": ["é\n\uD83D\uDE00", {"n": 1e9999}]}"#;
        for (end, _) in text.char_indices() {
            assert!(Json::parse(&text[..end]).is_err(), "truncated at {end}");
        }
        let deep = format!("{}0{}", "[".repeat(257), "]".repeat(257));
        assert!(Json::parse(&deep).unwrap_err().contains("nested too deeply"));
    }

    #[test]
    fn huge_numbers_and_python_nonfinite_constants_still_parse() {
        let text = format!("[{}, 1e9999, -1e9999, NaN, Infinity, -Infinity]", "9".repeat(400));
        let v = Json::parse(&text).unwrap();
        assert_eq!(v.items()[0].as_f64(), Some(f64::INFINITY));
        assert_eq!(v.items()[1].as_f64(), Some(f64::INFINITY));
        assert_eq!(v.items()[2].as_f64(), Some(f64::NEG_INFINITY));
        assert!(v.items()[3].as_f64().unwrap().is_nan());
        assert!(v.items()[3].truthy());
        assert_eq!(v.items()[4].as_f64(), Some(f64::INFINITY));
        assert_eq!(v.items()[5].as_f64(), Some(f64::NEG_INFINITY));
    }

    #[test]
    fn parses_requests_like_python() {
        let v = Json::parse(r#"{"a": [1, 2.5, -3e2, true, null], "b": {"c": "xé😀\n"}, "a": "last"}"#).unwrap();
        assert_eq!(v.at("a").as_str(), Some("last"));
        assert_eq!(v.at("b").at("c").as_str(), Some("xé😀\n"));
        assert!(v.at("missing").is_null());
        assert_eq!(v.dict().len(), 2);
        let arr = Json::parse("[1, 2.5, -3e2, true, null, \"\", {}]").unwrap();
        let t: Vec<bool> = arr.items().iter().map(Json::truthy).collect();
        assert_eq!(t, [true, true, true, true, false, false, false]);
        assert_eq!(arr.items()[2].as_f64(), Some(-300.0));
        assert!(Json::parse("{\"a\": 1,}").is_err());
        assert!(Json::parse("[1] x").is_err());
    }
}
