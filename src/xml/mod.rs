//! XML property lists: a `<plist>` element holding one value, written as
//! `<dict>` (alternating `<key>` and value elements), `<array>`,
//! `<string>`, `<integer>` (decimal or `0x` hexadecimal), `<real>`,
//! `<true/>`, `<false/>`, `<date>` (ISO 8601, UTC) and `<data>` (base64).
//!
//! A dictionary holding only a `CF$UID` integer is read as a
//! [`Value::Uid`], as Apple's reader does: that is how keyed archives
//! write references in XML.

mod lexer;
mod tree;

use std::borrow::Cow;

use crate::base64;
use crate::date;
use crate::value::{duplicate_keys, Value};
use crate::Error;
use tree::{Element, Node};

/// Damage met, each at a byte offset into the document's text.
type Problems = Vec<(usize, String)>;

/// Reads a UTF-16 code unit, in one byte order.
type CodeUnit = fn([u8; 2]) -> u16;

/// The key of the single entry of a dictionary that stands for a UID.
const UID_KEY: &str = "CF$UID";

/// The value an XML document holds and the damage met reading it, each
/// problem prefixed with its line.
pub(crate) fn parse(bytes: &[u8]) -> Result<(Value, Vec<String>), Error> {
    let mut problems = Problems::new();
    let text = decode(bytes, &mut problems);
    if !text.trim_start().starts_with('<') {
        return Err(Error::NotPlist);
    }
    let root = tree::build(&text, &mut problems).ok_or(Error::NotPlist)?;
    let value = Reader {
        problems: &mut problems,
    }
    .document(&root)?;
    Ok((value, with_lines(&text, problems)))
}

/// The document as text: UTF-8, or UTF-16 after its byte order mark.
/// Bytes that don't decode become U+FFFD, reported.
fn decode<'a>(bytes: &'a [u8], problems: &mut Problems) -> Cow<'a, str> {
    if let Some(utf8) = bytes.strip_prefix(b"\xEF\xBB\xBF") {
        return utf8_text(utf8, problems);
    }
    let (rest, unit): (&[u8], CodeUnit) = match bytes {
        [0xFE, 0xFF, rest @ ..] => (rest, u16::from_be_bytes),
        [0xFF, 0xFE, rest @ ..] => (rest, u16::from_le_bytes),
        _ => return utf8_text(bytes, problems),
    };
    let units = rest.chunks_exact(2).map(|pair| unit([pair[0], pair[1]]));
    let mut damaged = rest.len() % 2 != 0;
    let text = char::decode_utf16(units)
        .map(|unit| {
            unit.unwrap_or_else(|_| {
                damaged = true;
                char::REPLACEMENT_CHARACTER
            })
        })
        .collect();
    if damaged {
        problems.push((0, "UTF-16 that doesn't decode, replaced by U+FFFD".into()));
    }
    Cow::Owned(text)
}

fn utf8_text<'a>(bytes: &'a [u8], problems: &mut Problems) -> Cow<'a, str> {
    let text = String::from_utf8_lossy(bytes);
    if let Cow::Owned(_) = text {
        problems.push((0, "bytes that aren't UTF-8, replaced by U+FFFD".into()));
    }
    text
}

/// Each problem as `line N: …`.
fn with_lines(text: &str, problems: Problems) -> Vec<String> {
    let line_starts: Vec<usize> = std::iter::once(0)
        .chain(text.match_indices('\n').map(|(at, _)| at + 1))
        .collect();
    problems
        .into_iter()
        .map(|(at, message)| {
            let line = line_starts.partition_point(|&start| start <= at);
            format!("line {line}: {message}")
        })
        .collect()
}

/// Turns elements into values, reporting what doesn't fit.
struct Reader<'p> {
    problems: &'p mut Problems,
}

impl Reader<'_> {
    fn report(&mut self, at: usize, message: impl Into<String>) {
        self.problems.push((at, message.into()));
    }

    /// The value of `<plist>`, or of a root value element without one.
    fn document(&mut self, root: &Element) -> Result<Value, Error> {
        if root.name != "plist" {
            if !is_value(root.name) {
                return Err(Error::NotPlist);
            }
            self.report(root.at, "no <plist> element around the value");
            return self.value(root).ok_or(Error::NoValue);
        }
        let mut values = self.contents(root).into_iter();
        let value = values.next().ok_or(Error::NoValue)?;
        if values.next().is_some() {
            self.report(
                root.at,
                "<plist> holds more than one value; the first is read",
            );
        }
        Ok(value)
    }

    /// The values of a container's child elements; text other than
    /// whitespace between them is reported.
    fn contents(&mut self, container: &Element) -> Vec<Value> {
        let mut values = Vec::new();
        for child in &container.children {
            match child {
                Node::Element(element) => values.extend(self.value(element)),
                Node::Text(text) => self.stray_text(container, text),
            }
        }
        values
    }

    fn stray_text(&mut self, container: &Element, text: &str) {
        if !text.trim().is_empty() {
            let name = container.name;
            self.report(container.at, format!("text inside <{name}> ignored"));
        }
    }

    /// An element's value; `None` (reported) when it isn't one.
    fn value(&mut self, element: &Element) -> Option<Value> {
        match element.name {
            "dict" => Some(self.dictionary(element)),
            "array" => Some(Value::Array(self.contents(element))),
            "string" => Some(Value::String(self.text(element))),
            "key" => {
                self.report(element.at, "<key> outside a dictionary, read as a string");
                Some(Value::String(self.text(element)))
            }
            "true" | "false" => {
                if !self.text(element).trim().is_empty() {
                    self.report(element.at, format!("<{}> holds text", element.name));
                }
                Some(Value::Bool(element.name == "true"))
            }
            "integer" => self.scalar(element, integer),
            "real" => self.scalar(element, |text| text.trim().parse().ok().map(Value::Real)),
            "date" => self.scalar(element, |text| date::from_iso8601(text).map(Value::Date)),
            "data" => Some(self.data(element)),
            name => {
                self.report(element.at, format!("unknown element <{name}> skipped"));
                None
            }
        }
    }

    /// A value parsed from an element's text; `None` (reported) when the
    /// text doesn't parse.
    fn scalar(
        &mut self,
        element: &Element,
        parse: impl Fn(&str) -> Option<Value>,
    ) -> Option<Value> {
        let text = self.text(element);
        let value = parse(&text);
        if value.is_none() {
            let name = element.name;
            self.report(
                element.at,
                format!("<{name}> {text:?} doesn't parse, skipped"),
            );
        }
        value
    }

    fn data(&mut self, element: &Element) -> Value {
        let (bytes, problem) = base64::decode(&self.text(element));
        if let Some(problem) = problem {
            self.report(element.at, format!("<data>: {problem}"));
        }
        Value::Data(bytes)
    }

    /// Alternating `<key>` and value elements.
    fn dictionary(&mut self, element: &Element) -> Value {
        let mut entries = Vec::new();
        let mut key: Option<(String, usize)> = None;
        for child in &element.children {
            let child = match child {
                Node::Element(child) => child,
                Node::Text(text) => {
                    self.stray_text(element, text);
                    continue;
                }
            };
            if child.name == "key" {
                let text = self.text(child);
                if let Some((unpaired, at)) = key.replace((text, child.at)) {
                    self.report(at, format!("key {unpaired:?} has no value"));
                }
                continue;
            }
            let value = self.value(child);
            match (key.take(), value) {
                (Some((key, _)), Some(value)) => entries.push((key, value)),
                (None, Some(value)) => {
                    let kind = value.kind();
                    self.report(child.at, format!("a {kind} without a key, dropped"));
                }
                (_, None) => {}
            }
        }
        if let Some((unpaired, at)) = key {
            self.report(at, format!("key {unpaired:?} has no value"));
        }
        for duplicate in duplicate_keys(&entries) {
            let message = format!("key {duplicate:?} appears more than once");
            self.report(element.at, message);
        }
        uid(&entries).unwrap_or(Value::Dictionary(entries))
    }

    /// The text an element holds; elements inside it are reported.
    fn text(&mut self, element: &Element) -> String {
        let mut text = String::new();
        for child in &element.children {
            match child {
                Node::Text(part) => text.push_str(part),
                Node::Element(inner) => {
                    let (outer, inner) = (element.name, inner.name);
                    self.report(element.at, format!("<{inner}> inside <{outer}> ignored"));
                }
            }
        }
        text
    }
}

/// The element names that hold a value.
fn is_value(name: &str) -> bool {
    matches!(
        name,
        "dict" | "array" | "string" | "integer" | "real" | "true" | "false" | "date" | "data"
    )
}

/// A dictionary of one `CF$UID` integer entry, as the UID it stands for.
fn uid(entries: &[(String, Value)]) -> Option<Value> {
    match entries {
        [(key, value)] if key == UID_KEY => value.as_u64().map(Value::Uid),
        _ => None,
    }
}

/// Decimal, or hexadecimal after `0x`; either signed.
fn integer(text: &str) -> Option<Value> {
    let text = text.trim();
    let (negative, unsigned) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    let (digits, radix) = match unsigned.strip_prefix("0x").or(unsigned.strip_prefix("0X")) {
        Some(hex) => (hex, 16),
        None => (unsigned, 10),
    };
    if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
        return None;
    }
    let magnitude = u128::from_str_radix(digits, radix).ok()?;
    let value = if negative {
        0i128.checked_sub_unsigned(magnitude)?
    } else {
        i128::try_from(magnitude).ok()?
    };
    Some(Value::Integer(value))
}

#[cfg(test)]
mod tests;
