//! JSON editor for the GPUI port: one parsed document shown as Text, Tree or Table.
//!
//! Follows the web extension (`src/extensions/bundles/json.ts`). The file's own
//! indentation, line endings and final newline are kept when the view writes
//! back. Numbers are stored as the exact text from the file, so
//! `12345678901234567890` and `1.0` survive every edit.
//!
//! The data layer (`Value`, `parse`, `parse_lenient`, path operations) has no
//! GPUI dependency and is unit-tested at the bottom of this file. The view
//! (`JsonView`) is built on the crate's `Editor`.
//!
//! The Text / Tree / Table switch lives in the tab bar, not in this view, so the
//! view only exposes `set_mode` and `mode`.

use std::cmp::Ordering;
use std::collections::HashSet;
use std::fmt;
use std::ops::Range;

use gpui::prelude::*;
use gpui::{
    div, px, uniform_list, AnyElement, AnyWindowHandle, App, ClickEvent, Context, CursorStyle, Div,
    ElementId, Entity, FocusHandle, Focusable, Rgba, SharedString, Stateful, UniformListScrollHandle,
    Window,
};

use crate::app_icon;
use crate::editor::{Editor, MONO_FONT};
use crate::theme::Theme;

// ---------------------------------------------------------------------------
// Data layer
// ---------------------------------------------------------------------------

/// Index path from the root: array positions or object entry positions.
/// Entry positions (not keys) make duplicate keys addressable.
pub type JsonPath = Vec<usize>;

const MAX_DEPTH: usize = 512;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Null,
    Bool,
    Number,
    String,
    Array,
    Object,
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    /// The number exactly as written: "12345678901234567890", "1.0", "1e400".
    Number(String),
    String(String),
    Array(Vec<Value>),
    /// Entries in file order. Duplicate keys are kept as written.
    Object(Vec<(String, Value)>),
}

/// A parse failure with a 1-based line and column (column counts characters).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ParseError {
    pub line: usize,
    pub column: usize,
    pub message: String,
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "line {}, column {}: {}", self.line, self.column, self.message)
    }
}

fn missing() -> String {
    "No value at that path".to_string()
}

fn child_path(parent: &[usize], index: usize) -> JsonPath {
    let mut path = parent.to_vec();
    path.push(index);
    path
}

/// `base`, or `base 2`, `base 3`, ... whichever is first unused among `entries`.
fn unique_key(entries: &[(String, Value)], base: &str) -> String {
    if !entries.iter().any(|(k, _)| k == base) {
        return base.to_string();
    }
    (2..)
        .map(|n| format!("{base} {n}"))
        .find(|candidate| !entries.iter().any(|(k, _)| k == candidate))
        .unwrap_or_else(|| base.to_string())
}

fn rank(value: &Value) -> u8 {
    match value {
        Value::Null => 0,
        Value::Bool(_) => 1,
        Value::Number(_) => 2,
        Value::String(_) => 3,
        Value::Array(_) => 4,
        Value::Object(_) => 5,
    }
}

fn compare_values(a: &Value, b: &Value) -> Ordering {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => match (x.parse::<f64>(), y.parse::<f64>()) {
            (Ok(x), Ok(y)) if x.is_finite() && y.is_finite() => {
                x.partial_cmp(&y).unwrap_or(Ordering::Equal)
            }
            _ => x.cmp(y),
        },
        (Value::Bool(x), Value::Bool(y)) => x.cmp(y),
        (Value::String(x), Value::String(y)) => x.cmp(y),
        _ => {
            if rank(a) == rank(b) {
                a.to_compact().cmp(&b.to_compact())
            } else {
                rank(a).cmp(&rank(b))
            }
        }
    }
}

impl Value {
    /// A number value, if `text` is a JSON number spelled exactly as given.
    #[cfg_attr(not(test), allow(dead_code))] // only the tests construct it so far
    pub fn number(text: &str) -> Option<Value> {
        match parse(text) {
            Ok(Value::Number(t)) if t == text => Some(Value::Number(t)),
            _ => None,
        }
    }

    pub fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Bool(_) => Kind::Bool,
            Value::Number(_) => Kind::Number,
            Value::String(_) => Kind::String,
            Value::Array(_) => Kind::Array,
            Value::Object(_) => Kind::Object,
        }
    }

    pub fn is_container(&self) -> bool {
        matches!(self, Value::Array(_) | Value::Object(_))
    }

    pub fn child_count(&self) -> usize {
        match self {
            Value::Array(items) => items.len(),
            Value::Object(entries) => entries.len(),
            _ => 0,
        }
    }

    /// Children of the container at `path`; 0 when there is none.
    pub fn child_count_at(&self, path: &[usize]) -> usize {
        self.get(path).map_or(0, Value::child_count)
    }

    /// The first entry with this key, for objects.
    pub fn member(&self, key: &str) -> Option<&Value> {
        match self {
            Value::Object(entries) => entries.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn get(&self, path: &[usize]) -> Option<&Value> {
        let mut cur = self;
        for &i in path {
            cur = match cur {
                Value::Array(items) => items.get(i)?,
                Value::Object(entries) => &entries.get(i)?.1,
                _ => return None,
            };
        }
        Some(cur)
    }

    pub fn get_mut(&mut self, path: &[usize]) -> Option<&mut Value> {
        let mut cur = self;
        for &i in path {
            cur = match cur {
                Value::Array(items) => items.get_mut(i)?,
                Value::Object(entries) => &mut entries.get_mut(i)?.1,
                _ => return None,
            };
        }
        Some(cur)
    }

    /// The key of the entry at `path`, when its parent is an object.
    pub fn key_at(&self, path: &[usize]) -> Option<&str> {
        let (last, parent) = path.split_last()?;
        match self.get(parent)? {
            Value::Object(entries) => entries.get(*last).map(|(k, _)| k.as_str()),
            _ => None,
        }
    }

    /// Replaces the value at `path`. An empty path replaces the whole document.
    pub fn set(&mut self, path: &[usize], value: Value) -> Result<(), String> {
        let slot = self.get_mut(path).ok_or_else(missing)?;
        *slot = value;
        Ok(())
    }

    /// Removes the value at `path` and returns it. The root cannot be removed.
    pub fn delete(&mut self, path: &[usize]) -> Result<Value, String> {
        let Some((last, parent)) = path.split_last() else {
            return Err("The top-level value cannot be deleted".to_string());
        };
        match self.get_mut(parent) {
            Some(Value::Array(items)) if *last < items.len() => Ok(items.remove(*last)),
            Some(Value::Object(entries)) if *last < entries.len() => Ok(entries.remove(*last).1),
            _ => Err(missing()),
        }
    }

    /// Inserts into the array at `parent`, returning the new element's path.
    pub fn insert_item(
        &mut self,
        parent: &[usize],
        index: usize,
        value: Value,
    ) -> Result<JsonPath, String> {
        match self.get_mut(parent) {
            Some(Value::Array(items)) if index <= items.len() => {
                items.insert(index, value);
                Ok(child_path(parent, index))
            }
            Some(Value::Array(_)) => Err("Index is past the end of the list".to_string()),
            _ => Err("Items can only be added to a list".to_string()),
        }
    }

    /// Inserts a new entry into the object at `parent`. Refuses an existing key.
    pub fn insert_entry(
        &mut self,
        parent: &[usize],
        index: usize,
        key: &str,
        value: Value,
    ) -> Result<JsonPath, String> {
        match self.get_mut(parent) {
            Some(Value::Object(entries)) => {
                if entries.iter().any(|(k, _)| k == key) {
                    return Err(format!("A key named \"{key}\" already exists here"));
                }
                if index > entries.len() {
                    return Err("Index is past the end of the object".to_string());
                }
                entries.insert(index.min(entries.len()), (key.to_string(), value));
                Ok(child_path(parent, index))
            }
            _ => Err("Keys can only be added to an object".to_string()),
        }
    }

    /// Sets `key` on the object at `parent`: replaces the first match or appends.
    pub fn put_key(&mut self, parent: &[usize], key: &str, value: Value) -> Result<(), String> {
        match self.get_mut(parent) {
            Some(Value::Object(entries)) => {
                match entries.iter_mut().find(|(k, _)| k == key) {
                    Some((_, slot)) => *slot = value,
                    None => entries.push((key.to_string(), value)),
                }
                Ok(())
            }
            _ => Err("Keys can only be set on an object".to_string()),
        }
    }

    /// Renames the entry at `path`. Refuses a key a sibling already has.
    pub fn rename_key(&mut self, path: &[usize], key: &str) -> Result<(), String> {
        let Some((last, parent)) = path.split_last() else {
            return Err("The top-level value has no key".to_string());
        };
        match self.get_mut(parent) {
            Some(Value::Object(entries)) => {
                if *last >= entries.len() {
                    return Err(missing());
                }
                let clash = entries
                    .iter()
                    .enumerate()
                    .any(|(i, (k, _))| i != *last && k == key);
                if clash {
                    return Err(format!("A key named \"{key}\" already exists here"));
                }
                entries[*last].0 = key.to_string();
                Ok(())
            }
            _ => Err("Only object entries have keys".to_string()),
        }
    }

    /// Inserts a copy of the value at `path` right after it. Object copies get
    /// a fresh key ("name copy", "name copy 2", ...). Returns the copy's path.
    pub fn duplicate(&mut self, path: &[usize]) -> Result<JsonPath, String> {
        let Some((last, parent)) = path.split_last() else {
            return Err("The top-level value cannot be duplicated".to_string());
        };
        let last = *last;
        match self.get_mut(parent) {
            Some(Value::Array(items)) if last < items.len() => {
                let copy = items[last].clone();
                items.insert(last + 1, copy);
                Ok(child_path(parent, last + 1))
            }
            Some(Value::Object(entries)) if last < entries.len() => {
                let key = unique_key(entries, &format!("{} copy", entries[last].0));
                let copy = entries[last].1.clone();
                entries.insert(last + 1, (key, copy));
                Ok(child_path(parent, last + 1))
            }
            _ => Err(missing()),
        }
    }

    /// Sorts object keys A to Z (byte order). With `recursive`, nested objects too.
    pub fn sort_keys(&mut self, recursive: bool) {
        match self {
            Value::Object(entries) => {
                entries.sort_by(|a, b| a.0.cmp(&b.0));
                if recursive {
                    for (_, value) in entries.iter_mut() {
                        value.sort_keys(true);
                    }
                }
            }
            Value::Array(items) if recursive => {
                for item in items.iter_mut() {
                    item.sort_keys(true);
                }
            }
            _ => {}
        }
    }

    /// Sorts array items by type, then value. Changes order, so it is never automatic.
    pub fn sort_arrays(&mut self, recursive: bool) {
        match self {
            Value::Array(items) => {
                if recursive {
                    for item in items.iter_mut() {
                        item.sort_arrays(true);
                    }
                }
                items.sort_by(compare_values);
            }
            Value::Object(entries) if recursive => {
                for (_, value) in entries.iter_mut() {
                    value.sort_arrays(true);
                }
            }
            _ => {}
        }
    }

    /// Index path for an RFC 6901 JSON pointer such as `/users/0/name`.
    /// For duplicate keys, the first entry wins.
    #[cfg_attr(not(test), allow(dead_code))] // only the tests use it so far
    pub fn pointer_path(&self, pointer: &str) -> Option<JsonPath> {
        if pointer.is_empty() {
            return Some(Vec::new());
        }
        let rest = pointer.strip_prefix('/')?;
        let mut path = Vec::new();
        let mut cur = self;
        for raw in rest.split('/') {
            let token = raw.replace("~1", "/").replace("~0", "~");
            let index = match cur {
                Value::Array(items) => {
                    let i: usize = token.parse().ok()?;
                    if i >= items.len() {
                        return None;
                    }
                    i
                }
                Value::Object(entries) => entries.iter().position(|(k, _)| *k == token)?,
                _ => return None,
            };
            path.push(index);
            cur = cur.get(&[index])?;
        }
        Some(path)
    }

    /// Paths of every array and object, root first.
    pub fn container_paths(&self) -> Vec<JsonPath> {
        let mut out = Vec::new();
        self.collect_containers(&mut Vec::new(), &mut out);
        out
    }

    fn collect_containers(&self, path: &mut JsonPath, out: &mut Vec<JsonPath>) {
        if !self.is_container() {
            return;
        }
        out.push(path.clone());
        match self {
            Value::Array(items) => {
                for (i, item) in items.iter().enumerate() {
                    path.push(i);
                    item.collect_containers(path, out);
                    path.pop();
                }
            }
            Value::Object(entries) => {
                for (i, (_, item)) in entries.iter().enumerate() {
                    path.push(i);
                    item.collect_containers(path, out);
                    path.pop();
                }
            }
            _ => {}
        }
    }

    /// Single-line JSON with no spaces.
    pub fn to_compact(&self) -> String {
        let mut out = String::new();
        write_value(self, None, 0, &mut out);
        out
    }

    /// Multi-line JSON, one `indent` string per level.
    pub fn to_pretty(&self, indent: &str) -> String {
        let mut out = String::new();
        write_value(self, Some(indent), 0, &mut out);
        out
    }
}

fn newline_indent(indent: Option<&str>, depth: usize, out: &mut String) {
    if let Some(unit) = indent {
        out.push('\n');
        for _ in 0..depth {
            out.push_str(unit);
        }
    }
}

fn write_value(value: &Value, indent: Option<&str>, depth: usize, out: &mut String) {
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(true) => out.push_str("true"),
        Value::Bool(false) => out.push_str("false"),
        Value::Number(text) => out.push_str(text),
        Value::String(s) => write_string(s, out),
        Value::Array(items) => {
            if items.is_empty() {
                out.push_str("[]");
                return;
            }
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline_indent(indent, depth + 1, out);
                write_value(item, indent, depth + 1, out);
            }
            newline_indent(indent, depth, out);
            out.push(']');
        }
        Value::Object(entries) => {
            if entries.is_empty() {
                out.push_str("{}");
                return;
            }
            out.push('{');
            for (i, (key, item)) in entries.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                newline_indent(indent, depth + 1, out);
                write_string(key, out);
                out.push(':');
                if indent.is_some() {
                    out.push(' ');
                }
                write_value(item, indent, depth + 1, out);
            }
            newline_indent(indent, depth, out);
            out.push('}');
        }
    }
}

fn write_string(s: &str, out: &mut String) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Strict RFC 8259 parse. Duplicate keys and number spelling are preserved.
pub fn parse(text: &str) -> Result<Value, ParseError> {
    parse_with(text, false)
}

/// Parse that also accepts common breakage: comments, trailing commas, single
/// quotes, unquoted keys, raw control characters in strings.
pub fn parse_lenient(text: &str) -> Result<Value, ParseError> {
    parse_with(text, true)
}

/// The value typed into a tree, cell or table field. Text that is not valid
/// JSON becomes a string, except input starting with `{`, `[` or `"`, which
/// must parse.
pub fn parse_value_text(input: &str) -> Result<Value, String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return Err("Nothing to save".to_string());
    }
    match parse(trimmed) {
        Ok(value) => Ok(value),
        Err(e) if trimmed.starts_with(['{', '[', '"']) => Err(e.to_string()),
        Err(_) => Ok(Value::String(trimmed.to_string())),
    }
}

fn parse_with(text: &str, lenient: bool) -> Result<Value, ParseError> {
    let mut p = Parser {
        src: text,
        bytes: text.as_bytes(),
        pos: if text.starts_with('\u{feff}') { 3 } else { 0 },
        lenient,
    };
    p.skip_ws()?;
    if p.pos >= p.bytes.len() {
        return Err(p.fail("Unexpected end of input"));
    }
    let value = p.value(0)?;
    p.skip_ws()?;
    if p.pos < p.bytes.len() {
        return Err(p.fail("Unexpected text after the value"));
    }
    Ok(value)
}

fn locate(src: &str, offset: usize) -> (usize, usize) {
    let bytes = &src.as_bytes()[..offset.min(src.len())];
    let line = bytes.iter().filter(|&&b| b == b'\n').count() + 1;
    let line_start = bytes.iter().rposition(|&b| b == b'\n').map_or(0, |i| i + 1);
    // Count characters, not bytes: skip UTF-8 continuation bytes.
    let column = bytes[line_start..]
        .iter()
        .filter(|&&b| (b & 0xC0) != 0x80)
        .count()
        + 1;
    (line, column)
}

struct Parser<'a> {
    src: &'a str,
    bytes: &'a [u8],
    pos: usize,
    lenient: bool,
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_' || b == b'$'
}

fn is_ident_continue(b: u8) -> bool {
    is_ident_start(b) || b.is_ascii_digit()
}

impl<'a> Parser<'a> {
    fn peek(&self) -> Option<u8> {
        self.bytes.get(self.pos).copied()
    }

    fn error_at(&self, offset: usize, message: &str) -> ParseError {
        let (line, column) = locate(self.src, offset);
        ParseError {
            line,
            column,
            message: message.to_string(),
        }
    }

    /// Error at the current position. At end of input the message says so.
    fn fail(&self, message: &str) -> ParseError {
        if self.pos >= self.bytes.len() {
            return self.error_at(self.pos, "Unexpected end of input");
        }
        let at_comment = self.bytes[self.pos] == b'/'
            && matches!(self.bytes.get(self.pos + 1), Some(b'/' | b'*'));
        if !self.lenient && at_comment {
            return self.error_at(self.pos, "Comments are not allowed in JSON (Repair removes them)");
        }
        self.error_at(self.pos, message)
    }

    fn skip_ws(&mut self) -> Result<(), ParseError> {
        loop {
            match self.peek() {
                Some(b' ' | b'\t' | b'\n' | b'\r') => self.pos += 1,
                Some(b'/') if self.lenient && self.bytes.get(self.pos + 1) == Some(&b'/') => {
                    while let Some(b) = self.peek() {
                        if b == b'\n' {
                            break;
                        }
                        self.pos += 1;
                    }
                }
                Some(b'/') if self.lenient && self.bytes.get(self.pos + 1) == Some(&b'*') => {
                    let start = self.pos;
                    self.pos += 2;
                    loop {
                        if self.pos + 1 >= self.bytes.len() {
                            return Err(self.error_at(start, "Unterminated comment"));
                        }
                        if self.bytes[self.pos] == b'*' && self.bytes[self.pos + 1] == b'/' {
                            self.pos += 2;
                            break;
                        }
                        self.pos += 1;
                    }
                }
                _ => return Ok(()),
            }
        }
    }

    fn value(&mut self, depth: usize) -> Result<Value, ParseError> {
        if depth > MAX_DEPTH {
            return Err(self.fail("Nested too deeply"));
        }
        match self.peek() {
            Some(b'{') => self.object(depth),
            Some(b'[') => self.array(depth),
            Some(b'"') => Ok(Value::String(self.string(b'"')?)),
            Some(b'\'') if self.lenient => Ok(Value::String(self.string(b'\'')?)),
            Some(b'-' | b'0'..=b'9') => self.number(),
            Some(b't') => self.literal("true", Value::Bool(true)),
            Some(b'f') => self.literal("false", Value::Bool(false)),
            Some(b'n') => self.literal("null", Value::Null),
            Some(_) => Err(self.fail("Expected a value")),
            None => Err(self.fail("Unexpected end of input")),
        }
    }

    fn literal(&mut self, word: &str, value: Value) -> Result<Value, ParseError> {
        if self.bytes[self.pos..].starts_with(word.as_bytes()) {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(self.fail("Expected a value"))
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.peek(), Some(b'0'..=b'9')) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<Value, ParseError> {
        let start = self.pos;
        if self.peek() == Some(b'-') {
            self.pos += 1;
        }
        match self.peek() {
            Some(b'0') => self.pos += 1,
            Some(b'1'..=b'9') => {
                self.digits();
            }
            _ => return Err(self.fail("Invalid number")),
        }
        if self.peek() == Some(b'.') {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.fail("Expected digits after the decimal point"));
            }
        }
        if matches!(self.peek(), Some(b'e' | b'E')) {
            self.pos += 1;
            if matches!(self.peek(), Some(b'+' | b'-')) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.fail("Expected digits in the exponent"));
            }
        }
        Ok(Value::Number(self.src[start..self.pos].to_string()))
    }

    fn hex4(&mut self) -> Result<u32, ParseError> {
        let mut code = 0u32;
        for _ in 0..4 {
            let digit = self
                .peek()
                .and_then(|b| (b as char).to_digit(16))
                .ok_or_else(|| self.fail("Invalid \\u escape"))?;
            code = code * 16 + digit;
            self.pos += 1;
        }
        Ok(code)
    }

    /// Reads the part after `\u`. Surrogate pairs combine; a lone surrogate is
    /// an error in strict mode and U+FFFD in lenient mode.
    fn unicode_escape(&mut self) -> Result<char, ParseError> {
        let start = self.pos;
        let hi = self.hex4()?;
        let lone = |p: &Parser<'a>| {
            if p.lenient {
                Ok('\u{FFFD}')
            } else {
                Err(p.error_at(start, "Unpaired surrogate in \\u escape"))
            }
        };
        if (0xD800..0xDC00).contains(&hi) {
            if self.bytes.get(self.pos..self.pos + 2) == Some(b"\\u".as_slice()) {
                let save = self.pos;
                self.pos += 2;
                let lo = self.hex4()?;
                if (0xDC00..0xE000).contains(&lo) {
                    let cp = 0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00);
                    return Ok(char::from_u32(cp).unwrap_or('\u{FFFD}'));
                }
                self.pos = save;
            }
            return lone(self);
        }
        if (0xDC00..0xE000).contains(&hi) {
            return lone(self);
        }
        Ok(char::from_u32(hi).unwrap_or('\u{FFFD}'))
    }

    fn string(&mut self, quote: u8) -> Result<String, ParseError> {
        let start = self.pos;
        self.pos += 1;
        let mut out = String::new();
        let mut run = self.pos;
        loop {
            let Some(b) = self.peek() else {
                return Err(self.error_at(start, "Unterminated string"));
            };
            if b == quote {
                out.push_str(&self.src[run..self.pos]);
                self.pos += 1;
                return Ok(out);
            }
            match b {
                b'\\' => {
                    out.push_str(&self.src[run..self.pos]);
                    let escape_at = self.pos;
                    self.pos += 1;
                    let Some(esc) = self.peek() else {
                        return Err(self.error_at(start, "Unterminated string"));
                    };
                    self.pos += 1;
                    match esc {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{8}'),
                        b'f' => out.push('\u{c}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'\'' if self.lenient => out.push('\''),
                        b'u' => out.push(self.unicode_escape()?),
                        _ => return Err(self.error_at(escape_at, "Invalid escape sequence")),
                    }
                    run = self.pos;
                }
                0..=0x1f if !self.lenient => {
                    return Err(self.fail("Control character in string (escape it as \\n, \\t or \\u00XX)"));
                }
                _ => self.pos += 1,
            }
        }
    }

    fn key(&mut self) -> Result<String, ParseError> {
        match self.peek() {
            Some(b'"') => self.string(b'"'),
            Some(b'\'') if self.lenient => self.string(b'\''),
            Some(b) if self.lenient && is_ident_start(b) => {
                let start = self.pos;
                while matches!(self.peek(), Some(c) if is_ident_continue(c)) {
                    self.pos += 1;
                }
                Ok(self.src[start..self.pos].to_string())
            }
            _ => Err(self.fail("Expected a quoted key")),
        }
    }

    fn object(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut entries = Vec::new();
        self.skip_ws()?;
        if self.peek() == Some(b'}') {
            self.pos += 1;
            return Ok(Value::Object(entries));
        }
        loop {
            let key = self.key()?;
            self.skip_ws()?;
            if self.peek() != Some(b':') {
                return Err(self.fail("Expected ':' after the key"));
            }
            self.pos += 1;
            self.skip_ws()?;
            let value = self.value(depth + 1)?;
            entries.push((key, value));
            self.skip_ws()?;
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws()?;
                    if self.peek() == Some(b'}') {
                        if self.lenient {
                            self.pos += 1;
                            return Ok(Value::Object(entries));
                        }
                        return Err(self.fail("Trailing comma is not allowed in JSON"));
                    }
                }
                Some(b'}') => {
                    self.pos += 1;
                    return Ok(Value::Object(entries));
                }
                _ => return Err(self.fail("Expected ',' or '}' after the value")),
            }
        }
    }

    fn array(&mut self, depth: usize) -> Result<Value, ParseError> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws()?;
        if self.peek() == Some(b']') {
            self.pos += 1;
            return Ok(Value::Array(items));
        }
        loop {
            items.push(self.value(depth + 1)?);
            self.skip_ws()?;
            match self.peek() {
                Some(b',') => {
                    self.pos += 1;
                    self.skip_ws()?;
                    if self.peek() == Some(b']') {
                        if self.lenient {
                            self.pos += 1;
                            return Ok(Value::Array(items));
                        }
                        return Err(self.fail("Trailing comma is not allowed in JSON"));
                    }
                }
                Some(b']') => {
                    self.pos += 1;
                    return Ok(Value::Array(items));
                }
                _ => return Err(self.fail("Expected ',' or ']' after the value")),
            }
        }
    }
}

/// How a file was written, so the view writes it back the same way.
#[derive(Clone, Debug, PartialEq)]
pub struct Style {
    pub indent: String,
    pub crlf: bool,
    pub final_newline: bool,
}

impl Default for Style {
    fn default() -> Self {
        Style {
            indent: "  ".to_string(),
            crlf: false,
            final_newline: false,
        }
    }
}

impl Style {
    /// Indent from the first indented line (tab or spaces), line endings from
    /// the first line break, and whether the text ends in a newline.
    pub fn detect(text: &str) -> Style {
        let indent = text
            .lines()
            .find_map(|line| {
                if let Some(rest) = line.strip_prefix('\t') {
                    return rest
                        .chars()
                        .next()
                        .filter(|c| !c.is_whitespace())
                        .map(|_| "\t".to_string());
                }
                let spaces = line.bytes().take_while(|&b| b == b' ').count();
                if spaces == 0 {
                    return None;
                }
                line[spaces..]
                    .chars()
                    .next()
                    .filter(|c| !c.is_whitespace())
                    .map(|_| line[..spaces].to_string())
            })
            .unwrap_or_else(|| "  ".to_string());
        Style {
            indent,
            crlf: text.contains("\r\n"),
            final_newline: text.ends_with('\n'),
        }
    }

    /// The text the view works on: LF line endings, with the final newline set.
    pub fn render(&self, doc: &Value) -> String {
        let mut out = doc.to_pretty(&self.indent);
        if self.final_newline {
            out.push('\n');
        }
        out
    }

    /// Applies the file's own line endings to LF text.
    pub fn to_file_text(&self, lf_text: &str) -> String {
        if self.crlf {
            lf_text.replace('\n', "\r\n")
        } else {
            lf_text.to_string()
        }
    }
}

/// Text with CRLF turned into LF, for the editor.
pub fn normalize_newlines(text: &str) -> String {
    text.replace("\r\n", "\n")
}

/// Top-level array of objects -> (column names, every item is an object).
fn table_columns(doc: &Value) -> Result<Vec<String>, String> {
    let Value::Array(items) = doc else {
        return Err("The table view shows a list of objects. This file's top level is not a list.".to_string());
    };
    let mut columns: Vec<String> = Vec::new();
    for item in items {
        let Value::Object(entries) = item else {
            return Err("The table view needs every item in the list to be an object.".to_string());
        };
        for (key, _) in entries {
            if !columns.iter().any(|c| c == key) {
                columns.push(key.clone());
            }
        }
    }
    Ok(columns)
}

// ---------------------------------------------------------------------------
// View
// ---------------------------------------------------------------------------

const ROW_H: f32 = 26.0;
const INDENT_W: f32 = 16.0;
const COL_W: f32 = 180.0;
const INDEX_W: f32 = 56.0;
const ACTIONS_W: f32 = 84.0;
const MAX_PREVIEW: usize = 240;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Text,
    Tree,
    Table,
}

impl Mode {
    pub fn id(self) -> &'static str {
        match self {
            Mode::Text => "text",
            Mode::Tree => "tree",
            Mode::Table => "table",
        }
    }

    pub fn from_id(id: &str) -> Option<Mode> {
        match id {
            "text" => Some(Mode::Text),
            "tree" => Some(Mode::Tree),
            "table" => Some(Mode::Table),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
enum EditTarget {
    /// Rename the key of the entry at this path.
    Key(JsonPath),
    /// Replace the value at this path.
    Value(JsonPath),
    /// Set `key` on the object at table row `row`.
    Cell { row: usize, key: String },
}

struct Edit {
    target: EditTarget,
    input: Entity<Editor>,
}

#[derive(Clone, Debug)]
struct TreeRow {
    path: JsonPath,
    depth: usize,
    /// Object key, or the index for array items.
    label: Option<String>,
    is_index: bool,
    kind: Kind,
    preview: String,
    open: bool,
    matched: bool,
}

impl TreeRow {
    fn is_container(&self) -> bool {
        matches!(self.kind, Kind::Array | Kind::Object)
    }
}

pub struct JsonView {
    theme_name: String,
    mode: Mode,
    style: Style,
    /// LF text the Text editor shows. Always the source of truth.
    text: String,
    doc: Option<Value>,
    error: Option<ParseError>,
    /// (message, is_error)
    notice: Option<(String, bool)>,
    text_editor: Entity<Editor>,
    search_editor: Entity<Editor>,
    query: String,
    search_dirty: bool,
    search_hits: usize,
    expanded: HashSet<JsonPath>,
    edit: Option<Edit>,
    tree_rows: Vec<TreeRow>,
    table_cols: Vec<String>,
    table_rows: Vec<usize>,
    tree_scroll: UniformListScrollHandle,
    table_scroll: UniformListScrollHandle,
    focus: FocusHandle,
    /// The window the current edit started in, so focus can return to the
    /// view once a deferred commit or cancel has dropped the input.
    edit_window: Option<AnyWindowHandle>,
    /// Called with the full text after every edit, with the file's line endings.
    /// Do not read the view from inside this callback; the view is busy then.
    pub on_change: Option<Box<dyn Fn(&str, &mut App)>>,
}

impl JsonView {
    /// Opens `content`. Starts in Tree when the JSON is valid, else Text.
    pub fn new(content: &str, theme_name: &str, cx: &mut Context<Self>) -> Self {
        let style = Style::detect(content);
        let text = normalize_newlines(content);
        let (doc, error) = match parse(&text) {
            Ok(value) => (Some(value), None),
            Err(e) => (None, Some(e)),
        };
        let theme_name = theme_name.to_string();
        let weak = cx.weak_entity();

        let text_editor = cx.new(|cx| {
            let mut editor = Editor::new(false, cx);
            editor.theme_name = theme_name.clone();
            editor
        });
        let w = weak.clone();
        let initial = text.clone();
        text_editor.update(cx, |editor, cx| {
            editor.set_text(&initial, cx);
            editor.on_change = Some(Box::new(move |t, cx| {
                let t = t.to_string();
                let _ = w.update(cx, |this, cx| this.on_text_input(t, cx));
            }));
        });

        let search_editor = cx.new(|cx| {
            Editor::new(true, cx)
                .with_placeholder("Search")
                .with_padding(6.0, 2.0)
        });
        let w = weak.clone();
        search_editor.update(cx, |editor, _| {
            editor.theme_name = theme_name.clone();
            editor.on_change = Some(Box::new(move |t, cx| {
                let t = t.to_string();
                let _ = w.update(cx, |this, cx| {
                    this.query = t;
                    this.search_dirty = true;
                    cx.notify();
                });
            }));
        });

        let mode = if doc.is_some() { Mode::Tree } else { Mode::Text };
        let mut expanded = HashSet::new();
        expanded.insert(Vec::new());

        JsonView {
            theme_name,
            mode,
            style,
            text,
            doc,
            error,
            notice: None,
            text_editor,
            search_editor,
            query: String::new(),
            search_dirty: false,
            search_hits: 0,
            expanded,
            edit: None,
            tree_rows: Vec::new(),
            table_cols: Vec::new(),
            table_rows: Vec::new(),
            tree_scroll: UniformListScrollHandle::new(),
            table_scroll: UniformListScrollHandle::new(),
            focus: cx.focus_handle(),
            edit_window: None,
            on_change: None,
        }
    }

    /// The current document as it would be written to the file.
    pub fn text(&self) -> String {
        self.style.to_file_text(&self.text)
    }

    /// The raw-text editor, for Find and Replace while the Text view is shown.
    pub fn text_editor(&self) -> Entity<Editor> {
        self.text_editor.clone()
    }

    /// "text", "tree" or "table".
    pub fn mode(&self) -> &'static str {
        self.mode.id()
    }

    /// Switches view. Unknown ids are ignored, and Tree and Table are refused
    /// while the text is not valid JSON (only Text can show the error).
    pub fn set_mode(&mut self, mode: &str, cx: &mut Context<Self>) {
        let Some(next) = Mode::from_id(mode) else {
            return;
        };
        if next == self.mode || (next != Mode::Text && self.doc.is_none()) {
            return;
        }
        self.mode = next;
        self.edit = None;
        self.search_dirty = true;
        cx.notify();
    }

    pub fn set_theme(&mut self, name: &str, cx: &mut Context<Self>) {
        if self.theme_name == name {
            return;
        }
        self.theme_name = name.to_string();
        let theme = self.theme_name.clone();
        self.text_editor.update(cx, |editor, cx| editor.set_theme(&theme, cx));
        self.search_editor.update(cx, |editor, cx| editor.set_theme(&theme, cx));
        if let Some(edit) = self.edit.as_ref() {
            let input = edit.input.clone();
            input.update(cx, |editor, cx| editor.set_theme(&theme, cx));
        }
        cx.notify();
    }

    /// Replaces the whole content, for example after the file changed on disk.
    /// Does not fire `on_change`. No caller yet: the reload path is not wired.
    #[allow(dead_code)]
    pub fn set_content(&mut self, content: &str, cx: &mut Context<Self>) {
        self.style = Style::detect(content);
        self.reparse(normalize_newlines(content));
        self.edit = None;
        self.expanded.clear();
        self.expanded.insert(Vec::new());
        self.search_dirty = true;
        self.set_editor_text(cx);
        cx.notify();
    }

    fn reparse(&mut self, text: String) {
        match parse(&text) {
            Ok(value) => {
                self.doc = Some(value);
                self.error = None;
            }
            Err(e) => {
                self.doc = None;
                self.error = Some(e);
            }
        }
        self.text = text;
    }

    fn on_text_input(&mut self, text: String, cx: &mut Context<Self>) {
        self.reparse(text);
        self.notice = None;
        self.search_dirty = true;
        self.emit(cx);
    }

    fn set_editor_text(&mut self, cx: &mut Context<Self>) {
        let text = self.text.clone();
        self.text_editor.update(cx, |editor, cx| editor.set_text(&text, cx));
    }

    fn emit(&mut self, cx: &mut Context<Self>) {
        if let Some(on_change) = self.on_change.as_ref() {
            let out = self.text();
            on_change(&out, &mut **cx);
        }
        cx.notify();
    }

    fn notify_user(&mut self, message: impl Into<String>, is_error: bool) {
        self.notice = Some((message.into(), is_error));
    }

    /// Regenerates the text from the document and tells the parent.
    fn sync_text(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_ref() {
            self.text = self.style.render(doc);
        }
        self.error = None;
        self.set_editor_text(cx);
        self.search_dirty = true;
        self.emit(cx);
    }

    /// Applies a change to the document and re-renders the text. Returns None
    /// (and shows the message) if the change is refused.
    fn mutate<T>(
        &mut self,
        cx: &mut Context<Self>,
        f: impl FnOnce(&mut Value) -> Result<T, String>,
    ) -> Option<T> {
        let result = match self.doc.as_mut() {
            Some(doc) => f(doc),
            None => Err("Fix the JSON syntax first, or use Repair".to_string()),
        };
        match result {
            Ok(value) => {
                self.notice = None;
                self.sync_text(cx);
                Some(value)
            }
            Err(message) => {
                self.notify_user(message, true);
                cx.notify();
                None
            }
        }
    }

    fn repair(&mut self, cx: &mut Context<Self>) {
        match parse_lenient(&self.text) {
            Ok(doc) => {
                self.doc = Some(doc);
                self.sync_text(cx);
                self.notify_user("Repaired. Check the result before saving.", false);
            }
            Err(e) => self.notify_user(format!("Could not repair: {e}"), true),
        }
        cx.notify();
    }

    fn format_doc(&mut self, cx: &mut Context<Self>) {
        if self.doc.is_none() {
            self.notify_user("Fix the JSON syntax first, or use Repair", true);
        } else {
            self.sync_text(cx);
            self.notify_user("Formatted", false);
        }
        cx.notify();
    }

    fn sort_keys_action(&mut self, cx: &mut Context<Self>) {
        if self.mutate(cx, |doc| {
            doc.sort_keys(true);
            Ok(())
        }).is_some()
        {
            self.notify_user("Keys sorted A to Z, recursively", false);
            cx.notify();
        }
    }

    fn sort_arrays_action(&mut self, cx: &mut Context<Self>) {
        if self.mutate(cx, |doc| {
            doc.sort_arrays(true);
            Ok(())
        }).is_some()
        {
            self.notify_user("List items sorted, recursively", false);
            cx.notify();
        }
    }

    fn toggle(&mut self, path: JsonPath, cx: &mut Context<Self>) {
        if !self.expanded.remove(&path) {
            self.expanded.insert(path);
        }
        cx.notify();
    }

    fn expand_all(&mut self, cx: &mut Context<Self>) {
        if let Some(doc) = self.doc.as_ref() {
            self.expanded.extend(doc.container_paths());
        }
        cx.notify();
    }

    fn collapse_all(&mut self, cx: &mut Context<Self>) {
        self.expanded.clear();
        self.expanded.insert(Vec::new());
        cx.notify();
    }

    fn add_child(&mut self, parent: JsonPath, window: &mut Window, cx: &mut Context<Self>) {
        let kind = self.doc.as_ref().and_then(|d| d.get(&parent)).map(|v| v.kind());
        match kind {
            Some(Kind::Object) => {
                let added = self.mutate(cx, |doc| {
                    let count = doc.child_count_at(&parent);
                    let key = match doc.get(&parent) {
                        Some(Value::Object(entries)) => unique_key(entries, "key"),
                        _ => return Err("Keys can only be added to an object".to_string()),
                    };
                    let path = doc.insert_entry(&parent, count, &key, Value::Null)?;
                    Ok((path, key))
                });
                if let Some((path, key)) = added {
                    self.expanded.insert(parent);
                    self.start_edit(EditTarget::Key(path), &key, window, cx);
                }
            }
            Some(Kind::Array) => {
                let added = self.mutate(cx, |doc| {
                    let count = doc.child_count_at(&parent);
                    doc.insert_item(&parent, count, Value::Null)
                });
                if let Some(path) = added {
                    self.expanded.insert(parent);
                    self.start_edit(EditTarget::Value(path), "null", window, cx);
                }
            }
            _ => self.notify_user("Items can only be added to a list or an object", true),
        }
        cx.notify();
    }

    fn duplicate_node(&mut self, path: JsonPath, cx: &mut Context<Self>) {
        self.edit = None;
        self.mutate(cx, |doc| doc.duplicate(&path));
    }

    fn delete_node(&mut self, path: JsonPath, cx: &mut Context<Self>) {
        self.edit = None;
        self.mutate(cx, |doc| doc.delete(&path).map(|_| ()));
    }

    fn begin_value_edit(&mut self, path: JsonPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(value) = self.doc.as_ref().and_then(|d| d.get(&path)) else {
            return;
        };
        if value.is_container() {
            return;
        }
        let initial = value.to_compact();
        self.start_edit(EditTarget::Value(path), &initial, window, cx);
    }

    fn begin_key_edit(&mut self, path: JsonPath, window: &mut Window, cx: &mut Context<Self>) {
        let Some(key) = self.doc.as_ref().and_then(|d| d.key_at(&path)).map(str::to_string) else {
            return;
        };
        self.start_edit(EditTarget::Key(path), &key, window, cx);
    }

    fn begin_cell_edit(&mut self, row: usize, key: String, window: &mut Window, cx: &mut Context<Self>) {
        let initial = self
            .doc
            .as_ref()
            .and_then(|d| d.get(&[row]))
            .and_then(|item| item.member(&key))
            .map(|v| v.to_compact())
            .unwrap_or_default();
        self.start_edit(EditTarget::Cell { row, key }, &initial, window, cx);
    }

    fn start_edit(
        &mut self,
        target: EditTarget,
        initial: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let theme_name = self.theme_name.clone();
        let input = cx.new(|cx| {
            let mut editor = Editor::new(true, cx).with_padding(4.0, 0.0);
            editor.theme_name = theme_name;
            editor
        });
        let initial = initial.to_string();
        let weak_submit = cx.weak_entity();
        let weak_cancel = weak_submit.clone();
        input.update(cx, |editor, cx| {
            editor.set_text(&initial, cx);
            editor.select(0..initial.len(), cx);
            // Enter and Escape arrive inside the editor's own update, so the
            // edit is committed or dropped on a deferred turn.
            editor.on_submit = Some(Box::new(move |cx: &mut App| {
                let weak = weak_submit.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.commit_edit(cx);
                        this.refocus_after_edit(cx);
                    });
                });
            }));
            editor.on_cancel = Some(Box::new(move |cx: &mut App| {
                let weak = weak_cancel.clone();
                cx.defer(move |cx| {
                    let _ = weak.update(cx, |this, cx| {
                        this.cancel_edit(cx);
                        this.refocus_after_edit(cx);
                    });
                });
            }));
        });
        let handle = input.read(cx).focus_handle(cx);
        window.focus(&handle);
        self.edit = Some(Edit { target, input });
        self.edit_window = Some(window.window_handle());
        cx.notify();
    }

    /// Moves keyboard focus back to the view once an edit has ended. A commit
    /// that was refused keeps its input, and so keeps focus.
    fn refocus_after_edit(&mut self, cx: &mut Context<Self>) {
        if self.edit.is_some() {
            return;
        }
        if let Some(handle) = self.edit_window {
            let focus = self.focus.clone();
            let _ = handle.update(cx, |_, window, _| window.focus(&focus));
        }
    }

    fn cancel_edit(&mut self, cx: &mut Context<Self>) {
        self.edit = None;
        cx.notify();
    }

    fn commit_edit(&mut self, cx: &mut Context<Self>) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        let text = edit.input.read(cx).text().to_string();
        if text.trim().is_empty() {
            cx.notify();
            return;
        }
        match edit.target.clone() {
            EditTarget::Key(path) => {
                let current = self.doc.as_ref().and_then(|d| d.key_at(&path)).map(str::to_string);
                if current.as_deref() != Some(text.as_str()) {
                    self.mutate(cx, |doc| doc.rename_key(&path, &text));
                }
            }
            EditTarget::Value(path) => match parse_value_text(&text) {
                Ok(value) => {
                    self.mutate(cx, |doc| doc.set(&path, value));
                }
                Err(message) => {
                    self.edit = Some(edit);
                    self.notify_user(message, true);
                }
            },
            EditTarget::Cell { row, key } => match parse_value_text(&text) {
                Ok(value) => {
                    self.mutate(cx, |doc| doc.put_key(&[row], &key, value));
                }
                Err(message) => {
                    self.edit = Some(edit);
                    self.notify_user(message, true);
                }
            },
        }
        cx.notify();
    }

    // ---- derived rows ---------------------------------------------------

    fn refresh_tree(&mut self) {
        let mut rows = Vec::new();
        let mut hits = 0;
        let query = self.query.to_lowercase();
        let filter = if query.is_empty() { None } else { Some(query.as_str()) };
        if let Some(doc) = self.doc.as_ref() {
            let mut path = Vec::new();
            walk_rows(
                doc,
                &self.expanded,
                &mut path,
                0,
                None,
                None,
                filter,
                &mut rows,
                &mut hits,
            );
        }
        self.tree_rows = rows;
        self.search_hits = hits;
    }

    fn refresh_table(&mut self) {
        self.table_cols.clear();
        self.table_rows.clear();
        let Some(doc) = self.doc.as_ref() else {
            return;
        };
        let Ok(columns) = table_columns(doc) else {
            return;
        };
        let Value::Array(items) = doc else {
            return;
        };
        let query = self.query.to_lowercase();
        self.table_rows = (0..items.len())
            .filter(|&i| {
                query.is_empty()
                    || match &items[i] {
                        Value::Object(entries) => entries.iter().any(|(k, v)| {
                            k.to_lowercase().contains(&query)
                                || v.to_compact().to_lowercase().contains(&query)
                        }),
                        _ => false,
                    }
            })
            .collect();
        self.search_hits = self.table_rows.len();
        self.table_cols = columns;
    }

    fn table_error(&self) -> Option<String> {
        self.doc.as_ref().and_then(|doc| table_columns(doc).err())
    }

    // ---- rendering ------------------------------------------------------

    fn tree_rows_for(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        let start = range.start;
        let rows: Vec<TreeRow> = self.tree_rows.get(range).map(|r| r.to_vec()).unwrap_or_default();
        let mut out = Vec::with_capacity(rows.len());
        for (i, row) in rows.iter().enumerate() {
            out.push(self.tree_row_element(start + i, row, &theme, cx));
        }
        out
    }

    fn table_rows_for(&mut self, range: Range<usize>, window: &mut Window, cx: &mut Context<Self>) -> Vec<AnyElement> {
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        let start = range.start;
        let indices: Vec<usize> = self.table_rows.get(range).map(|r| r.to_vec()).unwrap_or_default();
        let mut out = Vec::with_capacity(indices.len());
        for (i, row) in indices.into_iter().enumerate() {
            out.push(self.table_row_element(start + i, row, &theme, cx));
        }
        out
    }

    fn edit_input_for(&self, predicate: impl Fn(&EditTarget) -> bool) -> Option<Entity<Editor>> {
        self.edit
            .as_ref()
            .filter(|e| predicate(&e.target))
            .map(|e| e.input.clone())
    }

    fn tree_row_element(&self, ix: usize, row: &TreeRow, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let path = row.path.clone();
        let is_root = path.is_empty();
        let container = row.is_container();
        let key_input = self.edit_input_for(|t| matches!(t, EditTarget::Key(p) if *p == path));
        let value_input = self.edit_input_for(|t| matches!(t, EditTarget::Value(p) if *p == path));

        let toggle = {
            let p = path.clone();
            div()
                .id(("tree-toggle", ix))
                .w(px(14.0))
                .flex_shrink_0()
                .text_color(theme.muted)
                .children(container.then(|| {
                    let name = if row.open { "arrow-down01" } else { "arrow-right01" };
                    app_icon::icon(name, 12.0).text_color(theme.muted)
                }))
                .when(container, |el| {
                    el.cursor(CursorStyle::PointingHand).on_click(cx.listener(
                        move |this, _: &ClickEvent, _, cx| this.toggle(p.clone(), cx),
                    ))
                })
        };

        let key_cell: AnyElement = match key_input {
            Some(input) => div()
                .w(px(180.0))
                .h(px(24.0))
                .flex_shrink_0()
                .overflow_hidden()
                .rounded(px(4.0))
                .border_1()
                .border_color(theme.accent)
                .bg(theme.bg)
                .child(input)
                .into_any_element(),
            None => {
                let label = row.label.clone().unwrap_or_default();
                let editable = !is_root && !row.is_index;
                let colour = if row.is_index { theme.muted } else { theme.accent };
                let p = path.clone();
                div()
                    .id(("tree-key", ix))
                    .max_w(px(180.0))
                    .flex_shrink_0()
                    .truncate()
                    .text_color(colour)
                    .child(label)
                    .when(editable, |el| {
                        el.cursor(CursorStyle::IBeam).on_click(cx.listener(
                            move |this, ev: &ClickEvent, window, cx| {
                                if ev.click_count() >= 2 {
                                    this.begin_key_edit(p.clone(), window, cx);
                                }
                            },
                        ))
                    })
                    .into_any_element()
            }
        };

        let has_label = row.label.is_some();
        let colon = div()
            .flex_shrink_0()
            .text_color(theme.muted)
            .child(if has_label { ":" } else { "" });

        let key_group = div()
            .flex()
            .flex_row()
            .items_center()
            .flex_shrink_0()
            .max_w(px(200.0))
            .child(key_cell)
            .child(colon);

        let value_cell: AnyElement = match value_input {
            Some(input) => div()
                .flex_1()
                .min_w(px(0.0))
                .h(px(24.0))
                .overflow_hidden()
                .rounded(px(4.0))
                .border_1()
                .border_color(theme.accent)
                .bg(theme.bg)
                .child(input)
                .into_any_element(),
            None => {
                let p = path.clone();
                div()
                    .id(("tree-value", ix))
                    .flex_1()
                    .min_w(px(0.0))
                    .truncate()
                    .text_color(value_colour(row.kind, theme))
                    .child(row.preview.clone())
                    .when(!container, |el| {
                        el.cursor(CursorStyle::IBeam).on_click(cx.listener(
                            move |this, ev: &ClickEvent, window, cx| {
                                if ev.click_count() >= 2 {
                                    this.begin_value_edit(p.clone(), window, cx);
                                }
                            },
                        ))
                    })
                    .when(container, |el| {
                        el.cursor(CursorStyle::PointingHand).on_click(cx.listener(
                            move |this, ev: &ClickEvent, _, cx| {
                                if ev.click_count() >= 2 {
                                    this.toggle(path.clone(), cx);
                                }
                            },
                        ))
                    })
                    .into_any_element()
            }
        };

        let p_add = row.path.clone();
        let p_dup = row.path.clone();
        let p_del = row.path.clone();
        let p_edit = row.path.clone();
        let actions = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .flex_shrink_0()
            .when(container, |el| {
                el.child(icon_button(theme, ("tree-add", ix), "plus-sign").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| this.add_child(p_add.clone(), window, cx),
                )))
            })
            .when(!container, |el| {
                el.child(icon_button(theme, ("tree-edit", ix), "pencil-edit01").on_click(cx.listener(
                    move |this, _: &ClickEvent, window, cx| {
                        this.begin_value_edit(p_edit.clone(), window, cx)
                    },
                )))
            })
            .when(!is_root, |el| {
                el.child(icon_button(theme, ("tree-dup", ix), "copy01").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| this.duplicate_node(p_dup.clone(), cx),
                )))
                .child(icon_button(theme, ("tree-del", ix), "trash").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| this.delete_node(p_del.clone(), cx),
                )))
            });

        div()
            .id(("tree-row", ix))
            .h(px(ROW_H))
            .w_full()
            .flex()
            .flex_row()
            .items_center()
            .gap_1()
            .pl(px(8.0 + row.depth as f32 * INDENT_W))
            .pr_2()
            .font_family(MONO_FONT)
            .text_size(px(13.0))
            .when(row.matched, |el| el.bg(theme.accent_soft))
            .hover(|s| s.bg(theme.hover))
            .child(toggle)
            .child(key_group)
            .child(value_cell)
            .child(actions)
            .into_any_element()
    }

    fn table_row_element(&self, ix: usize, item: usize, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        let entries: Vec<(String, Value)> = match self.doc.as_ref().and_then(|d| d.get(&[item])) {
            Some(Value::Object(entries)) => entries.clone(),
            _ => Vec::new(),
        };
        let mut row = div()
            .id(("table-row", ix))
            .h(px(ROW_H))
            .flex()
            .flex_row()
            .items_center()
            .font_family(MONO_FONT)
            .text_size(px(12.0))
            .border_b_1()
            .border_color(theme.border)
            .hover(|s| s.bg(theme.hover))
            .child(
                div()
                    .w(px(INDEX_W))
                    .flex_shrink_0()
                    .px_2()
                    .text_color(theme.muted)
                    .child(item.to_string()),
            );

        for column in &self.table_cols {
            let editing = self
                .edit
                .as_ref()
                .map(|e| matches!(&e.target, EditTarget::Cell { row, key } if *row == item && key == column))
                .unwrap_or(false);
            let cell_value = entries.iter().find(|(k, _)| k == column).map(|(_, v)| v);
            let cell: AnyElement = if editing {
                let input = self.edit.as_ref().map(|e| e.input.clone());
                div()
                    .w(px(COL_W))
                    .flex_shrink_0()
                    .h(px(22.0))
                    .px_1()
                    .child(
                        div()
                            .size_full()
                            .overflow_hidden()
                            .rounded(px(4.0))
                            .border_1()
                            .border_color(theme.accent)
                            .bg(theme.bg)
                            .children(input),
                    )
                    .into_any_element()
            } else {
                let (text, colour) = match cell_value {
                    Some(v) => (preview_of(v), value_colour(v.kind(), theme)),
                    None => (String::new(), theme.muted),
                };
                let key = column.clone();
                div()
                    .id(("table-cell", ix * 100_000 + column_index(&self.table_cols, column)))
                    .w(px(COL_W))
                    .flex_shrink_0()
                    .px_2()
                    .truncate()
                    .cursor(CursorStyle::IBeam)
                    .text_color(colour)
                    .child(text)
                    .on_click(cx.listener(move |this, ev: &ClickEvent, window, cx| {
                        if ev.click_count() >= 2 {
                            this.begin_cell_edit(item, key.clone(), window, cx);
                        }
                    }))
                    .into_any_element()
            };
            row = row.child(cell);
        }

        let p_dup = item;
        let p_del = item;
        row.child(
            div()
                .w(px(ACTIONS_W))
                .flex_shrink_0()
                .flex()
                .flex_row()
                .gap_1()
                .items_center()
                .child(icon_button(theme, ("table-dup", ix), "copy01").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| this.duplicate_node(vec![p_dup], cx),
                )))
                .child(icon_button(theme, ("table-del", ix), "trash").on_click(cx.listener(
                    move |this, _: &ClickEvent, _, cx| this.delete_node(vec![p_del], cx),
                ))),
        )
        .into_any_element()
    }

    fn render_toolbar(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        let mode = self.mode;
        let mut bar = div()
            .flex()
            .flex_row()
            .items_center()
            .gap_2()
            .flex_shrink_0()
            .px_2()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface)
            .child(
                button(theme, "json-format", "Format", false)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.format_doc(cx))),
            )
            .child(
                button(theme, "json-sort-keys", "Sort keys", false)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.sort_keys_action(cx))),
            )
            .child(
                button(theme, "json-sort-arrays", "Sort lists", false)
                    .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.sort_arrays_action(cx))),
            );

        if mode == Mode::Tree {
            bar = bar
                .child(
                    button(theme, "json-expand", "Expand all", false)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.expand_all(cx))),
                )
                .child(
                    button(theme, "json-collapse", "Collapse all", false)
                        .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.collapse_all(cx))),
                );
        }
        if mode == Mode::Table {
            bar = bar.child(
                button(theme, "json-add-row", "+ Row", false).on_click(cx.listener(
                    |this, _: &ClickEvent, _, cx| {
                        let index = this.doc.as_ref().map(|d| d.child_count()).unwrap_or(0);
                        this.mutate(cx, |doc| doc.insert_item(&[], index, Value::Object(Vec::new())));
                        cx.notify();
                    },
                )),
            );
        }

        let count = if self.query.is_empty() {
            String::new()
        } else if self.search_hits == 0 {
            "No matches".to_string()
        } else {
            format!("{} found", self.search_hits)
        };
        bar.child(div().flex_1())
            .child(
                div()
                    .w(px(200.0))
                    .h(px(24.0))
                    .min_w(px(96.0))
                    .rounded(px(5.0))
                    .border_1()
                    .border_color(theme.border)
                    .bg(theme.bg)
                    .overflow_hidden()
                    .child(self.search_editor.clone()),
            )
            .child(
                div()
                    .w(px(72.0))
                    .flex_shrink_0()
                    .text_size(px(11.0))
                    .text_color(theme.muted)
                    .child(count),
            )
    }

    fn render_banner(&self, theme: &Theme, cx: &mut Context<Self>) -> Option<Div> {
        if self.error.is_none() && self.notice.is_none() {
            return None;
        }
        let mut col = div()
            .flex()
            .flex_col()
            .gap_1()
            .flex_shrink_0()
            .px_3()
            .py_1()
            .border_b_1()
            .border_color(theme.border)
            .bg(theme.surface);
        if let Some(err) = self.error.as_ref() {
            col = col.child(
                div()
                    .flex()
                    .flex_row()
                    .items_center()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.0))
                            .text_size(px(12.0))
                            .text_color(theme.danger)
                            .child(format!("Not valid JSON: {err}")),
                    )
                    .child(
                        button(theme, "json-repair", "Repair", false)
                            .on_click(cx.listener(|this, _: &ClickEvent, _, cx| this.repair(cx))),
                    ),
            );
        }
        if let Some((message, is_error)) = self.notice.as_ref() {
            let colour = if *is_error { theme.danger } else { theme.muted };
            col = col.child(
                div()
                    .text_size(px(12.0))
                    .text_color(colour)
                    .child(message.clone()),
            );
        }
        Some(col)
    }

    fn render_text(&self) -> Div {
        div()
            .flex_1()
            .min_h(px(0.0))
            .overflow_hidden()
            .child(self.text_editor.clone())
    }

    fn render_tree(&self, theme: &Theme, cx: &mut Context<Self>) -> Div {
        if self.doc.is_none() {
            return message_box(theme, "This is not valid JSON. Fix the syntax, or use Repair above.");
        }
        if self.tree_rows.is_empty() {
            return message_box(theme, "Nothing matches the search.");
        }
        let view = cx.entity();
        let list = uniform_list(
            "json-tree-rows",
            self.tree_rows.len(),
            move |range: Range<usize>, window: &mut Window, app: &mut App| {
                view.update(app, |this, cx| this.tree_rows_for(range, window, cx))
            },
        )
        .track_scroll(self.tree_scroll.clone())
        .size_full();
        div()
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_hidden()
            .child(list)
    }

    fn render_table(&self, theme: &Theme, cx: &mut Context<Self>) -> AnyElement {
        if self.doc.is_none() {
            return message_box(theme, "This is not valid JSON. Fix the syntax, or use Repair above.").into_any_element();
        }
        if let Some(message) = self.table_error() {
            return message_box(theme, &message).into_any_element();
        }
        if self.table_cols.is_empty() && self.table_rows.is_empty() {
            return message_box(theme, "The list is empty. Use + Row to add an object.").into_any_element();
        }
        if self.table_rows.is_empty() {
            return message_box(theme, "Nothing matches the search.").into_any_element();
        }

        let total = INDEX_W + COL_W * self.table_cols.len() as f32 + ACTIONS_W;
        let mut header = div()
            .flex()
            .flex_row()
            .items_center()
            .h(px(ROW_H))
            .flex_shrink_0()
            .border_b_1()
            .border_color(theme.border)
            .text_size(px(11.0))
            .text_color(theme.muted)
            .child(div().w(px(INDEX_W)).flex_shrink_0().px_2().child("#"));
        for column in &self.table_cols {
            header = header.child(
                div()
                    .w(px(COL_W))
                    .flex_shrink_0()
                    .px_2()
                    .truncate()
                    .child(column.clone()),
            );
        }
        header = header.child(div().w(px(ACTIONS_W)).flex_shrink_0());

        let view = cx.entity();
        let list = uniform_list(
            "json-table-rows",
            self.table_rows.len(),
            move |range: Range<usize>, window: &mut Window, app: &mut App| {
                view.update(app, |this, cx| this.table_rows_for(range, window, cx))
            },
        )
        .track_scroll(self.table_scroll.clone())
        .size_full();

        let inner = div()
            .w(px(total))
            .h_full()
            .flex()
            .flex_col()
            .child(header)
            .child(div().flex_1().min_h(px(0.0)).w_full().child(list));

        div()
            .id("json-table-scroll")
            .flex_1()
            .min_h(px(0.0))
            .w_full()
            .overflow_x_scroll()
            .child(inner)
            .into_any_element()
    }

    /// Pushes search highlights into the Text editor. Only the Text view shows
    /// them, so this runs once per query or edit and only in that view.
    fn sync_text_search(&mut self, cx: &mut Context<Self>) {
        if !self.search_dirty || self.mode != Mode::Text {
            return;
        }
        self.search_dirty = false;
        let ranges = if self.query.is_empty() {
            Vec::new()
        } else {
            self.text_editor.read(cx).find_ranges(&self.query)
        };
        self.search_hits = ranges.len();
        self.text_editor
            .update(cx, |editor, cx| editor.set_search(ranges, None, cx));
    }
}

#[allow(clippy::too_many_arguments)]
fn walk_rows(
    value: &Value,
    expanded: &HashSet<JsonPath>,
    path: &mut JsonPath,
    depth: usize,
    key: Option<&str>,
    index: Option<usize>,
    filter: Option<&str>,
    out: &mut Vec<TreeRow>,
    hits: &mut usize,
) {
    let self_hit = filter.is_some_and(|q| node_matches(value, key, q));
    let below = filter.is_some_and(|q| has_match_below(value, q));
    if filter.is_some() && !self_hit && !below {
        return;
    }
    if self_hit {
        *hits += 1;
    }
    let container = value.is_container();
    let open = container && (expanded.contains(path) || (filter.is_some() && below));
    let label = match (key, index) {
        (Some(k), _) => Some(k.to_string()),
        (None, Some(i)) => Some(i.to_string()),
        _ => None,
    };
    out.push(TreeRow {
        path: path.clone(),
        depth,
        label,
        is_index: index.is_some(),
        kind: value.kind(),
        preview: preview_of(value),
        open,
        matched: self_hit,
    });
    if !open {
        return;
    }
    match value {
        Value::Array(items) => {
            for (i, item) in items.iter().enumerate() {
                path.push(i);
                walk_rows(item, expanded, path, depth + 1, None, Some(i), filter, out, hits);
                path.pop();
            }
        }
        Value::Object(entries) => {
            for (i, (k, item)) in entries.iter().enumerate() {
                path.push(i);
                walk_rows(item, expanded, path, depth + 1, Some(k), None, filter, out, hits);
                path.pop();
            }
        }
        _ => {}
    }
}

fn node_matches(value: &Value, key: Option<&str>, query: &str) -> bool {
    if key.is_some_and(|k| k.to_lowercase().contains(query)) {
        return true;
    }
    !value.is_container() && value.to_compact().to_lowercase().contains(query)
}

fn has_match_below(value: &Value, query: &str) -> bool {
    match value {
        Value::Array(items) => items
            .iter()
            .any(|item| node_matches(item, None, query) || has_match_below(item, query)),
        Value::Object(entries) => entries.iter().any(|(k, item)| {
            node_matches(item, Some(k), query) || has_match_below(item, query)
        }),
        _ => false,
    }
}

fn clip(text: String, max: usize) -> String {
    if text.chars().count() <= max {
        text
    } else {
        let mut clipped: String = text.chars().take(max).collect();
        clipped.push('…');
        clipped
    }
}

/// One-line summary of a value: scalars as JSON text, containers by size.
fn preview_of(value: &Value) -> String {
    match value {
        Value::Object(entries) if entries.is_empty() => "{}".to_string(),
        Value::Object(entries) => format!(
            "{{ {} {} }}",
            entries.len(),
            if entries.len() == 1 { "key" } else { "keys" }
        ),
        Value::Array(items) if items.is_empty() => "[]".to_string(),
        Value::Array(items) => format!(
            "[ {} {} ]",
            items.len(),
            if items.len() == 1 { "item" } else { "items" }
        ),
        _ => clip(value.to_compact(), MAX_PREVIEW),
    }
}

fn value_colour(kind: Kind, theme: &Theme) -> Rgba {
    match kind {
        Kind::String => theme.code,
        Kind::Number => theme.accent,
        Kind::Bool => theme.heading,
        Kind::Null => theme.muted,
        Kind::Array | Kind::Object => theme.muted,
    }
}

fn column_index(columns: &[String], name: &str) -> usize {
    columns.iter().position(|c| c == name).unwrap_or(0)
}

fn button(
    theme: &Theme,
    id: impl Into<ElementId>,
    label: impl Into<SharedString>,
    active: bool,
) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .h(px(24.0))
        .px_2()
        .rounded(px(5.0))
        .border_1()
        .border_color(theme.border)
        .bg(if active { theme.accent_soft } else { theme.surface })
        .text_size(px(12.0))
        .text_color(if active { theme.accent } else { theme.fg })
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(theme.hover))
        .child(label.into())
}

fn icon_button(theme: &Theme, id: impl Into<ElementId>, icon: &'static str) -> Stateful<Div> {
    div()
        .id(id)
        .flex()
        .items_center()
        .justify_center()
        .w(px(20.0))
        .h(px(20.0))
        .rounded(px(4.0))
        .cursor(CursorStyle::PointingHand)
        .hover(|s| s.bg(theme.hover))
        .child(app_icon::icon(icon, 13.0).text_color(theme.muted))
}

fn message_box(theme: &Theme, message: &str) -> Div {
    div()
        .flex_1()
        .min_h(px(0.0))
        .flex()
        .items_center()
        .justify_center()
        .p_4()
        .text_size(px(13.0))
        .text_color(theme.muted)
        .child(message.to_string())
}

impl Render for JsonView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let theme = Theme::for_name(&self.theme_name, window.appearance());
        match self.mode {
            Mode::Tree => self.refresh_tree(),
            Mode::Table => self.refresh_table(),
            Mode::Text => self.sync_text_search(cx),
        }
        let body: AnyElement = match self.mode {
            Mode::Text => self.render_text().into_any_element(),
            Mode::Tree => self.render_tree(&theme, cx).into_any_element(),
            Mode::Table => self.render_table(&theme, cx),
        };
        let banner = self.render_banner(&theme, cx);
        let toolbar = self.render_toolbar(&theme, cx);
        div()
            .track_focus(&self.focus)
            .flex()
            .flex_col()
            .size_full()
            .overflow_hidden()
            .bg(theme.bg)
            .text_color(theme.fg)
            .child(toolbar)
            .when_some(banner, |el, banner| el.child(banner))
            .child(body)
    }
}

impl Focusable for JsonView {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        match self.mode {
            Mode::Text => self.text_editor.read(cx).focus_handle(cx),
            Mode::Tree | Mode::Table => self.focus.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn round_trip(text: &str) -> String {
        parse(text).expect("parses").to_compact()
    }

    #[test]
    fn big_integers_and_number_spelling_survive() {
        assert_eq!(round_trip("12345678901234567890"), "12345678901234567890");
        assert_eq!(round_trip("1.0"), "1.0");
        assert_eq!(round_trip("1e400"), "1e400");
        assert_eq!(round_trip("-0.0"), "-0.0");
        assert_eq!(round_trip("1E+5"), "1E+5");
        assert_eq!(
            round_trip(r#"{"id":12345678901234567890,"price":1.0,"big":1e400}"#),
            r#"{"id":12345678901234567890,"price":1.0,"big":1e400}"#
        );
        let pretty = parse("[12345678901234567890, 1.0]").unwrap().to_pretty("  ");
        assert_eq!(pretty, "[\n  12345678901234567890,\n  1.0\n]");
    }

    #[test]
    fn keys_keep_order_and_duplicates() {
        let v = parse(r#"{"z":1,"a":2,"z":3}"#).unwrap();
        assert_eq!(v.to_compact(), r#"{"z":1,"a":2,"z":3}"#);
        assert_eq!(v.key_at(&[2]), Some("z"));
    }

    #[test]
    fn pretty_is_two_spaces_and_compact_has_no_space() {
        let v = parse(r#"{"a":[1,{"b":null}],"c":{},"d":[]}"#).unwrap();
        assert_eq!(
            v.to_pretty("  "),
            "{\n  \"a\": [\n    1,\n    {\n      \"b\": null\n    }\n  ],\n  \"c\": {},\n  \"d\": []\n}"
        );
        assert_eq!(v.to_compact(), r#"{"a":[1,{"b":null}],"c":{},"d":[]}"#);
    }

    #[test]
    fn strict_rejects_what_repair_fixes() {
        for bad in [
            "[1,2,]",
            "{\"a\":1,}",
            "{'a':1}",
            "{a:1}",
            "{\"a\":1 // note\n}",
            "/* c */ {\"a\":1}",
            "{\"a\":01}",
            "{\"a\":1} x",
            "",
        ] {
            assert!(parse(bad).is_err(), "strict should reject {bad:?}");
        }
        let fixed = parse_lenient("{\n  // comment\n  a: 'it\\'s', /* block */\n  \"list\": [1, 2,],\n}")
            .expect("lenient parse");
        assert_eq!(fixed.to_compact(), r#"{"a":"it's","list":[1,2]}"#);
        let big = parse_lenient("{'id': 12345678901234567890,}").unwrap();
        assert_eq!(big.to_compact(), r#"{"id":12345678901234567890}"#);
    }

    #[test]
    fn errors_report_line_and_column() {
        let e = parse("{\n  \"a\": x\n}").unwrap_err();
        assert_eq!((e.line, e.column), (2, 8));
        assert!(e.to_string().starts_with("line 2, column 8: "));
        let end = parse("[1, 2").unwrap_err();
        assert_eq!(end.message, "Unexpected end of input");
        let trailing = parse("[1,]").unwrap_err();
        assert!(trailing.message.contains("Trailing comma"));
        assert!(parse("{\"a\":1 // x\n}").unwrap_err().message.starts_with("Comments are not allowed"));
        let unterminated = parse("\"abc").unwrap_err();
        assert_eq!((unterminated.line, unterminated.column), (1, 1));
    }

    #[test]
    fn strings_escape_and_unescape() {
        let v = parse(r#""a\"b\\c\n\u0001é😀\ud83d\ude00\/""#).unwrap();
        assert_eq!(v, Value::String("a\"b\\c\n\u{1}é😀😀/".to_string()));
        assert_eq!(v.to_compact(), r#""a\"b\\c\n\u0001é😀😀/""#);
        assert!(parse("\"\\ud83d\"").is_err());
        assert_eq!(parse_lenient("\"\\ud83d\"").unwrap(), Value::String("\u{FFFD}".into()));
        assert!(parse("\"raw\nnewline\"").is_err());
        assert_eq!(
            parse_lenient("\"raw\nnewline\"").unwrap().to_compact(),
            r#""raw\nnewline""#
        );
    }

    #[test]
    fn style_detection_keeps_file_shape() {
        assert_eq!(Style::detect("{\n\t\"a\": 1\n}\n").indent, "\t");
        assert_eq!(Style::detect("{\n    \"a\": 1\n}").indent, "    ");
        assert_eq!(Style::detect("{}").indent, "  ");
        let crlf = Style::detect("{\r\n  \"a\": 1\r\n}\r\n");
        assert!(crlf.crlf && crlf.final_newline);
        assert_eq!(crlf.to_file_text("a\nb\n"), "a\r\nb\r\n");
        let v = parse("{\"a\":1}").unwrap();
        assert_eq!(
            Style { indent: "\t".into(), crlf: false, final_newline: true }.render(&v),
            "{\n\t\"a\": 1\n}\n"
        );
    }

    #[test]
    fn path_get_set_delete() {
        let mut v = parse(r#"{"users":[{"id":12345678901234567890}],"n":1.0}"#).unwrap();
        assert_eq!(v.get(&[0, 0, 0]).map(|x| x.to_compact()), Some("12345678901234567890".into()));
        assert_eq!(v.pointer_path("/users/0/id"), Some(vec![0, 0, 0]));
        assert_eq!(v.pointer_path("/nope"), None);
        v.set(&[1], Value::Number("2.50".into())).unwrap();
        assert_eq!(v.to_compact(), r#"{"users":[{"id":12345678901234567890}],"n":2.50}"#);
        let removed = v.delete(&[0]).unwrap();
        assert_eq!(removed.kind(), Kind::Array);
        assert_eq!(v.to_compact(), r#"{"n":2.50}"#);
        assert!(v.delete(&[]).is_err());
        assert!(v.delete(&[7]).is_err());
        assert!(v.set(&[9, 9], Value::Null).is_err());
    }

    #[test]
    fn insert_rename_duplicate() {
        let mut v = parse(r#"{"a":1,"b":[10]}"#).unwrap();
        let p = v.insert_entry(&[], 1, "x", Value::Bool(true)).unwrap();
        assert_eq!(p, vec![1]);
        assert!(v.insert_entry(&[], 0, "a", Value::Null).is_err());
        assert_eq!(v.to_compact(), r#"{"a":1,"x":true,"b":[10]}"#);

        v.rename_key(&[1], "y").unwrap();
        assert!(v.rename_key(&[1], "a").is_err());
        assert_eq!(v.key_at(&[1]), Some("y"));

        let item = v.insert_item(&[2], 0, Value::String("s".into())).unwrap();
        assert_eq!(item, vec![2, 0]);
        assert!(v.insert_item(&[2], 9, Value::Null).is_err());
        assert!(v.insert_item(&[0], 0, Value::Null).is_err());

        let copy = v.duplicate(&[0]).unwrap();
        assert_eq!(copy, vec![1]);
        assert_eq!(v.key_at(&[1]), Some("a copy"));
        let dup_again = v.duplicate(&[0]).unwrap();
        assert_eq!(v.key_at(&dup_again), Some("a copy 2"));
        let b = v.pointer_path("/b").unwrap();
        let dup_item = v.duplicate(&[b[0], 0]).unwrap();
        assert_eq!(dup_item, vec![b[0], 1]);
        assert!(v.duplicate(&[]).is_err());

        v.put_key(&[], "a", Value::Number("7".into())).unwrap();
        v.put_key(&[], "new", Value::Null).unwrap();
        assert_eq!(v.member("a"), Some(&Value::Number("7".into())));
        assert_eq!(v.member("new"), Some(&Value::Null));
        assert!(v.put_key(&[2], "k", Value::Null).is_err());
    }

    #[test]
    fn sorting() {
        let mut v = parse(r#"{"b":{"d":1,"c":2},"a":[3,1,{"z":1,"y":2}]}"#).unwrap();
        v.sort_keys(true);
        assert_eq!(v.to_compact(), r#"{"a":[3,1,{"y":2,"z":1}],"b":{"c":2,"d":1}}"#);
        v.sort_arrays(true);
        assert_eq!(v.to_compact(), r#"{"a":[1,3,{"y":2,"z":1}],"b":{"c":2,"d":1}}"#);

        let mut mixed = parse(r#"[ "b", 10, null, true, 9, "a", [], {} ]"#).unwrap();
        mixed.sort_arrays(false);
        assert_eq!(mixed.to_compact(), r#"[null,true,9,10,"a","b",[],{}]"#);

        let mut flat = parse(r#"{"b":1,"a":{"d":1,"c":2}}"#).unwrap();
        flat.sort_keys(false);
        assert_eq!(flat.to_compact(), r#"{"a":{"d":1,"c":2},"b":1}"#);
    }

    #[test]
    fn container_paths_and_value_text() {
        let v = parse(r#"{"a":[{}],"b":1}"#).unwrap();
        assert_eq!(v.container_paths(), vec![vec![], vec![0], vec![0, 0]]);
        assert_eq!(parse_value_text(" 12345678901234567890 ").unwrap().to_compact(), "12345678901234567890");
        assert_eq!(parse_value_text("hello").unwrap(), Value::String("hello".into()));
        assert_eq!(parse_value_text("\"x\"").unwrap(), Value::String("x".into()));
        assert_eq!(parse_value_text("true").unwrap(), Value::Bool(true));
        assert!(parse_value_text("{oops").is_err());
        assert!(parse_value_text("   ").is_err());
        assert_eq!(Value::number("1.0"), Some(Value::Number("1.0".into())));
        assert_eq!(Value::number("01"), None);
    }

    #[test]
    fn preview_text() {
        assert_eq!(preview_of(&parse("{}").unwrap()), "{}");
        assert_eq!(preview_of(&parse("[1]").unwrap()), "[ 1 item ]");
        assert_eq!(preview_of(&parse(r#"{"a":1,"b":2}"#).unwrap()), "{ 2 keys }");
        assert_eq!(preview_of(&Value::String("x".repeat(300))).chars().count(), MAX_PREVIEW + 1);
    }

    #[test]
    fn filter_keeps_ancestors_of_matches() {
        let doc = parse(r#"{"users":[{"name":"Ann"},{"name":"Bob"}],"x":{"y":1}}"#).unwrap();
        let expanded = HashSet::new();
        let mut rows = Vec::new();
        let mut hits = 0;
        walk_rows(&doc, &expanded, &mut vec![], 0, None, None, Some("bob"), &mut rows, &mut hits);
        assert_eq!(hits, 1);
        let paths: Vec<JsonPath> = rows.iter().map(|r| r.path.clone()).collect();
        assert_eq!(paths, vec![vec![], vec![0], vec![0, 1], vec![0, 1, 0]]);
        assert!(rows.iter().find(|r| r.path == vec![0, 1, 0]).unwrap().matched);
    }
}
