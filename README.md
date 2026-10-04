# plist

Apple property lists, read without CoreFoundation: binary (`bplist00`) and XML, as macOS and iOS store preferences, launch agents and daemons, application bundles' `Info.plist`, Safari and Finder state, Bluetooth and Wi-Fi history, backups' manifests, and the NSKeyedArchiver archives inside many of them. Written after Apple's open-source CoreFoundation ([CFBinaryPList.c and CFPropertyList.c](https://github.com/apple-oss-distributions/CF)) and the [property list DTD](https://www.apple.com/DTDs/PropertyList-1.0.dtd). Read-only, and no dependencies but Sootmark's own timestamp type.

```toml
[dependencies]
sootmark-plist = "0.1"
```

```rust
let file = std::fs::read("com.example.agent.plist")?;
let plist = plist::parse(&file)?;
let program = plist.value.get("ProgramArguments").and_then(|v| v.as_array());
println!("{:?}: {:?}", plist.format, program);
for problem in &plist.problems {
    eprintln!("{problem}");
}
// An NSKeyedArchiver archive, with its references resolved.
if let Some(archive) = plist::unarchive(&plist.value) {
    println!("{:?}", archive.value.get("root"));
}
```

## What you get

- `parse(bytes)`: the format, recognised by the first bytes, and the top `Value`: `Bool`, `Integer` (`i128`: binary files hold up to 16 bytes, unsigned 64-bit values among them), `Real`, `Date` (a `Ts`, UTC), `Data`, `String`, `Uid`, `Array`, `Dictionary` (entries in file order, duplicate keys kept), `Null`. Accessors: `get(key)` (the last entry of that key, as Apple's readers have it), `as_str`, `as_bool`, `as_integer`, `as_i64`, `as_u64`, `as_f64`, `as_date`, `as_data`, `as_uid`, `as_array`, `as_dictionary`, `is_null`, `kind`.
- Binary property lists: the header and the 32-byte trailer (offset size, reference size, object count, top object, offset table), and every object marker: null, false, true, integers of 1, 2, 4, 8 and 16 bytes (the narrow ones unsigned, the wide ones signed), reals of 4 and 8 bytes, dates (seconds since 2001-01-01, as a double), data, ASCII and UTF-16BE strings, UIDs of 1 to 16 bytes, arrays, sets and ordered sets (read as arrays), dictionaries; counts of 15 and more in the integer that follows the marker; references of 1 to 8 bytes resolved, shared objects read for each reference.
- XML property lists: `<plist>` (or a value element alone, reported), `<dict>`, `<array>`, `<string>`, `<key>`, `<integer>` (decimal or `0x` hexadecimal, signed), `<real>` (`nan`, `inf`, `infinity` too), `<true/>`, `<false/>`, `<date>` (ISO 8601 in UTC, fields after the year optional, as Apple and Python accept it), `<data>` (base64, broken and indented anywhere); a dictionary of one `CF$UID` integer is a `Uid`, as CoreFoundation reads it. The XML is read by a small tokenizer: the declaration, processing instructions, comments and the `DOCTYPE` (internal subset included) skipped, attributes ignored, CDATA sections, the five predefined entities and numeric character references decoded, UTF-8 or UTF-16 with a byte order mark.
- `unarchive(value)`: an NSKeyedArchiver archive (`$archiver`, `$top`, `$objects`) with every UID replaced by the object it refers to. `NSDictionary`, `NSArray`, `NSSet`, `NSOrderedSet`, `NSString`, `NSData`, `NSDate` and `NSNull`, their mutable forms and their subclasses (found in `$classes`) become the plain values they hold; other objects stay dictionaries of their fields, `$class` naming the class. The result is `$top` resolved: most archives keep their object under `root`.

## Damage is contained

Evidence is hostile. Only a file that isn't a property list, or whose structure can't be used, is an error: neither format, another binary version (`bplist15`, `bplist16`), a trailer whose sizes are invalid or whose offset table or top object lies outside the file, a top object that can't be read, an XML document without a value. The rest is listed in `problems` and skipped: objects at offsets outside the object area, references past the object table, reference cycles, unknown markers and fill bytes where an object should be, counts and lengths larger than the bytes that follow, dictionary keys that aren't strings, duplicate keys, ASCII strings holding other bytes (read as Latin-1), unpaired UTF-16 surrogates; in XML, tags left open or closed twice, the document ending inside an element, text where values go, unknown elements, keys without values and values without keys, integers, reals, dates and base64 that don't parse, unknown entities, bytes that aren't UTF-8 (each with its line). In archives, references to objects being resolved (cycles) and past `$objects` stay UIDs, reported.

Every read ends and memory stays proportional to the input: nesting is bounded at 512 levels, counts are checked against the bytes there are before anything is allocated, and the tree that shared references expand into is bounded (64 values or bytes per byte of input, plus 64 KiB), so a small file whose arrays all point at one large array can't expand into gigabytes.

## Not yet

- The old ASCII (OpenStep) property list format, `{ key = value; }`, and JSON property lists: reported as not a property list.
- `bplist15` and `bplist16`, private formats Apple doesn't document (CoreFoundation's NSXPC and some caches): reported as unsupported.
- Objects the top object doesn't reach (left over by an editor or hidden on purpose) aren't read or reported.
- XML in encodings other than UTF-8 and UTF-16 (Apple writes only UTF-8).
- `unarchive` drops the extra fields of Foundation subclasses (a PyObjC string's `pytype`), and leaves classes it doesn't know (`NSURL`, `NSUUID`, application classes) as their encoded fields, unconverted.

## How it's checked

| Check | Result |
|---|---|
| Generated property lists (`tests/fixtures/gen.py`): every type in both formats written by Python's plistlib and by CoreFoundation (`plutil -convert`), integers at every width boundary from −2⁶³ to 2⁶⁴−1, reals including −0, subnormals, infinities and NaN, dates from year 1 to 9999, data and strings across the extended-count boundary, non-BMP text, markup characters, control characters (which CoreFoundation writes raw into XML), UIDs to 2³², 300-key dictionaries, 40 levels of nesting, 2-byte references and 3-byte offsets (an 81 KB file); NSKeyedArchiver archives of a Foundation object graph written by Swift (`archive.swift`), binary and XML. Two authored samples: a launch agent and an application `Info.plist`, XML and CoreFoundation's binary copies. Against plistlib's reading of every value (`tests/oracle/gen.py`, Python 3.14) | 11 files, all match |
| The 32 property lists in [plaso](https://github.com/log2timeline/plaso)'s test data (Apache-2.0, `tests/fixtures/plaso/`), binary and XML, from macOS and iOS: Safari history and downloads, install history, Bluetooth, AirPort, Time Machine, launchd jobs, an NSKeyedArchiver archive written by PyObjC; against plistlib's reading | the 29 plistlib reads all match (3,636 values across all 40 files); the 3 it can't are checked by hand: a date in year 0 read, a truncated file and an empty `<plist>` reported as errors |
| Keyed archives resolved: the Swift graph (strings, numbers, a date, data, arrays, a set, nested dictionaries, an object shared by two parents, a URL, `NSNull`) in both formats, and plaso's PyObjC archive (subclasses of Foundation classes) | every value as archived |
| Unit tests: every binary marker and extended count by hand; reference cycles, nesting past the limit, references shared until the output limit, offsets, references and counts out of range, unknown markers; XML entities, CDATA, declarations, unclosed and stray tags, deep nesting, malformed scalars, unpaired keys, UTF-16 documents | each read or reported, every read ends |
| Property tests: arbitrary bytes (alone and behind each format's header), arbitrary objects behind a real trailer, every fixture damaged and cut anywhere, then unarchived | no panic, output bounded |

## Licence

MIT or Apache-2.0, at your option. The plaso test files are Apache-2.0 (see `tests/fixtures/plaso/NOTICE`); the generated and authored fixtures are this repository's.
