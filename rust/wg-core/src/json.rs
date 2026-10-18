//! Writer that reproduces the byte layout of the legacy `WindowGrid.settings` files.
//!
//! The legacy app serialised with `DataContractJsonSerializer` and then re-indented the
//! result with `JSONFormatter`: four-space indent, CRLF line endings, a space after
//! every `:`, and `{` / `[` always on their own line (even after a key). Empty
//! containers therefore print as an opening and a closing line.

use std::fmt::Write;

/// The small JSON subset the settings file uses. Floats keep `f32` precision.
#[derive(Clone, Debug, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Int(i64),
    Float(f32),
    Str(String),
    Array(Vec<Json>),
    Object(Vec<(String, Json)>),
}

impl Json {
    pub fn str(s: impl Into<String>) -> Json {
        Json::Str(s.into())
    }

    pub fn object<const N: usize>(fields: [(&str, Json); N]) -> Json {
        Json::Object(fields.into_iter().map(|(k, v)| (k.to_string(), v)).collect())
    }
}

/// Escapes a string the way `DataContractJsonSerializer` does (`/` becomes `\/`).
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' => out.push_str("\\/"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

fn scalar(v: &Json) -> String {
    match v {
        Json::Null => "null".to_string(),
        Json::Bool(b) => b.to_string(),
        Json::Int(i) => i.to_string(),
        // Rust prints the shortest representation that round-trips, and `3.0` as `3`,
        // which is what .NET printed for `float`.
        Json::Float(f) => f.to_string(),
        Json::Str(s) => escape(s),
        Json::Array(_) | Json::Object(_) => unreachable!("containers are not scalars"),
    }
}

fn emit(v: &Json, indent: usize, key: Option<&str>, comma: bool, lines: &mut Vec<String>) {
    let pad = "    ".repeat(indent);
    let comma = if comma { "," } else { "" };
    let prefix = key.map(|k| format!("{}:", escape(k)));
    match v {
        Json::Array(items) => {
            if let Some(p) = prefix {
                lines.push(format!("{pad}{p}"));
            }
            lines.push(format!("{pad}["));
            for (i, item) in items.iter().enumerate() {
                emit(item, indent + 1, None, i + 1 < items.len(), lines);
            }
            lines.push(format!("{pad}]{comma}"));
        }
        Json::Object(fields) => {
            if let Some(p) = prefix {
                lines.push(format!("{pad}{p}"));
            }
            lines.push(format!("{pad}{{"));
            for (i, (k, item)) in fields.iter().enumerate() {
                emit(item, indent + 1, Some(k), i + 1 < fields.len(), lines);
            }
            lines.push(format!("{pad}}}{comma}"));
        }
        other => match prefix {
            Some(p) => lines.push(format!("{pad}{p} {}{comma}", scalar(other))),
            None => lines.push(format!("{pad}{}{comma}", scalar(other))),
        },
    }
}

/// Formats `value` exactly like the legacy writer, including the trailing CRLF.
pub fn format(value: &Json) -> String {
    let mut lines = Vec::new();
    emit(value, 0, None, false, &mut lines);
    let mut out = lines.join("\r\n");
    out.push_str("\r\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_like_legacy_writer() {
        let v = Json::object([
            ("A", Json::object([("X", Json::Bool(true)), ("Y", Json::Float(3.0))])),
            ("L", Json::Array(vec![Json::object([("K", Json::str("v"))])])),
            ("E", Json::Array(vec![])),
        ]);
        let expected = "{\r\n    \"A\":\r\n    {\r\n        \"X\": true,\r\n        \"Y\": 3\r\n    },\r\n    \"L\":\r\n    [\r\n        {\r\n            \"K\": \"v\"\r\n        }\r\n    ],\r\n    \"E\":\r\n    [\r\n    ]\r\n}\r\n";
        assert_eq!(format(&v), expected);
    }

    #[test]
    fn floats_use_shortest_form() {
        assert_eq!(scalar(&Json::Float(0.75)), "0.75");
        assert_eq!(scalar(&Json::Float(3.0)), "3");
        assert_eq!(scalar(&Json::Float(0.1)), "0.1");
    }

    #[test]
    fn escapes_like_data_contract_serializer() {
        assert_eq!(escape("a/b\"c\\\n"), "\"a\\/b\\\"c\\\\\\n\"");
        assert_eq!(escape("\u{1}"), "\"\\u0001\"");
    }
}
