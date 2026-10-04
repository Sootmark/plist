//! NSKeyedArchiver archives: a property list whose `$objects` array holds
//! every archived object once, objects referring to each other by UID
//! (an index into `$objects`), and whose `$top` dictionary names the
//! archived roots. Each object that isn't a plain value is a dictionary
//! whose `$class` refers to a class description, `$classname` and the
//! superclass chain in `$classes`.
//!
//! Resolving replaces every UID with the object it refers to. Foundation's
//! collections, strings, data, dates and `NSNull` (and their subclasses,
//! found in `$classes`) become the plain values they hold; other objects
//! stay dictionaries of their encoded fields, with `$class` naming their
//! class. A reference to an object being resolved (archives
//! of object graphs have cycles) stays a UID, as do references past
//! `$objects`; both are reported. Shared objects are resolved once per
//! reference, within the same depth and size bounds as binary files.

use crate::date;
use crate::limits::{Budget, MAX_DEPTH};
use crate::value::Value;

/// An archive resolved into the tree of values it encodes.
#[derive(Debug, Clone, PartialEq)]
pub struct Unarchived {
    /// The `$top` dictionary, its references resolved. Archives written by
    /// `NSKeyedArchiver.archivedData(withRootObject:)` keep their object
    /// under `root`.
    pub value: Value,
    /// What couldn't be resolved, and why.
    pub problems: Vec<String>,
}

/// Resolves an NSKeyedArchiver archive (a property list with `$top` and
/// `$objects`); `None` for any other value.
#[must_use]
pub fn unarchive(archive: &Value) -> Option<Unarchived> {
    let top = archive.get("$top")?.as_dictionary()?;
    let all_objects = archive.get("$objects")?;
    let objects = all_objects.as_array()?;
    let mut resolver = Resolver {
        objects,
        resolving: vec![false; objects.len()],
        budget: Budget::for_input(count_values(all_objects)),
        exhausted: false,
        problems: Vec::new(),
    };
    let entries = top
        .iter()
        .map(|(key, value)| (key.clone(), resolver.resolve(value, 0)))
        .collect();
    Some(Unarchived {
        value: Value::Dictionary(entries),
        problems: resolver.problems,
    })
}

/// The values in a tree: a container and everything in it.
fn count_values(value: &Value) -> usize {
    1 + match value {
        Value::Array(elements) => elements.iter().map(count_values).sum(),
        Value::Dictionary(entries) => entries.iter().map(|(_, value)| count_values(value)).sum(),
        _ => 0,
    }
}

/// How Foundation's classes encode what they hold.
enum Encoding {
    /// `NS.keys` and `NS.objects`, as many of each.
    Dictionary,
    /// `NS.objects`.
    Collection,
    /// The value of one field.
    Field(&'static str),
    /// `NS.time`, seconds since 2001-01-01.
    Date,
    /// Nothing: `NSNull`.
    Null,
}

/// The classes whose objects become plain values, and how they're held.
const FOUNDATION: &[(&str, Encoding)] = &[
    ("NSDictionary", Encoding::Dictionary),
    ("NSMutableDictionary", Encoding::Dictionary),
    ("NSArray", Encoding::Collection),
    ("NSMutableArray", Encoding::Collection),
    ("NSSet", Encoding::Collection),
    ("NSMutableSet", Encoding::Collection),
    ("NSOrderedSet", Encoding::Collection),
    ("NSMutableOrderedSet", Encoding::Collection),
    ("NSString", Encoding::Field("NS.string")),
    ("NSMutableString", Encoding::Field("NS.string")),
    ("NSData", Encoding::Field("NS.data")),
    ("NSMutableData", Encoding::Field("NS.data")),
    ("NSDate", Encoding::Date),
    ("NSNull", Encoding::Null),
];

/// The string that stands for nil.
const NULL: &str = "$null";

struct Resolver<'a> {
    objects: &'a [Value],
    /// The objects being resolved, from the top down to the current one.
    resolving: Vec<bool>,
    budget: Budget,
    exhausted: bool,
    problems: Vec<String>,
}

impl Resolver<'_> {
    /// `value` with the references in it resolved.
    fn resolve(&mut self, value: &Value, depth: usize) -> Value {
        if depth > MAX_DEPTH {
            self.problems.push(format!(
                "nested deeper than {MAX_DEPTH} levels, left unresolved"
            ));
            return value.clone();
        }
        if !self.spend() {
            return Value::Null;
        }
        match value {
            Value::Uid(index) => self.reference(*index, depth),
            Value::Array(elements) => Value::Array(
                elements
                    .iter()
                    .map(|element| self.resolve(element, depth + 1))
                    .collect(),
            ),
            Value::Dictionary(entries) => Value::Dictionary(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), self.resolve(value, depth + 1)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }

    /// The object a UID refers to, resolved.
    fn reference(&mut self, index: u64, depth: usize) -> Value {
        let Some(slot) = usize::try_from(index)
            .ok()
            .filter(|&slot| slot < self.objects.len())
        else {
            self.problems.push(format!(
                "UID {index} is past the archive's {} objects, left unresolved",
                self.objects.len()
            ));
            return Value::Uid(index);
        };
        if self.resolving[slot] {
            self.problems
                .push(format!("object {index} contains itself, left unresolved"));
            return Value::Uid(index);
        }
        self.resolving[slot] = true;
        let objects = self.objects;
        let object = self.object(&objects[slot], depth);
        self.resolving[slot] = false;
        object
    }

    /// An entry of `$objects`, resolved.
    fn object(&mut self, object: &Value, depth: usize) -> Value {
        if object.as_str() == Some(NULL) {
            return Value::Null;
        }
        let Some(class) = object.get("$class").and_then(Value::as_uid) else {
            return self.resolve(object, depth + 1);
        };
        let classes = self.classes(class);
        let encoding = classes.iter().find_map(|class| {
            FOUNDATION
                .iter()
                .find(|(name, _)| name == class)
                .map(|(_, encoding)| encoding)
        });
        let decoded = encoding.and_then(|encoding| self.decode(object, encoding, depth));
        decoded.unwrap_or_else(|| self.generic(object, classes.first(), depth))
    }

    /// The class names of a class description: `$classname`, then the
    /// superclasses in `$classes`.
    fn classes(&self, class: u64) -> Vec<String> {
        let description = usize::try_from(class)
            .ok()
            .and_then(|slot| self.objects.get(slot));
        let Some(description) = description else {
            return Vec::new();
        };
        let name = description.get("$classname").and_then(Value::as_str);
        let chain = description
            .get("$classes")
            .and_then(Value::as_array)
            .unwrap_or_default()
            .iter()
            .filter_map(Value::as_str);
        name.into_iter().chain(chain).map(str::to_owned).collect()
    }

    /// A Foundation object as the plain value it holds; `None` when its
    /// fields aren't as expected.
    fn decode(&mut self, object: &Value, encoding: &Encoding, depth: usize) -> Option<Value> {
        match encoding {
            Encoding::Dictionary => {
                let keys = object.get("NS.keys")?.as_array()?;
                let values = object.get("NS.objects")?.as_array()?;
                Some(self.dictionary(keys, values, depth))
            }
            Encoding::Collection => {
                let elements = object.get("NS.objects")?.as_array()?;
                let elements = elements
                    .iter()
                    .map(|element| self.resolve(element, depth + 1))
                    .collect();
                Some(Value::Array(elements))
            }
            Encoding::Field(name) => Some(self.resolve(object.get(name)?, depth + 1)),
            Encoding::Date => {
                let seconds = object.get("NS.time")?.as_f64()?;
                Some(Value::Date(date::from_reference_seconds(seconds)))
            }
            Encoding::Null => Some(Value::Null),
        }
    }

    fn dictionary(&mut self, keys: &[Value], values: &[Value], depth: usize) -> Value {
        if keys.len() != values.len() {
            self.problems.push(format!(
                "a dictionary of {} keys and {} values; the extra are dropped",
                keys.len(),
                values.len()
            ));
        }
        let mut entries = Vec::with_capacity(keys.len().min(values.len()));
        for (key, value) in keys.iter().zip(values) {
            match self.resolve(key, depth + 1) {
                Value::String(key) => entries.push((key, self.resolve(value, depth + 1))),
                other => {
                    let kind = other.kind();
                    self.problems
                        .push(format!("a dictionary key that is a {kind}, entry dropped"));
                }
            }
        }
        Value::Dictionary(entries)
    }

    /// Any other object: its fields resolved, `$class` naming its class.
    fn generic(&mut self, object: &Value, class: Option<&String>, depth: usize) -> Value {
        let fields = object.as_dictionary().unwrap_or_default();
        let class = class.map_or(Value::Null, |name| Value::String(name.clone()));
        let entries = fields
            .iter()
            .map(|(key, value)| match key.as_str() {
                "$class" => (key.clone(), class.clone()),
                _ => (key.clone(), self.resolve(value, depth + 1)),
            })
            .collect();
        Value::Dictionary(entries)
    }

    /// Takes one value from the budget, reporting once when it runs out.
    fn spend(&mut self) -> bool {
        if !self.exhausted && self.budget.spend(0) {
            return true;
        }
        if !self.exhausted {
            self.exhausted = true;
            self.problems.push(
                "the archive expands past the output limit (objects shared too often); \
                 the rest is left as null"
                    .to_owned(),
            );
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn string(text: &str) -> Value {
        Value::String(text.into())
    }

    fn dict(entries: &[(&str, Value)]) -> Value {
        Value::Dictionary(
            entries
                .iter()
                .map(|(key, value)| ((*key).to_owned(), value.clone()))
                .collect(),
        )
    }

    fn class(name: &str) -> Value {
        dict(&[
            ("$classname", string(name)),
            (
                "$classes",
                Value::Array(vec![string(name), string("NSObject")]),
            ),
        ])
    }

    fn archive(objects: Vec<Value>) -> Value {
        dict(&[
            ("$archiver", string("NSKeyedArchiver")),
            ("$top", dict(&[("root", Value::Uid(1))])),
            ("$objects", Value::Array(objects)),
            ("$version", Value::Integer(100_000)),
        ])
    }

    #[test]
    fn foundation_objects_become_plain_values() {
        let objects = vec![
            string(NULL),
            dict(&[
                ("$class", Value::Uid(2)),
                ("NS.keys", Value::Array(vec![Value::Uid(3), Value::Uid(4)])),
                (
                    "NS.objects",
                    Value::Array(vec![Value::Uid(5), Value::Uid(0)]),
                ),
            ]),
            class("NSMutableDictionary"),
            string("when"),
            string("nothing"),
            dict(&[("$class", Value::Uid(6)), ("NS.time", Value::Real(0.0))]),
            class("NSDate"),
        ];
        let resolved = unarchive(&archive(objects)).unwrap();
        assert!(resolved.problems.is_empty(), "{:?}", resolved.problems);
        let root = resolved.value.get("root").unwrap();
        assert_eq!(
            root.get("when")
                .and_then(Value::as_date)
                .map(|ts| ts.to_string()),
            Some("2001-01-01T00:00:00.0000000Z".into())
        );
        assert_eq!(root.get("nothing"), Some(&Value::Null));
    }

    #[test]
    fn other_objects_keep_their_fields_and_class() {
        let objects = vec![
            string(NULL),
            dict(&[("$class", Value::Uid(2)), ("name", Value::Uid(3))]),
            class("Note"),
            string("hello"),
        ];
        let resolved = unarchive(&archive(objects)).unwrap();
        assert_eq!(
            resolved.value.get("root"),
            Some(&dict(&[
                ("$class", string("Note")),
                ("name", string("hello"))
            ]))
        );
    }

    #[test]
    fn cycles_and_dangling_references_stay_uids() {
        let objects = vec![
            string(NULL),
            dict(&[
                ("$class", Value::Uid(2)),
                (
                    "NS.objects",
                    Value::Array(vec![Value::Uid(1), Value::Uid(99)]),
                ),
            ]),
            class("NSArray"),
        ];
        let resolved = unarchive(&archive(objects)).unwrap();
        assert_eq!(
            resolved.value.get("root"),
            Some(&Value::Array(vec![Value::Uid(1), Value::Uid(99)]))
        );
        assert_eq!(resolved.problems.len(), 2);
    }

    #[test]
    fn shared_objects_expand_within_the_budget() {
        // Each array holds the next one twice: 2^60 values once expanded.
        let mut objects = vec![string(NULL)];
        for index in 1..=60u64 {
            objects.push(Value::Array(vec![
                Value::Uid(index + 1),
                Value::Uid(index + 1),
            ]));
        }
        objects.push(string("leaf"));
        let resolved = unarchive(&archive(objects)).unwrap();
        assert!(resolved.problems[0].contains("output limit"));
    }

    #[test]
    fn other_values_are_not_archives() {
        assert_eq!(unarchive(&Value::Array(vec![])), None);
        assert_eq!(unarchive(&dict(&[("$top", Value::Null)])), None);
    }
}
