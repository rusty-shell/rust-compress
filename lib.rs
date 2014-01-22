#[crate_id = "compress"];
#[crate_type = "rlib"];
#[deny(warnings, missing_doc)];
#[feature(macro_rules)];

//! dox (placeholder)

extern mod extra;

pub mod bwt;
pub mod dc;
pub mod flate;
pub mod lz4;

/// Entropy coder family
//http://en.wikipedia.org/wiki/Entropy_encoding
pub mod entropy {
	pub mod ari;
}
