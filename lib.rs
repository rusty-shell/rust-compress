#[crate_id = "compress"];
#[crate_type = "rlib"];
#[crate_type = "dylib"];
#[deny(warnings, missing_doc)];
#[feature(macro_rules)];

//! dox (placeholder)

#[cfg(test)] extern crate extra;

mod adler32;

pub mod bwt;
pub mod dc;
pub mod flate;
pub mod lz4;
pub mod zlib;

/// Entropy coder family
// http://en.wikipedia.org/wiki/Entropy_encoding
pub mod entropy {
	pub mod ari;
}

/// Second step algorithms, designed to leverage BWT-output redundancy
// http://citeseerx.ist.psu.edu/viewdoc/summary?doi=10.1.1.16.2897
pub mod post_bwt {
	pub mod mtf;
}
