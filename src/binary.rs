//! Binary property lists, `bplist00`: an 8-byte header, the objects, a
//! table of their offsets, and a 32-byte trailer saying where the table
//! is, how wide its entries and the object references are, how many
//! objects there are and which one is the top.
//!
//! Each object starts with a marker byte: its type in the high nibble and,
//! for most types, a count in the low one (`0xF` when a following integer
//! object holds a larger count). Containers hold references, object
//! indexes into the offset table, so objects can be shared and a damaged
//! file can make one contain itself: the reader tracks the objects being
//! read, bounds the nesting and the total output, and checks every offset
//! and count against the bytes there are.

use crate::date;
use crate::limits::{Budget, MAX_DEPTH};
use crate::value::{duplicate_keys, Value};
use crate::Error;

/// The header of the only version Apple writes.
pub(crate) const MAGIC: &[u8] = b"bplist00";
const HEADER_LEN: usize = 8;
const TRAILER_LEN: usize = 32;
/// The widest integer object: 2^4 = 16 bytes.
const MAX_INTEGER_WIDTH_LOG2: u8 = 4;
/// Counts in the low nibble stop at 14; 15 means an integer follows.
const EXTENDED_COUNT: u8 = 0xF;

/// The top object's value and the damage met reading the file.
pub(crate) fn parse(bytes: &[u8]) -> Result<(Value, Vec<String>), Error> {
    let trailer = Trailer::read(bytes)?;
    let mut objects = Objects::new(bytes, &trailer);
    match objects.value(trailer.top, 0) {
        Ok(value) => Ok((value, objects.problems)),
        Err(Failure::Damaged(reason)) => Err(Error::UnreadableTop(reason)),
        Err(Failure::Exhausted) => Err(Error::UnreadableTop(EXHAUSTED.to_owned())),
    }
}

/// What the last 32 bytes say.
struct Trailer {
    offset_size: usize,
    ref_size: usize,
    object_count: usize,
    top: usize,
    /// Where the offset table starts; objects lie between the header and it.
    table: usize,
}

impl Trailer {
    fn read(bytes: &[u8]) -> Result<Self, Error> {
        let Some(start) = bytes.len().checked_sub(TRAILER_LEN) else {
            return Err(Error::TooShort(bytes.len()));
        };
        if start < HEADER_LEN {
            return Err(Error::TooShort(bytes.len()));
        }
        let trailer = &bytes[start..];
        let offset_size = trailer[6];
        let ref_size = trailer[7];
        let object_count = u64_at(trailer, 8);
        let top = u64_at(trailer, 16);
        let table = u64_at(trailer, 24);
        if !(1..=8).contains(&offset_size) {
            return Err(Error::OffsetSize(offset_size));
        }
        if !(1..=8).contains(&ref_size) {
            return Err(Error::ReferenceSize(ref_size));
        }
        if top >= object_count {
            return Err(Error::TopObject {
                top,
                objects: object_count,
            });
        }
        let table_end = usize::try_from(object_count)
            .ok()
            .and_then(|count| count.checked_mul(usize::from(offset_size)))
            .and_then(|size| size.checked_add(usize::try_from(table).ok()?));
        let table_fits = table >= HEADER_LEN as u64 && table_end.is_some_and(|end| end <= start);
        if !table_fits {
            return Err(Error::OffsetTable {
                offset: table,
                objects: object_count,
            });
        }
        Ok(Self {
            offset_size: usize::from(offset_size),
            ref_size: usize::from(ref_size),
            // Both fit: the table holding them fits in memory.
            object_count: object_count as usize,
            top: top as usize,
            table: table as usize,
        })
    }
}

fn u64_at(bytes: &[u8], at: usize) -> u64 {
    be_unsigned(&bytes[at..at + 8]) as u64
}

/// A big-endian unsigned integer of up to 16 bytes.
fn be_unsigned(bytes: &[u8]) -> u128 {
    bytes
        .iter()
        .fold(0, |value, &byte| (value << 8) | u128::from(byte))
}

/// Why an object wasn't read.
enum Failure {
    /// The object is damaged; the reason, naming it.
    Damaged(String),
    /// The output limit was reached (reported once, where it happened).
    Exhausted,
}

const EXHAUSTED: &str = "the objects expand past the output limit (references shared too \
     often); the rest of the file is not read";

type Read<T> = Result<T, Failure>;

/// The objects of one file, read on demand.
struct Objects<'a> {
    /// The bytes objects can occupy: everything before the offset table.
    area: &'a [u8],
    table: &'a [u8],
    offset_size: usize,
    ref_size: usize,
    object_count: usize,
    /// The objects being read, from the top down to the current one.
    reading: Vec<bool>,
    budget: Budget,
    exhausted: bool,
    problems: Vec<String>,
}

impl<'a> Objects<'a> {
    fn new(bytes: &'a [u8], trailer: &Trailer) -> Self {
        Self {
            area: &bytes[..trailer.table],
            table: &bytes[trailer.table..],
            offset_size: trailer.offset_size,
            ref_size: trailer.ref_size,
            object_count: trailer.object_count,
            reading: vec![false; trailer.object_count],
            budget: Budget::for_input(bytes.len()),
            exhausted: false,
            problems: Vec::new(),
        }
    }

    /// The value of object `index`, `depth` containers below the top.
    fn value(&mut self, index: usize, depth: usize) -> Read<Value> {
        if depth > MAX_DEPTH {
            return damaged(index, &format!("nested deeper than {MAX_DEPTH} levels"));
        }
        if self.reading[index] {
            return damaged(index, "contains itself (a reference cycle)");
        }
        let offset = self.offset(index)?;
        self.reading[index] = true;
        let value = self.object(index, offset, depth);
        self.reading[index] = false;
        value
    }

    /// Where object `index` starts, checked to be among the objects.
    fn offset(&self, index: usize) -> Read<usize> {
        let at = index * self.offset_size;
        let offset = be_unsigned(&self.table[at..at + self.offset_size]);
        match usize::try_from(offset) {
            Ok(offset) if (HEADER_LEN..self.area.len()).contains(&offset) => Ok(offset),
            _ => damaged(
                index,
                &format!("offset {offset:#x} is outside the object area"),
            ),
        }
    }

    fn object(&mut self, index: usize, offset: usize, depth: usize) -> Read<Value> {
        match self.area[offset] >> 4 {
            // 0xB (ordered set) and 0xC (set) are documented but never
            // written; read as arrays.
            0xA..=0xC => self.array(index, offset, depth),
            0xD => self.dictionary(index, offset, depth),
            _ => {
                let value = self
                    .scalar(index, offset)
                    .map_err(|reason| damaged_at(index, offset, &reason))?;
                self.spend(payload_len(&value))?;
                Ok(value)
            }
        }
    }

    /// Any object but a container.
    fn scalar(&mut self, index: usize, offset: usize) -> Result<Value, String> {
        let marker = self.area[offset];
        let low = marker & 0x0F;
        let at = offset + 1;
        match marker >> 4 {
            0x0 => singleton(marker),
            0x1 => self.integer(low, at),
            0x2 => self.real(low, at),
            0x3 if low == 3 => self.date(at),
            0x4 => Ok(Value::Data(self.data(low, at)?.to_vec())),
            0x5 => self.ascii(index, low, at),
            0x6 => self.utf16(index, low, at),
            0x8 => self.uid(low, at),
            _ => Err(format!("unknown marker {marker:#04x}")),
        }
    }

    /// 1, 2 and 4-byte integers are unsigned, 8 and 16-byte ones signed.
    fn integer(&self, low: u8, at: usize) -> Result<Value, String> {
        if low > MAX_INTEGER_WIDTH_LOG2 {
            return Err(format!("integer of 2^{low} bytes"));
        }
        let width = 1usize << low;
        let unsigned = be_unsigned(self.bytes(at, width)?);
        let value = if width == 8 {
            i128::from(unsigned as u64 as i64)
        } else {
            // 16 bytes wrap to signed; narrower values are unsigned.
            unsigned as i128
        };
        Ok(Value::Integer(value))
    }

    fn real(&self, low: u8, at: usize) -> Result<Value, String> {
        match low {
            2 => {
                let bits = be_unsigned(self.bytes(at, 4)?) as u32;
                Ok(Value::Real(f64::from(f32::from_bits(bits))))
            }
            3 => Ok(Value::Real(self.double(at)?)),
            _ => Err(format!("real of 2^{low} bytes")),
        }
    }

    fn date(&self, at: usize) -> Result<Value, String> {
        Ok(Value::Date(date::from_reference_seconds(self.double(at)?)))
    }

    fn double(&self, at: usize) -> Result<f64, String> {
        Ok(f64::from_bits(be_unsigned(self.bytes(at, 8)?) as u64))
    }

    fn data(&self, low: u8, at: usize) -> Result<&'a [u8], String> {
        let (len, start) = self.count(low, at)?;
        self.bytes(start, len)
    }

    /// ASCII text; other bytes are read as Latin-1 and reported.
    fn ascii(&mut self, index: usize, low: u8, at: usize) -> Result<Value, String> {
        let bytes = self.data(low, at)?;
        if !bytes.is_ascii() {
            self.problems.push(format!(
                "object {index}: an ASCII string holds other bytes, read as Latin-1"
            ));
        }
        Ok(Value::String(
            bytes.iter().map(|&b| char::from(b)).collect(),
        ))
    }

    /// UTF-16 big-endian text, its length counted in code units. Unpaired
    /// surrogates become U+FFFD and are reported.
    fn utf16(&mut self, index: usize, low: u8, at: usize) -> Result<Value, String> {
        let (units, start) = self.count(low, at)?;
        let len = units
            .checked_mul(2)
            .ok_or_else(|| format!("{units} UTF-16 units"))?;
        let bytes = self.bytes(start, len)?;
        let units = bytes
            .chunks_exact(2)
            .map(|pair| u16::from_be_bytes([pair[0], pair[1]]));
        let mut unpaired = false;
        let text = char::decode_utf16(units)
            .map(|unit| {
                unit.unwrap_or_else(|_| {
                    unpaired = true;
                    char::REPLACEMENT_CHARACTER
                })
            })
            .collect();
        if unpaired {
            self.problems.push(format!(
                "object {index}: unpaired UTF-16 surrogates, replaced by U+FFFD"
            ));
        }
        Ok(Value::String(text))
    }

    /// A keyed archive reference of 1 to 16 bytes (the low nibble plus one).
    fn uid(&self, low: u8, at: usize) -> Result<Value, String> {
        let width = usize::from(low) + 1;
        let value = be_unsigned(self.bytes(at, width)?);
        u64::try_from(value)
            .map(Value::Uid)
            .map_err(|_| format!("UID {value} doesn't fit 64 bits"))
    }

    fn array(&mut self, index: usize, offset: usize, depth: usize) -> Read<Value> {
        let low = self.area[offset] & 0x0F;
        let refs = self
            .references(low, offset + 1, 1)
            .map_err(|reason| damaged_at(index, offset, &reason))?;
        self.spend(0)?;
        let mut elements = Vec::with_capacity(refs.len());
        for (position, &child) in refs.iter().enumerate() {
            let context = || format!("element {position} of object {index}");
            elements.extend(self.child(child, depth, context));
            if self.exhausted {
                break;
            }
        }
        Ok(Value::Array(elements))
    }

    /// Key references, then as many value references.
    fn dictionary(&mut self, index: usize, offset: usize, depth: usize) -> Read<Value> {
        let low = self.area[offset] & 0x0F;
        let refs = self
            .references(low, offset + 1, 2)
            .map_err(|reason| damaged_at(index, offset, &reason))?;
        self.spend(0)?;
        let (keys, values) = refs.split_at(refs.len() / 2);
        let mut entries = Vec::with_capacity(keys.len());
        for (position, (&key, &value)) in keys.iter().zip(values).enumerate() {
            let context = || format!("entry {position} of object {index}");
            match self.child(key, depth, context) {
                Some(Value::String(key)) => {
                    if let Some(value) = self.child(value, depth, context) {
                        entries.push((key, value));
                    }
                }
                Some(key) => {
                    let kind = key.kind();
                    self.problems
                        .push(format!("{}: the key is of type {kind}, dropped", context()));
                }
                None => {}
            }
            if self.exhausted {
                break;
            }
        }
        for key in duplicate_keys(&entries) {
            self.problems.push(format!(
                "object {index}: key {key:?} appears more than once"
            ));
        }
        Ok(Value::Dictionary(entries))
    }

    /// A container's child; `None` when damaged (reported, with
    /// `context`) or past the output limit.
    fn child(&mut self, child: usize, depth: usize, context: impl Fn() -> String) -> Option<Value> {
        match self.value(child, depth + 1) {
            Ok(value) => Some(value),
            Err(Failure::Damaged(reason)) => {
                self.problems
                    .push(format!("{reason}; {} dropped", context()));
                None
            }
            Err(Failure::Exhausted) => None,
        }
    }

    /// The object indexes a container holds: `count × per_entry` of them.
    fn references(&self, low: u8, at: usize, per_entry: usize) -> Result<Vec<usize>, String> {
        let (count, start) = self.count(low, at)?;
        let refs = count
            .checked_mul(per_entry)
            .ok_or_else(|| format!("{count} entries"))?;
        let len = refs
            .checked_mul(self.ref_size)
            .ok_or_else(|| format!("{count} entries"))?;
        let bytes = self.bytes(start, len)?;
        bytes
            .chunks_exact(self.ref_size)
            .map(|reference| {
                let index = be_unsigned(reference);
                usize::try_from(index)
                    .ok()
                    .filter(|&index| index < self.object_count)
                    .ok_or_else(|| {
                        format!(
                            "refers to object {index}, past the {} there are",
                            self.object_count
                        )
                    })
            })
            .collect()
    }

    /// An object's count and where its contents start: the low nibble,
    /// or the integer object after the marker when the nibble is 15.
    fn count(&self, low: u8, at: usize) -> Result<(usize, usize), String> {
        if low != EXTENDED_COUNT {
            return Ok((usize::from(low), at));
        }
        let marker = self.bytes(at, 1)?[0];
        if marker >> 4 != 0x1 || marker & 0x0F > 3 {
            return Err(format!(
                "the count after the marker isn't an integer of up to 8 bytes ({marker:#04x})"
            ));
        }
        let width = 1usize << (marker & 0x0F);
        let count = be_unsigned(self.bytes(at + 1, width)?);
        let count = usize::try_from(count).map_err(|_| format!("count {count}"))?;
        Ok((count, at + 1 + width))
    }

    /// `len` bytes at `at`, when they lie within the object area.
    fn bytes(&self, at: usize, len: usize) -> Result<&'a [u8], String> {
        at.checked_add(len)
            .and_then(|end| self.area.get(at..end))
            .ok_or_else(|| {
                format!(
                    "{len} bytes at {at:#x} run past the object area (ends at {:#x})",
                    self.area.len()
                )
            })
    }

    /// Takes one value's output from the budget, reporting once when it
    /// runs out.
    fn spend(&mut self, bytes: usize) -> Read<()> {
        if !self.exhausted && self.budget.spend(bytes) {
            return Ok(());
        }
        if !self.exhausted {
            self.exhausted = true;
            self.problems.push(EXHAUSTED.to_owned());
        }
        Err(Failure::Exhausted)
    }
}

/// Null, false, true; fill bytes and the reserved markers are damage.
fn singleton(marker: u8) -> Result<Value, String> {
    match marker {
        0x00 => Ok(Value::Null),
        0x08 => Ok(Value::Bool(false)),
        0x09 => Ok(Value::Bool(true)),
        0x0F => Err("a fill byte where an object should start".to_owned()),
        _ => Err(format!("unknown marker {marker:#04x}")),
    }
}

/// The bytes a scalar holds, for the output budget.
fn payload_len(value: &Value) -> usize {
    match value {
        Value::Data(bytes) => bytes.len(),
        Value::String(text) => text.len(),
        _ => 0,
    }
}

fn damaged<T>(index: usize, reason: &str) -> Read<T> {
    Err(Failure::Damaged(format!("object {index}: {reason}")))
}

fn damaged_at(index: usize, offset: usize, reason: &str) -> Failure {
    Failure::Damaged(format!("object {index} at {offset:#x}: {reason}"))
}

#[cfg(test)]
mod tests;
