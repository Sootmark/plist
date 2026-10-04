//! XML property lists, written by hand: every element, and the damage
//! Apple's writer never produces.

use super::*;

fn document(body: &str) -> String {
    format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \
         \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n\
         <plist version=\"1.0\">\n{body}\n</plist>\n"
    )
}

fn read(body: &str) -> (Value, Vec<String>) {
    parse(document(body).as_bytes()).unwrap()
}

fn read_clean(body: &str) -> Value {
    let (value, problems) = read(body);
    assert!(problems.is_empty(), "{problems:?}");
    value
}

fn has(problems: &[String], text: &str) -> bool {
    problems.iter().any(|problem| problem.contains(text))
}

#[test]
fn every_element() {
    let value = read_clean(
        "<dict>
            <key>s</key><string> a &amp; b </string>
            <key>i</key><integer>-42</integer>
            <key>h</key><integer>0x1F</integer>
            <key>big</key><integer>18446744073709551615</integer>
            <key>r</key><real>2.5</real>
            <key>inf</key><real>-infinity</real>
            <key>t</key><true/>
            <key>f</key><false></false>
            <key>d</key><date>2019-09-16T18:42:44Z</date>
            <key>b</key><data>
                3q2+
                7w==
            </data>
            <key>a</key><array><string/><dict/></array>
            <key>u</key><dict><key>CF$UID</key><integer>7</integer></dict>
        </dict>",
    );
    let get = |key| value.get(key).unwrap();
    assert_eq!(get("s").as_str(), Some(" a & b "));
    assert_eq!(get("i").as_i64(), Some(-42));
    assert_eq!(get("h").as_i64(), Some(31));
    assert_eq!(get("big").as_u64(), Some(u64::MAX));
    assert_eq!(get("r").as_f64(), Some(2.5));
    assert_eq!(get("inf").as_f64(), Some(f64::NEG_INFINITY));
    assert_eq!(get("t").as_bool(), Some(true));
    assert_eq!(get("f").as_bool(), Some(false));
    assert_eq!(
        get("d").as_date().unwrap().to_string(),
        "2019-09-16T18:42:44.0000000Z"
    );
    assert_eq!(get("b").as_data(), Some(&[0xde, 0xad, 0xbe, 0xef][..]));
    assert_eq!(
        get("a"),
        &Value::Array(vec![
            Value::String(String::new()),
            Value::Dictionary(vec![])
        ])
    );
    assert_eq!(get("u"), &Value::Uid(7));
}

#[test]
fn dictionaries_keep_file_order() {
    let value = read_clean("<dict><key>z</key><true/><key>a</key><false/></dict>");
    let keys: Vec<_> = value
        .as_dictionary()
        .unwrap()
        .iter()
        .map(|(key, _)| key.as_str())
        .collect();
    assert_eq!(keys, ["z", "a"]);
}

#[test]
fn values_that_dont_parse_are_skipped_and_reported() {
    let (value, problems) = read(
        "<array>
            <integer>12abc</integer>
            <integer>0x</integer>
            <integer>340282366920938463463374607431768211456</integer>
            <real>one</real>
            <date>yesterday</date>
            <data>3q*2+</data>
            <true>yes</true>
            <blob/>
        </array>",
    );
    assert_eq!(
        value,
        Value::Array(vec![Value::Data(vec![0xde, 0xad, 0xbe]), Value::Bool(true)])
    );
    assert_eq!(problems.len(), 8, "{problems:?}");
    assert!(has(&problems, "line 5: <integer> \"12abc\" doesn't parse"));
    assert!(has(&problems, "unknown element <blob>"));
}

#[test]
fn unpaired_keys_and_values_are_reported() {
    let (value, problems) = read(
        "<dict>
            <key>lonely</key>
            <key>kept</key><true/>
            <false/>
            stray
            <key>dangling</key>
        </dict>",
    );
    assert_eq!(
        value,
        Value::Dictionary(vec![("kept".into(), Value::Bool(true))])
    );
    assert!(has(&problems, "key \"lonely\" has no value"));
    assert!(has(&problems, "a bool without a key"));
    assert!(has(&problems, "text inside <dict>"));
    assert!(has(&problems, "key \"dangling\" has no value"));
}

#[test]
fn a_truncated_document_keeps_what_it_holds() {
    let text = document("<dict><key>a</key><array><string>x</string><string>y");
    let cut = text.find("</plist>").unwrap();
    let (value, problems) = parse(&text.as_bytes()[..cut]).unwrap();
    assert_eq!(
        value.get("a"),
        Some(&Value::Array(vec![
            Value::String("x".into()),
            Value::String("y\n".into())
        ]))
    );
    assert!(has(&problems, "the document ends inside <string>"));
}

#[test]
fn a_value_without_plist_is_read_and_reported() {
    let (value, problems) = parse(b"  <array><true/></array>").unwrap();
    assert_eq!(value, Value::Array(vec![Value::Bool(true)]));
    assert!(has(&problems, "no <plist> element"));
}

#[test]
fn utf16_documents_are_decoded() {
    let text = document("<string>caf\u{e9}</string>");
    let mut little = vec![0xFF, 0xFE];
    little.extend(text.encode_utf16().flat_map(u16::to_le_bytes));
    let mut big = vec![0xFE, 0xFF];
    big.extend(text.encode_utf16().flat_map(u16::to_be_bytes));
    for bytes in [little, big] {
        let (value, problems) = parse(&bytes).unwrap();
        assert_eq!(value, Value::String("café".into()));
        assert!(problems.is_empty());
    }
}

#[test]
fn documents_without_a_value_are_errors() {
    assert_eq!(parse(document("").as_bytes()), Err(Error::NoValue));
    assert_eq!(parse(document("<blob/>").as_bytes()), Err(Error::NoValue));
    assert_eq!(parse(b"<html><body/></html>"), Err(Error::NotPlist));
    assert_eq!(parse(b"{ a = b; }"), Err(Error::NotPlist));
    assert_eq!(parse(b"<!-- nothing -->"), Err(Error::NotPlist));
}

#[test]
fn integers_of_every_form() {
    let parsed = |text| integer(text).and_then(|value| value.as_integer());
    assert_eq!(parsed(" +7 "), Some(7));
    assert_eq!(parsed("-0X10"), Some(-16));
    assert_eq!(
        parsed("-170141183460469231731687303715884105728"),
        Some(i128::MIN)
    );
    assert_eq!(parsed("170141183460469231731687303715884105728"), None);
    assert_eq!(parsed("--1"), None);
    assert_eq!(parsed("0x-1"), None);
    assert_eq!(parsed(""), None);
}
