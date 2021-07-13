//! # swf-rs
//!
//! Library for reading and writing Adobe Flash SWF files.
//!
//! # Organization
//!
//! This library consists of a `read` module for decoding SWF data, and a `write` library for
//! writing SWF data.
#![allow(
    renamed_and_removed_lints,
    clippy::unknown_clippy_lints,
    clippy::unusual_byte_groupings,
    clippy::upper_case_acronyms
)]

extern crate byteorder;
#[cfg(feature = "flate2")]
extern crate flate2;
#[cfg(feature = "libflate")]
extern crate libflate;
#[macro_use]
extern crate num_derive;
extern crate num_traits;

pub mod avm1;
pub mod avm2;
pub mod error;
// TODO: Make this private?
pub mod extensions;
pub mod read;
mod string;
mod tag_code;
mod types;
pub mod write;

#[cfg(test)]
mod test_data;

/// Reexports
pub use read::{decompress_swf, parse_swf};
pub use string::*;
pub use tag_code::TagCode;
pub use types::*;
pub use write::write_swf;
