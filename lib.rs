#![crate_id = "compress"]
#![crate_type = "rlib"]
#![crate_type = "dylib"]
#![deny(warnings, missing_doc)]
#![feature(macro_rules, phase)]

//! dox (placeholder)

#[phase(syntax, link)] extern crate log;
#[cfg(test)] extern crate rand;
#[cfg(test)] extern crate test;

mod adler32;

pub mod bwt;
pub mod flate;
pub mod lz4;
pub mod zlib;

/// Entropy coder family
// http://en.wikipedia.org/wiki/Entropy_encoding
pub mod entropy {
	pub mod ari;
}
