#![crate_name = "compress"]
#![deny(missing_doc, warnings)]
#![feature(macro_rules, phase)]

//! dox (placeholder)

#[phase(plugin, link)]
extern crate log;

#[cfg(test)] extern crate rand;
#[cfg(test)] extern crate test;

/// Public exports
pub use self::checksum::adler::State32 as Adler32;

/// Checksum algorithms
// http://en.wikipedia.org/wiki/Checksum
pub mod checksum {
    pub mod adler;
}

pub mod bwt;
pub mod flate;
pub mod lz4;
pub mod zlib;

/// Entropy coder family
// http://en.wikipedia.org/wiki/Entropy_encoding
pub mod entropy {
    pub mod ari;
}
