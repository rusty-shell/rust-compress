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

