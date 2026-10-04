//! Damage is reported, never a panic, and every read ends: property tests
//! over arbitrary bytes, arbitrary objects behind a real trailer, and
//! every fixture damaged and cut anywhere, read and unarchived; plus the
//! worst shapes by hand (deep nesting, cycles, shared references).

mod support;

use plist::{Error, Value};
use proptest::prelude::*;

/// Every fixture, binary and XML.
const FIXTURES: &[&str] = &[
    "generated/types.bplist",
    "generated/types.xml.plist",
    "generated/types.cf.bplist",
    "generated/types.cf.xml.plist",
    "generated/large.bplist",
    "generated/keyed-archive.bplist",
    "generated/keyed-archive.xml.plist",
    "authored/com.example.updater.plist",
    "authored/com.example.updater.bplist",
    "authored/Info.plist",
    "authored/Info.bplist",
    "plaso/NSKeyedArchiver.plist",
    "plaso/com.apple.airport.preferences.plist",
    "plaso/com.apple.bluetooth.plist",
    "plaso/Downloads.plist",
];

/// Values in a tree, containers included.
fn size(value: &Value) -> usize {
    1 + match value {
        Value::Array(elements) => elements.iter().map(size).sum(),
        Value::Dictionary(entries) => entries.iter().map(|(_, value)| size(value)).sum(),
        _ => 0,
    }
}

/// Reads `bytes` as a whole application would: the property list, then
/// the archive it may be. The tree stays within the expansion bound.
fn read_everything(bytes: &[u8]) {
    let Ok(plist) = plist::parse(bytes) else {
        return;
    };
    assert!(size(&plist.value) <= 64 * bytes.len() + (1 << 16));
    if let Some(archive) = plist::unarchive(&plist.value) {
        let _ = archive.value.get("root");
    }
}

/// A binary trailer: 1-byte offsets and references, `objects` objects,
/// the top one first, the offset table at `table`.
fn trailer(objects: u64, table: u64) -> Vec<u8> {
    let mut trailer = vec![0; 6];
    trailer.extend([1, 1]);
    trailer.extend(objects.to_be_bytes());
    trailer.extend(0u64.to_be_bytes());
    trailer.extend(table.to_be_bytes());
    trailer
}

proptest! {
    #[test]
    fn arbitrary_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..2048)) {
        read_everything(&bytes);
        let mut binary = b"bplist00".to_vec();
        binary.extend(&bytes);
        read_everything(&binary);
        let mut xml = b"<plist version=\"1.0\">".to_vec();
        xml.extend(&bytes);
        read_everything(&xml);
    }

    #[test]
    fn arbitrary_objects_behind_a_real_trailer_never_panic(
        objects in proptest::collection::vec(any::<u8>(), 1..200),
        table in proptest::collection::vec(any::<u8>(), 1..60),
    ) {
        let mut file = b"bplist00".to_vec();
        file.extend(&objects);
        let table_at = file.len() as u64;
        // Offsets pointing anywhere among the objects (or not).
        file.extend(table.iter().map(|&offset| offset.wrapping_add(8)));
        file.extend(trailer(table.len() as u64, table_at));
        read_everything(&file);
    }

    #[test]
    fn damaged_fixtures_never_panic(
        fixture in 0..FIXTURES.len(),
        damage in proptest::collection::vec((any::<prop::sample::Index>(), any::<u8>()), 1..8),
        cut in any::<prop::sample::Index>(),
        truncate in any::<bool>(),
    ) {
        let mut bytes = support::fixture(FIXTURES[fixture]);
        for (at, byte) in damage {
            let at = at.index(bytes.len());
            bytes[at] = byte;
        }
        if truncate {
            bytes.truncate(cut.index(bytes.len()));
        }
        read_everything(&bytes);
    }

    #[test]
    fn fixtures_cut_anywhere_never_panic(fixture in 0..FIXTURES.len(), cut in any::<prop::sample::Index>()) {
        let bytes = support::fixture(FIXTURES[fixture]);
        read_everything(&bytes[..cut.index(bytes.len())]);
    }
}

#[test]
fn deep_xml_nesting_ends_without_overflowing_the_stack() {
    let depth = 100_000;
    let document = format!(
        "<plist>{}{}</plist>",
        "<array>".repeat(depth),
        "</array>".repeat(depth)
    );
    let plist = plist::parse(document.as_bytes()).unwrap();
    assert!(plist.problems.iter().any(|p| p.contains("nested deeper")));
    // Unclosed, too.
    let plist = plist::parse("<array>".repeat(depth).as_bytes()).unwrap();
    assert!(!plist.problems.is_empty());
}

#[test]
fn a_binary_file_whose_top_contains_itself_reads_empty() {
    // Object 0: an array holding object 0.
    let mut file = b"bplist00".to_vec();
    file.extend([0xA1, 0x00]);
    file.push(8);
    file.extend(trailer(1, 10));
    let plist = plist::parse(&file).unwrap();
    assert_eq!(plist.value, Value::Array(vec![]));
    assert_eq!(plist.problems.len(), 1, "{:?}", plist.problems);
}

#[test]
fn huge_counts_allocate_nothing() {
    // An array claiming 2^62 elements in a 50-byte file.
    let mut file = b"bplist00".to_vec();
    file.extend([0xAF, 0x13, 0x40, 0, 0, 0, 0, 0, 0, 0]);
    file.push(8);
    file.extend(trailer(1, 18));
    assert!(matches!(plist::parse(&file), Err(Error::UnreadableTop(_))));
}

#[test]
fn other_formats_are_errors() {
    assert_eq!(plist::parse(b""), Err(Error::NotPlist));
    assert_eq!(plist::parse(b"bplist00"), Err(Error::TooShort(8)));
    assert_eq!(
        plist::parse(b"bplist16\x00"),
        Err(Error::UnsupportedVersion("bplist16".into()))
    );
    assert_eq!(plist::parse(b"{\"json\": true}"), Err(Error::NotPlist));
    assert_eq!(plist::parse(b"\x00\x01\x02"), Err(Error::NotPlist));
}
