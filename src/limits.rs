//! Bounds on the work a hostile file can cause.
//!
//! Binary property lists and keyed archives refer to objects by index, so
//! one object can be reached many times. Expanding references into a tree
//! is bounded twice: in depth (which also catches cycles that a writer
//! could hide behind fresh references) and in total size, so that a small
//! file whose arrays all point at the same large array can't expand into
//! gigabytes.

/// The deepest nesting of arrays and dictionaries read. Apple's own files
/// stay within a few dozen levels.
pub(crate) const MAX_DEPTH: usize = 512;

/// Output allowed per byte (binary) or value (archive) of input. Writers
/// share equal strings and numbers between parents, so the expanded tree
/// of an ordinary file is a few times its size; 64 leaves ample room.
const EXPANSION_PER_UNIT: usize = 64;

/// Output allowed whatever the input's size.
const EXPANSION_FLOOR: usize = 1 << 16;

/// The output still allowed: each value costs one, plus its bytes.
pub(crate) struct Budget {
    remaining: usize,
}

impl Budget {
    /// The budget for an input of `units` bytes or values.
    pub(crate) fn for_input(units: usize) -> Self {
        Self {
            remaining: units
                .saturating_mul(EXPANSION_PER_UNIT)
                .saturating_add(EXPANSION_FLOOR),
        }
    }

    /// Takes the cost of one value holding `bytes` bytes, when there is
    /// enough left; `false` (and nothing taken) when there isn't.
    pub(crate) fn spend(&mut self, bytes: usize) -> bool {
        let cost = bytes.saturating_add(1);
        match self.remaining.checked_sub(cost) {
            Some(left) => {
                self.remaining = left;
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spending_stops_at_the_limit() {
        let mut budget = Budget::for_input(0);
        assert!(budget.spend(EXPANSION_FLOOR - 1));
        assert!(!budget.spend(0));
        assert!(!budget.spend(usize::MAX));
    }
}
