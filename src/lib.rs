#![deny(missing_docs)]
#![allow(missing_copy_implementations)]
#![allow(deprecated)]

//! dox (placeholder)

extern crate byteorder;
extern crate rand;

#[macro_use]
extern crate log;

#[cfg(test)]
#[cfg(feature="unstable")]
extern crate test;

use std::io::{self, Read};

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

#[cfg(feature="rle")]
pub mod rle;

#[cfg(any(feature = "lz4", feature = "entropy", feature = "bwt"))]
fn byteorder_err_to_io(err: io::Error) -> io::Error {
    match err {
        e if e.kind() == io::ErrorKind::UnexpectedEof =>
            io::Error::new(
                io::ErrorKind::Other,
                "unexpected end of file"
            ),
        e => e,
    }
}

#[cfg(test)]
mod test {
    use super::{io,byteorder_err_to_io};
    #[cfg(feature="unstable")]
    use test;
    
    fn force_byteorder_eof_error()->io::Result<u64>{
        use byteorder::{BigEndian,ReadBytesExt};
        let mut rdr = io::Cursor::new(vec![1,2]);
        rdr.read_u64::<BigEndian>()
    }
    
    #[test]
    fn byteorder_err_to_io_with_eof() {
    
        let err_from_byteorder = force_byteorder_eof_error().unwrap_err();
        let err = byteorder_err_to_io(err_from_byteorder);
        
        let err_expected = io::Error::new(
            io::ErrorKind::Other,
            "unexpected end of file"
        );
        assert_eq!(err.kind(),err_expected.kind());
    }
    
    #[test]
    fn byteorder_err_to_io_with_not_eof() {
   
        // using closure here to produce 2x the same error,
        // as io::Error does not impl Copy trait
        let build_other_io_error = || io::Error::new(
            io::ErrorKind::NotFound,
            "some other io error"
        );
         
        let err = byteorder_err_to_io(build_other_io_error());
        let err_expected = build_other_io_error();
        
        assert_eq!(err.kind(),err_expected.kind());
    }
}


/// Adds a convenience method for types with the read trait, very similar
/// to push_at_least in the late Reader trait
pub trait ReadExact: Read + Sized {
    /// Appends exact number of bytes to a buffer
    fn push_exactly(&mut self, bytes: u64, buf: &mut Vec<u8>) -> io::Result<()> {
        let n = try!(self.by_ref().take(bytes).read_to_end(buf)) as u64;

        if n < bytes {
            return Err(io::Error::new(
                io::ErrorKind::Other,
                "unexpected end of file"
            ));
        }

        Ok(())
    }
}

impl<T> ReadExact for T where T: Read + Sized {}
