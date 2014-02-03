#[crate_id = "compress"];
#[crate_type = "rlib"];
#[deny(warnings)];
#[deny(missing_doc)];

//! dox (placeholder)

extern mod extra;

pub mod bwt;
pub mod dc;
//mod flate;
pub mod lz4;

/// Entropy coder family
//http://en.wikipedia.org/wiki/Entropy_encoding
pub mod entropy {
	pub mod ari;
}
