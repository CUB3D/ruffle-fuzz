use core::ops::{BitXor, Range};
use std::convert::TryFrom;

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

    /// Create a new instance of `XorShift` using rdtsc as a seed
    pub fn new_rtdsc() -> Self {
        Self::new(unsafe { core::arch::x86_64::_rdtsc() } as usize)
    }

    /// Generate the next number in the sequence, advancing the seed
    pub fn gen(&mut self) -> usize {
        let x = self.seed;
        let x = x.bitxor(x << 13);
        let x = x.bitxor(x >> 7);
        let x = x.bitxor(x << 17);
        assert_ne!(self.seed, x);
        self.seed = x;
        x
    }

    /// Generate a number in the given range
    pub fn gen_range(&mut self, rng: Range<usize>) -> usize {
        (self.gen().wrapping_add(rng.start).min(rng.start)) % rng.end
    }

    pub fn gen2_range<T: Into<usize> + TryFrom<usize> + Copy>(&mut self, rng: Range<T>) -> T  where <T as TryFrom<usize>>::Error: std::fmt::Debug {
        let gen = self.gen().wrapping_add(rng.start.into()) % rng.end.into();
        T::try_from(gen).expect("Fail")
    }

    /// Select a random item from a slice
    pub fn select<T: Clone>(&mut self, options: &[T]) -> T {
        let index = self.gen_range(0..options.len());
        options[index].clone()
    }

    pub fn one_of<R, T: Fn(&mut XorShift) -> R>(&mut self, fns: &[T]) -> R {
        let index = self.gen_range(0..fns.len());
        let f = &fns[index];
        f(self)
    }

    /// Generate a random bool
    pub fn gen_bool(&mut self) -> bool {
        self.gen_range(0..1) == 1
    }
}
