//! Binary property lists built by hand: every marker, and the damage a
//! writer never produces.

use super::*;

/// A `bplist00` file of these objects (2-byte offsets, 1-byte refs).
fn bplist(objects: &[Vec<u8>], top: u64) -> Vec<u8> {
    let mut file = MAGIC.to_vec();
    let mut offsets = Vec::new();
    for object in objects {
        offsets.push(file.len() as u16);
        file.extend(object);
    }
    let table = file.len() as u64;
    for offset in offsets {
        file.extend(offset.to_be_bytes());
    }
    file.extend([0; 6]);
    file.extend([2, 1]);
    file.extend((objects.len() as u64).to_be_bytes());
    file.extend(top.to_be_bytes());
    file.extend(table.to_be_bytes());
    file
}

fn read(objects: &[Vec<u8>]) -> (Value, Vec<String>) {
    parse(&bplist(objects, 0)).unwrap()
}

/// An array (object 0) of objects 1 to `count`.
fn array_of(count: u8) -> Vec<u8> {
    let mut array = vec![0xA0 | count];
    array.extend(1..=count);
    array
}

fn elements(objects: &[Vec<u8>]) -> (Vec<Value>, Vec<String>) {
    let mut all = vec![array_of(objects.len() as u8)];
    all.extend(objects.iter().cloned());
    let (value, problems) = read(&all);
    (value.as_array().unwrap().to_vec(), problems)
}

#[test]
fn every_scalar_marker() {
    let mut sixteen = vec![0x14];
    sixteen.extend([0; 8]);
    sixteen.extend(u64::MAX.to_be_bytes());
    let mut minus_sixteen = vec![0x14];
    minus_sixteen.extend([0xFF; 16]);
    let (values, problems) = elements(&[
        vec![0x00],
        vec![0x08],
        vec![0x09],
        vec![0x10, 0xFF],
        vec![0x11, 0xFF, 0xFF],
        vec![0x12, 0xFF, 0xFF, 0xFF, 0xFF],
        [vec![0x13], (-2i64).to_be_bytes().to_vec()].concat(),
        sixteen,
        minus_sixteen,
        [vec![0x22], 1.5f32.to_be_bytes().to_vec()].concat(),
        [vec![0x23], (-0.25f64).to_be_bytes().to_vec()].concat(),
        [vec![0x33], 1.0f64.to_be_bytes().to_vec()].concat(),
        vec![0x80, 0x07],
        [vec![0x8F], vec![0; 8], 300u64.to_be_bytes().to_vec()].concat(),
    ]);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        values[..9],
        [
            Value::Null,
            Value::Bool(false),
            Value::Bool(true),
            Value::Integer(255),
            Value::Integer(65_535),
            Value::Integer(4_294_967_295),
            Value::Integer(-2),
            Value::Integer(i128::from(u64::MAX)),
            Value::Integer(-1),
        ]
    );
    assert_eq!(values[9], Value::Real(1.5));
    assert_eq!(values[10], Value::Real(-0.25));
    assert_eq!(
        values[11].as_date().unwrap().to_string(),
        "2001-01-01T00:00:01.0000000Z"
    );
    assert_eq!(values[12..], [Value::Uid(7), Value::Uid(300)]);
}

#[test]
fn strings_and_data_with_extended_counts() {
    let long = b"twenty bytes of data";
    let (values, problems) = elements(&[
        [vec![0x4F, 0x10, 20], long.to_vec()].concat(),
        [vec![0x55], b"ascii".to_vec()].concat(),
        // "é😀": one BMP character and a surrogate pair.
        vec![0x63, 0x00, 0xE9, 0xD8, 0x3D, 0xDE, 0x00],
        [vec![0x5F, 0x11, 0x00, 0x03], b"abc".to_vec()].concat(),
        vec![0x50],
    ]);
    assert!(problems.is_empty(), "{problems:?}");
    assert_eq!(
        values,
        [
            Value::Data(long.to_vec()),
            Value::String("ascii".into()),
            Value::String("é\u{1F600}".into()),
            Value::String("abc".into()),
            Value::String(String::new()),
        ]
    );
}

#[test]
fn dictionaries_keep_file_order_and_sets_read_as_arrays() {
    let (value, problems) = read(&[
        vec![0xD2, 1, 2, 3, 4],
        vec![0x51, b'b'],
        vec![0x51, b'a'],
        vec![0xC1, 5],
        vec![0x09],
        vec![0xA0],
    ]);
    assert!(problems.is_empty(), "{problems:?}");
    let entries = value.as_dictionary().unwrap();
    assert_eq!(entries[0].0, "b");
    assert_eq!(entries[0].1, Value::Array(vec![Value::Array(vec![])]));
    assert_eq!(entries[1], ("a".into(), Value::Bool(true)));
}

#[test]
fn shared_objects_are_read_for_each_reference() {
    let (values, problems) = elements(&[vec![0x51, b'x'], vec![0xA2, 1, 1]]);
    assert!(problems.is_empty());
    let x = Value::String("x".into());
    assert_eq!(values[1], Value::Array(vec![x.clone(), x]));
}

fn has(problems: &[String], text: &str) -> bool {
    problems.iter().any(|problem| problem.contains(text))
}

#[test]
fn a_reference_cycle_is_dropped_and_reported() {
    let (value, problems) = read(&[vec![0xA2, 1, 0], vec![0xA1, 0]]);
    assert_eq!(value, Value::Array(vec![Value::Array(vec![])]));
    assert!(has(&problems, "object 0: contains itself"), "{problems:?}");
    assert_eq!(problems.len(), 2);
}

#[test]
fn nesting_is_bounded() {
    // Object n is an array holding object n + 1: deeper than the limit.
    let count = MAX_DEPTH + 10;
    let mut objects: Vec<Vec<u8>> = (1..count)
        .map(|next| [vec![0xA1], (next as u16).to_be_bytes().to_vec()].concat())
        .collect();
    objects.push(vec![0x09]);
    let mut file = bplist(&objects, 0);
    let trailer = file.len() - TRAILER_LEN;
    file[trailer + 7] = 2; // 2-byte references
    let (_, problems) = parse(&file).unwrap();
    assert!(has(&problems, "nested deeper than 512"), "{problems:?}");
}

#[test]
fn shared_references_expand_within_the_budget() {
    // Object n holds object n + 1 twice, 60 levels: 2^60 values expanded.
    let mut objects: Vec<Vec<u8>> = (1..=60u8).map(|next| vec![0xA2, next, next]).collect();
    objects.push(vec![0x09]);
    let (_, problems) = read(&objects);
    assert_eq!(problems.len(), 1);
    assert!(has(&problems, "output limit"));
}

#[test]
fn damaged_objects_are_dropped_and_reported() {
    let (values, problems) = elements(&[
        vec![0x70],
        vec![0x0F],
        vec![0x15, 0],
        vec![0x21, 0, 0],
        vec![0x4F, 0x13, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF],
        vec![0x4F, 0x23],
        vec![0x8F; 2],
        vec![0xA1, 99],
        vec![0x08],
    ]);
    assert_eq!(values, [Value::Bool(false)]);
    for text in [
        "unknown marker 0x70",
        "a fill byte",
        "integer of 2^5 bytes",
        "real of 2^1 bytes",
        "run past the object area",
        "the count after the marker isn't an integer",
        "refers to object 99",
    ] {
        assert!(has(&problems, text), "{text}: {problems:?}");
    }
    assert_eq!(problems.len(), 8, "{problems:?}");
}

#[test]
fn offsets_outside_the_object_area_are_reported() {
    let mut file = bplist(&[array_of(1), vec![0x09]], 0);
    let table = file.len() - TRAILER_LEN - 4;
    file[table + 2..table + 4].copy_from_slice(&0xFFFFu16.to_be_bytes());
    let (value, problems) = parse(&file).unwrap();
    assert_eq!(value, Value::Array(vec![]));
    assert!(has(&problems, "offset 0xffff is outside"), "{problems:?}");
}

#[test]
fn keys_must_be_strings_and_duplicates_are_reported() {
    let (value, problems) = read(&[
        vec![0xD3, 1, 2, 1, 3, 3, 3],
        vec![0x51, b'k'],
        vec![0x10, 0x05],
        vec![0x09],
    ]);
    assert_eq!(value.as_dictionary().unwrap().len(), 2);
    assert!(has(&problems, "the key is of type integer"), "{problems:?}");
    assert!(has(&problems, "key \"k\" appears more than once"));
}

#[test]
fn odd_text_is_kept_and_reported() {
    let (values, problems) = elements(&[vec![0x51, 0xE9], vec![0x61, 0xD8, 0x00]]);
    assert_eq!(
        values,
        [Value::String("é".into()), Value::String("\u{FFFD}".into())]
    );
    assert_eq!(problems.len(), 2);
}

#[test]
fn unusable_trailers_are_errors() {
    let file = bplist(&[vec![0x09]], 0);
    let trailer = file.len() - TRAILER_LEN;
    assert_eq!(parse(&file[..39]), Err(Error::TooShort(39)));
    let damaged = |at: usize, value: u8| {
        let mut file = file.clone();
        file[trailer + at] = value;
        parse(&file).err()
    };
    assert_eq!(damaged(6, 0), Some(Error::OffsetSize(0)));
    assert_eq!(damaged(7, 9), Some(Error::ReferenceSize(9)));
    assert_eq!(
        damaged(23, 1),
        Some(Error::TopObject { top: 1, objects: 1 })
    );
    assert_eq!(
        damaged(31, 0xF0),
        Some(Error::OffsetTable {
            offset: 0xF0,
            objects: 1
        })
    );
    assert_eq!(
        damaged(15, 0xFF),
        Some(Error::OffsetTable {
            offset: 9,
            objects: 0xFF
        })
    );
}

#[test]
fn an_unreadable_top_object_is_an_error() {
    assert!(matches!(
        parse(&bplist(&[vec![0x70]], 0)),
        Err(Error::UnreadableTop(_))
    ));
}
