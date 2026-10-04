// Writes an NSKeyedArchiver archive of one object graph, binary and XML,
// into the directory given: `swift archive.swift <directory>` (gen.py runs
// it). Foundation strings, numbers, dates, data, arrays, sets, nested
// dictionaries, a URL, NSNull and an object shared by two parents.
import Foundation
let shared = NSMutableString(string: "shared by two parents")
let root: NSDictionary = [
    "title": "Quarterly notes",
    "count": NSNumber(value: 42),
    "ratio": NSNumber(value: 0.25),
    "flag": NSNumber(value: true),
    "created": NSDate(timeIntervalSinceReferenceDate: 700_000_000.5),
    "blob": NSData(bytes: [0xde, 0xad, 0xbe, 0xef] as [UInt8], length: 4),
    "tags": NSArray(array: ["a", "b", shared]),
    "unique": NSSet(array: ["x"]),
    "nested": NSDictionary(dictionary: ["again": shared, "unicode": "caf\u{e9} \u{1F600}"]),
    "url": NSURL(string: "https://example.com/report?id=7")!,
    "empty": NSNull(),
]
for (format, name) in [(PropertyListSerialization.PropertyListFormat.binary, "keyed-archive.bplist"), (.xml, "keyed-archive.xml.plist")] {
    let archiver = NSKeyedArchiver(requiringSecureCoding: false)
    archiver.outputFormat = format
    archiver.encode(root, forKey: NSKeyedArchiveRootObjectKey)
    archiver.finishEncoding()
    try! archiver.encodedData.write(to: URL(fileURLWithPath: CommandLine.arguments[1] + "/" + name))
}
