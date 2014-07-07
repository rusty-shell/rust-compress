#![crate_name = "compress#0.1"]
#![crate_type = "rlib"]
#![crate_type = "dylib"]
#![comment = "Various compression algorithms written in rust."]
#![deny(missing_doc)]
#![feature(macro_rules, phase)]

//! dox (placeholder)

#[phase(plugin, link)]
extern crate log;
extern crate debug;

#[cfg(test)] extern crate rand;
#[cfg(test)] extern crate test;

/// Public exports
pub use Adler32 = self::checksum::adler::State32;

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
