#![crate_name = "compress"]
#![deny(missing_docs, warnings)]
#![feature(macro_rules, phase, opt_out_copy)]

//! dox (placeholder)

#[phase(plugin, link)]
extern crate log;

#[cfg(test)] extern crate rand;
#[cfg(test)] extern crate test;

/// Public exports
#[cfg(feature="checksum")]
pub use self::checksum::adler::State32 as Adler32;

#[cfg(feature="checksum")]
/// Checksum algorithms. Requires `checksum` feature, enabled by default
// http://en.wikipedia.org/wiki/Checksum
pub mod checksum {
    pub mod adler;
}

#[cfg(feature="bwt")]
pub mod bwt;

#[cfg(feature="flate")]
pub mod flate;

#[cfg(feature="lz4")]
pub mod lz4;

#[cfg(feature="zlib")]
pub mod zlib;

/// Entropy coder family. Requires `entropy` feature, enabled by default
// http://en.wikipedia.org/wiki/Entropy_encoding
#[cfg(feature="entropy")]
pub mod entropy {
    pub mod ari;
}
