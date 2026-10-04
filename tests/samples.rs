//! What the fixtures hold, checked value by value where the oracle can't
//! speak (plistlib can't read them) or where meaning matters: dates out of
//! Python's range, files cut short or empty, a launch agent, and keyed
//! archives resolved.

mod support;

use plist::{Error, Value};

fn read(path: &str) -> plist::Plist {
    plist::parse(&support::fixture(path)).unwrap()
}

fn date(value: Option<&Value>) -> String {
    value.and_then(Value::as_date).unwrap().to_string()
}

#[test]
fn dates_out_of_pythons_range_are_read() {
    let plist = read("plaso/com.apple.security.KCN.plist");
    assert!(plist.problems.is_empty(), "{:?}", plist.problems);
    let value = &plist.value;
    // Year 0 of the proleptic Gregorian calendar (macOS shows it as
    // 0001-01-01, in the Julian calendar).
    assert_eq!(
        date(value.get("applicationDate")),
        "0000-12-30T00:00:00.0000000Z"
    );
    assert_eq!(
        date(value.get("lastWritten")),
        "2019-09-16T18:42:44.0000000Z"
    );
    assert_eq!(
        date(value.get("pendingApplicationReminder")),
        "4001-01-01T00:00:00.0000000Z"
    );
    assert_eq!(
        value
            .get("pendingApplicationReminderInterval")
            .and_then(Value::as_i64),
        Some(86_400)
    );
}

#[test]
fn a_truncated_binary_file_is_an_error() {
    let error = plist::parse(&support::fixture("plaso/truncated.plist")).unwrap_err();
    assert!(
        matches!(error, Error::OffsetSize(_) | Error::OffsetTable { .. }),
        "{error}"
    );
}

#[test]
fn an_empty_plist_element_holds_no_value() {
    assert_eq!(
        plist::parse(&support::fixture("plaso/empty.plist")),
        Err(Error::NoValue)
    );
}

#[test]
fn a_launch_agent_reads_the_same_in_both_formats() {
    let xml = read("authored/com.example.updater.plist");
    let mut binary = read("authored/com.example.updater.bplist");
    assert!(xml.problems.is_empty() && binary.problems.is_empty());
    let mut sorted = xml.value.clone();
    support::sort_keys(&mut sorted);
    support::sort_keys(&mut binary.value);
    // Equal but for the dates' precision: seconds in XML, microseconds
    // in binary.
    assert_eq!(support::typed(&sorted), support::typed(&binary.value));
    let agent = &xml.value;
    let arguments: Vec<_> = agent
        .get("ProgramArguments")
        .and_then(Value::as_array)
        .unwrap()
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert_eq!(
        arguments,
        [
            "/Users/Shared/.updater/updater",
            "--check",
            "--url=https://updates.example.com/feed?channel=beta&arch=arm64",
            "--banner=<b>Update ready</b> & restart",
        ]
    );
    assert_eq!(agent.get("RunAtLoad").and_then(Value::as_bool), Some(true));
    assert_eq!(
        agent.get("ThrottleInterval").and_then(Value::as_i64),
        Some(60)
    );
    assert_eq!(agent.get("Nice").and_then(Value::as_i64), Some(-5));
    assert_eq!(
        agent
            .get("EnvironmentVariables")
            .and_then(|env| env.get("DYLD_INSERT_LIBRARIES"))
            .and_then(Value::as_str),
        Some("/Users/Shared/.updater/libhook.dylib")
    );
    assert_eq!(
        date(agent.get("LastModified")),
        "2026-09-30T22:15:07.0000000Z"
    );
    assert_eq!(
        agent.get("SignatureHint").and_then(Value::as_data),
        Some(&b"Sootmark test fixture, not a signature."[..])
    );
    assert_eq!(
        agent.get("Description").and_then(Value::as_str),
        Some("Mise \u{e0} jour automatique \u{2014} \"beta\" <channel> it's")
    );
}

/// The archive `tests/fixtures/generated/archive.swift` writes, resolved.
fn check_swift_archive(path: &str) {
    let plist = read(path);
    let archive = plist::unarchive(&plist.value).unwrap();
    assert!(
        archive.problems.is_empty(),
        "{path}: {:?}",
        archive.problems
    );
    let root = archive.value.get("root").unwrap();
    let get = |key| root.get(key).unwrap_or_else(|| panic!("{path}: no {key}"));
    let strings = |value: &Value| -> Vec<String> {
        value
            .as_array()
            .unwrap()
            .iter()
            .map(|v| v.as_str().unwrap().to_owned())
            .collect()
    };
    assert_eq!(get("title").as_str(), Some("Quarterly notes"));
    assert_eq!(get("count").as_i64(), Some(42));
    assert_eq!(get("ratio").as_f64(), Some(0.25));
    assert_eq!(get("flag").as_bool(), Some(true));
    assert_eq!(
        get("created").as_date().unwrap().to_string(),
        "2023-03-08T20:26:40.5000000Z"
    );
    assert_eq!(get("blob").as_data(), Some(&[0xde, 0xad, 0xbe, 0xef][..]));
    assert_eq!(strings(get("tags")), ["a", "b", "shared by two parents"]);
    assert_eq!(strings(get("unique")), ["x"]);
    let nested = get("nested");
    assert_eq!(
        nested.get("again").and_then(Value::as_str),
        Some("shared by two parents")
    );
    assert_eq!(
        nested.get("unicode").and_then(Value::as_str),
        Some("caf\u{e9} \u{1F600}")
    );
    let url = get("url");
    assert_eq!(url.get("$class").and_then(Value::as_str), Some("NSURL"));
    assert_eq!(
        url.get("NS.relative").and_then(Value::as_str),
        Some("https://example.com/report?id=7")
    );
    assert_eq!(get("empty"), &Value::Null);
}

#[test]
fn foundations_archives_are_resolved_in_both_formats() {
    check_swift_archive("generated/keyed-archive.bplist");
    check_swift_archive("generated/keyed-archive.xml.plist");
}

#[test]
fn subclasses_of_foundation_classes_are_resolved() {
    // Written by PyObjC: its string classes are NSString subclasses.
    let plist = read("plaso/NSKeyedArchiver.plist");
    let archive = plist::unarchive(&plist.value).unwrap();
    assert!(archive.problems.is_empty(), "{:?}", archive.problems);
    let root = archive.value.get("root").unwrap();
    let keys: Vec<_> = root
        .as_dictionary()
        .unwrap()
        .iter()
        .map(|(key, _)| key.as_str())
        .collect();
    assert!(keys.contains(&"MyArray"), "{keys:?}");
}

#[test]
fn plain_property_lists_are_not_archives() {
    assert_eq!(plist::unarchive(&read("authored/Info.plist").value), None);
}
