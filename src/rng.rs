use core::ops::{BitXor, Range};

/// An implementation of XorShift
pub struct XorShift {
    /// The seed for the rng
    seed: usize
}

impl XorShift {
    /// Create a new instance of XorShift, with the given seed
    pub fn new(seed: usize) -> Self {
        Self {
            seed,
        }
    }

    /// Generate the next number in the sequence, advancing the seed
    pub fn gen(&mut self) -> usize {
        let x = self.seed;
        let x = x.bitxor(x << 13);
        let x = x.bitxor(x >> 7);
        let x = x.bitxor(x << 17);
        self.seed = x;
        x
    }

    /// Generate a number in the given range
    pub fn gen_range(&mut self, rng: Range<usize>) -> usize {
        (self.gen() + rng.start) % rng.end
    }
}
