//! Every fixture read as Python's plistlib reads it (`tests/oracle`):
//! every value of every type, keys in file order, without a problem.

mod support;

use plist::Format;

#[test]
fn every_fixture_matches_the_oracle() {
    let fixtures = support::oracle_fixtures();
    assert!(fixtures.len() >= 39, "{} oracle dumps", fixtures.len());
    let mut scalars = 0;
    for (path, oracle) in &fixtures {
        let plist = plist::parse(&support::fixture(path)).unwrap_or_else(|e| panic!("{path}: {e}"));
        assert!(plist.problems.is_empty(), "{path}: {:?}", plist.problems);
        let ours = support::typed(&plist.value);
        if let Some(difference) = support::first_difference(&ours, oracle, "") {
            panic!("{path}: {difference}");
        }
        scalars += support::count_scalars(oracle);
    }
    eprintln!("{} fixtures, {scalars} scalar values match", fixtures.len());
}

#[test]
fn formats_are_recognised() {
    let format = |path| plist::parse(&support::fixture(path)).unwrap().format;
    assert_eq!(format("generated/types.bplist"), Format::Binary);
    assert_eq!(format("generated/types.xml.plist"), Format::Xml);
    assert_eq!(format("plaso/leading_whitespace.plist"), Format::Xml);
}

#[test]
fn both_writers_agree_on_every_type() {
    // plistlib's XML and CoreFoundation's binary copy of it hold the same
    // values; CoreFoundation orders dictionary keys its own way.
    let read = |path| plist::parse(&support::fixture(path)).unwrap().value;
    let mut xml = read("generated/types.xml.plist");
    let mut binary = read("generated/types.cf.bplist");
    support::sort_keys(&mut xml);
    support::sort_keys(&mut binary);
    assert_eq!(support::typed(&xml), support::typed(&binary));
}
