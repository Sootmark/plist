//! Apple property lists, read without CoreFoundation: binary
//! (`bplist00`) and XML (`<plist version="1.0">`), every object type,
//! dictionaries in file order, and NSKeyedArchiver archives resolved into
//! the object tree they encode.
//!
//! Dependency-free (but for Sootmark's own timestamp type), for evidence:
//! the file is only read, and damage is reported, never a panic.
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! let file = std::fs::read("com.example.agent.plist")?;
//! let plist = plist::parse(&file)?;
//! if let Some(program) = plist.value.get("ProgramArguments").and_then(|v| v.as_array()) {
//!     println!("{:?} runs {program:?}", plist.format);
//! }
//! for problem in &plist.problems {
//!     eprintln!("{problem}");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! An NSKeyedArchiver archive is a plain property list, of objects that
//! refer to each other by [`Value::Uid`]. [`unarchive`] resolves it:
//!
//! ```no_run
//! # fn main() -> Result<(), Box<dyn std::error::Error>> {
//! # let file = std::fs::read("archive.plist")?;
//! let plist = plist::parse(&file)?;
//! if let Some(archive) = plist::unarchive(&plist.value) {
//!     println!("{:?}", archive.value.get("root"));
//! }
//! # Ok(())
//! # }
//! ```
//!
//! Only a file that isn't a property list, or whose structure can't be
//! used (a binary trailer pointing outside the file, a top object that
//! can't be read, an XML document without a value), is an error. The rest
//! is listed in `problems` and skipped: objects at offsets outside the
//! file, references past the object table, reference cycles, unknown
//! markers, counts larger than the bytes that follow, malformed XML,
//! unknown elements, values that don't parse. Nesting is bounded, and so
//! is the tree that shared references expand into, so every read ends
//! and memory stays proportional to the input.

mod base64;
mod binary;
mod date;
mod keyed;
mod limits;
mod value;
mod xml;

pub use common::time::{Precision, Semantic, Ts};
pub use keyed::{unarchive, Unarchived};
pub use value::Value;

/// This crate's version, for records of what parsed them.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// The two encodings Apple writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    /// `bplist00`.
    Binary,
    /// XML, `<plist version="1.0">`.
    Xml,
}

/// A property list read from a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Plist {
    /// Which encoding the file uses.
    pub format: Format,
    /// The top value (most often a dictionary).
    pub value: Value,
    /// Damage met reading the file; the affected values are skipped.
    pub problems: Vec<String>,
}

/// Why a file can't be read as a property list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// Neither a binary property list nor XML.
    NotPlist,
    /// A binary property list of another version than `00` (`bplist15`,
    /// `bplist16` are private formats Apple doesn't document).
    UnsupportedVersion(String),
    /// A binary property list too short for its header and trailer (this
    /// many bytes).
    TooShort(usize),
    /// The trailer's offset size isn't 1 to 8 bytes.
    OffsetSize(u8),
    /// The trailer's object reference size isn't 1 to 8 bytes.
    ReferenceSize(u8),
    /// The trailer's top object isn't among its objects.
    TopObject {
        /// The top object's index.
        top: u64,
        /// How many objects the trailer counts.
        objects: u64,
    },
    /// The offset table isn't between the header and the trailer.
    OffsetTable {
        /// Where the trailer puts it.
        offset: u64,
        /// How many entries it would hold.
        objects: u64,
    },
    /// The top object can't be read (why).
    UnreadableTop(String),
    /// An XML property list holding no value.
    NoValue,
}

impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::NotPlist => f.write_str("not a property list (neither bplist00 nor XML)"),
            Self::UnsupportedVersion(magic) => {
                write!(f, "binary property list version {magic:?} isn't supported")
            }
            Self::TooShort(size) => write!(
                f,
                "{size} bytes, too short for a binary property list's header and trailer"
            ),
            Self::OffsetSize(size) => write!(f, "invalid offset size {size} in the trailer"),
            Self::ReferenceSize(size) => {
                write!(f, "invalid object reference size {size} in the trailer")
            }
            Self::TopObject { top, objects } => {
                write!(f, "top object {top} isn't among the {objects} objects")
            }
            Self::OffsetTable { offset, objects } => write!(
                f,
                "an offset table of {objects} entries at {offset:#x} doesn't fit the file"
            ),
            Self::UnreadableTop(reason) => write!(f, "the top object can't be read: {reason}"),
            Self::NoValue => f.write_str("the property list holds no value"),
        }
    }
}

impl std::error::Error for Error {}

/// Reads a property list, binary or XML (UTF-8, or UTF-16 with a byte
/// order mark), recognised by its first bytes.
///
/// # Errors
///
/// When the bytes are neither format, or the file's structure can't be
/// used: see [`Error`].
pub fn parse(bytes: &[u8]) -> Result<Plist, Error> {
    if bytes.starts_with(binary::MAGIC) {
        let (value, problems) = binary::parse(bytes)?;
        return Ok(Plist {
            format: Format::Binary,
            value,
            problems,
        });
    }
    if let Some(version) = bytes.strip_prefix(b"bplist") {
        let version = version.get(..2).unwrap_or(version);
        return Err(Error::UnsupportedVersion(format!(
            "bplist{}",
            String::from_utf8_lossy(version)
        )));
    }
    let (value, problems) = xml::parse(bytes)?;
    Ok(Plist {
        format: Format::Xml,
        value,
        problems,
    })
}
