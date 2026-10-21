//! A tiny JSON value and serializer (no serde).
//!
//! Only what the bridge needs: objects keep insertion order, integers are
//! written exactly, and strings are escaped per RFC 8259 (quote, backslash,
//! and every control character below U+0020). Integers that may exceed
//! JavaScript's safe range (2^53 - 1), such as seeds and checksums, are
//! emitted as strings by the callers, never as numbers.

use std::fmt::Write as _;

/// A JSON value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Json {
    /// `null`
    Null,
    /// `true` / `false`
    Bool(bool),
    /// A signed integer. Callers keep it within ±(2^53 - 1).
    Int(i64),
    /// A string (escaped on output).
    Str(String),
    /// An array.
    Arr(Vec<Json>),
    /// An object; keys are written in this order.
    Obj(Vec<(&'static str, Json)>),
}

impl Json {
    /// A string value.
    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    /// An integer from any type losslessly convertible to `i64`.
    pub fn int(n: impl Into<i64>) -> Json {
        Json::Int(n.into())
    }

    /// `null` for `None`, otherwise `f(value)`.
    pub fn opt<T>(v: Option<T>, f: impl FnOnce(T) -> Json) -> Json {
        v.map_or(Json::Null, f)
    }

    /// Serializes to compact JSON text.
    pub fn encode(&self) -> String {
        let mut out = String::new();
        self.write(&mut out);
        out
    }

    fn write(&self, out: &mut String) {
        match self {
            Json::Null => out.push_str("null"),
            Json::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
            Json::Int(n) => {
                let _ = write!(out, "{n}");
            }
            Json::Str(s) => escape_into(s, out),
            Json::Arr(items) => {
                out.push('[');
                for (i, v) in items.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    v.write(out);
                }
                out.push(']');
            }
            Json::Obj(fields) => {
                out.push('{');
                for (i, (k, v)) in fields.iter().enumerate() {
                    if i > 0 {
                        out.push(',');
                    }
                    escape_into(k, out);
                    out.push(':');
                    v.write(out);
                }
                out.push('}');
            }
        }
    }
}

/// Writes `s` as a quoted, escaped JSON string.
pub fn escape_into(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0C}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            // U+2028/2029 are valid JSON but break some JS embedders.
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// `s` as a quoted, escaped JSON string.
pub fn escape(s: &str) -> String {
    let mut out = String::new();
    escape_into(s, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escapes() {
        assert_eq!(escape("plain"), r#""plain""#);
        assert_eq!(escape("a\"b\\c"), r#""a\"b\\c""#);
        assert_eq!(escape("\n\r\t\u{08}\u{0C}"), r#""\n\r\t\b\f""#);
        assert_eq!(escape("\u{0}\u{1f}"), r#""\u0000\u001f""#);
        assert_eq!(escape("é ♠ \u{2028}"), "\"é ♠ \\u2028\"");
    }

    #[test]
    fn encodes_nested() {
        let v = Json::Obj(vec![
            (
                "a",
                Json::Arr(vec![Json::int(1), Json::Int(-2), Json::Null]),
            ),
            ("b", Json::Bool(true)),
            ("c", Json::str("x\"y")),
            ("d", Json::Obj(vec![])),
        ]);
        assert_eq!(
            v.encode(),
            r#"{"a":[1,-2,null],"b":true,"c":"x\"y","d":{}}"#
        );
    }
}
