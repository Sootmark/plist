//! The values a property list holds, and the accessors that read them.

use std::collections::HashSet;

use common::time::Ts;

/// One value of a property list.
///
/// Dictionaries keep their entries in the order the file stores them, and
/// keep duplicate keys (reported as a problem); [`Value::get`] returns the
/// last, as Apple's readers do.
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `true` or `false`.
    Bool(bool),
    /// An integer. Binary property lists store up to 16 bytes, signed;
    /// unsigned 64-bit values above `i64::MAX` are among them.
    Integer(i128),
    /// A floating-point number (4-byte reals are widened).
    Real(f64),
    /// A date. Property lists count seconds since 2001-01-01 UTC (binary,
    /// to the microsecond here) or write ISO 8601 to the second (XML).
    Date(Ts),
    /// Raw bytes.
    Data(Vec<u8>),
    /// Text.
    String(String),
    /// A reference into an NSKeyedArchiver archive's `$objects` (binary
    /// `UID`, or an XML dictionary holding only `CF$UID`).
    Uid(u64),
    /// An ordered list (binary sets are read as arrays too).
    Array(Vec<Value>),
    /// Key and value pairs, in file order.
    Dictionary(Vec<(String, Value)>),
    /// The binary format's null object, which Apple's writers don't emit;
    /// also NSKeyedArchiver's `$null` once an archive is resolved.
    Null,
}

impl Value {
    /// The value of `key` in a dictionary: the last entry of that key.
    /// `None` for a missing key or a value that isn't a dictionary.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.as_dictionary()?
            .iter()
            .rev()
            .find(|(name, _)| name == key)
            .map(|(_, value)| value)
    }

    /// The text of a string.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::String(text) => Some(text),
            _ => None,
        }
    }

    /// A boolean's value.
    #[must_use]
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(value) => Some(*value),
            _ => None,
        }
    }

    /// An integer's value, at full width.
    #[must_use]
    pub fn as_integer(&self) -> Option<i128> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    /// An integer that fits an `i64`.
    #[must_use]
    pub fn as_i64(&self) -> Option<i64> {
        i64::try_from(self.as_integer()?).ok()
    }

    /// An integer that fits a `u64`.
    #[must_use]
    pub fn as_u64(&self) -> Option<u64> {
        u64::try_from(self.as_integer()?).ok()
    }

    /// A real's value, or an integer's as a real.
    #[must_use]
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Real(value) => Some(*value),
            Self::Integer(value) => Some(*value as f64),
            _ => None,
        }
    }

    /// A date.
    #[must_use]
    pub fn as_date(&self) -> Option<Ts> {
        match self {
            Self::Date(date) => Some(*date),
            _ => None,
        }
    }

    /// The bytes of a data value.
    #[must_use]
    pub fn as_data(&self) -> Option<&[u8]> {
        match self {
            Self::Data(bytes) => Some(bytes),
            _ => None,
        }
    }

    /// An archive reference's index.
    #[must_use]
    pub fn as_uid(&self) -> Option<u64> {
        match self {
            Self::Uid(index) => Some(*index),
            _ => None,
        }
    }

    /// The elements of an array.
    #[must_use]
    pub fn as_array(&self) -> Option<&[Value]> {
        match self {
            Self::Array(elements) => Some(elements),
            _ => None,
        }
    }

    /// The entries of a dictionary, in file order.
    #[must_use]
    pub fn as_dictionary(&self) -> Option<&[(String, Value)]> {
        match self {
            Self::Dictionary(entries) => Some(entries),
            _ => None,
        }
    }

    /// Whether this is the null value.
    #[must_use]
    pub fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The type's name, as property lists spell it (`dict`, `integer`, …).
    #[must_use]
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Bool(_) => "bool",
            Self::Integer(_) => "integer",
            Self::Real(_) => "real",
            Self::Date(_) => "date",
            Self::Data(_) => "data",
            Self::String(_) => "string",
            Self::Uid(_) => "uid",
            Self::Array(_) => "array",
            Self::Dictionary(_) => "dict",
            Self::Null => "null",
        }
    }
}

/// The keys that appear more than once among `entries`, each named once,
/// in the order their second occurrence comes.
pub(crate) fn duplicate_keys(entries: &[(String, Value)]) -> Vec<&str> {
    let mut seen = HashSet::new();
    let mut reported = HashSet::new();
    entries
        .iter()
        .map(|(key, _)| key.as_str())
        .filter(|key| !seen.insert(*key) && reported.insert(*key))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dictionary() -> Value {
        Value::Dictionary(vec![
            ("name".into(), Value::String("first".into())),
            ("size".into(), Value::Integer(i128::from(u64::MAX))),
            ("name".into(), Value::String("last".into())),
        ])
    }

    #[test]
    fn get_returns_the_last_entry_of_a_key() {
        let value = dictionary();
        assert_eq!(value.get("name").and_then(Value::as_str), Some("last"));
        assert_eq!(value.get("missing"), None);
        assert_eq!(Value::Bool(true).get("name"), None);
    }

    #[test]
    fn integers_are_read_at_the_width_asked() {
        let size = dictionary().get("size").cloned().unwrap();
        assert_eq!(size.as_u64(), Some(u64::MAX));
        assert_eq!(size.as_i64(), None);
        assert_eq!(Value::Integer(-1).as_u64(), None);
        assert_eq!(Value::Integer(3).as_f64(), Some(3.0));
    }

    #[test]
    fn duplicate_keys_are_named_once() {
        let mut entries = dictionary().as_dictionary().unwrap().to_vec();
        entries.push(("name".into(), Value::Null));
        assert_eq!(duplicate_keys(&entries), ["name"]);
    }
}
