#[crate_id = "compress"];
#[crate_type = "rlib"];
#[crate_type = "dylib"];
#[deny(warnings, missing_doc)];
#[feature(macro_rules)];

//! dox (placeholder)

#[cfg(test)] extern crate extra;

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
