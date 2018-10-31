//! ZLIB Compression and Decompression. Requires `zlib` feature, enabled by default
//!
//! This module contains an implementation of the ZLIB compression scheme. This
//! compression format is based on an underlying DEFLATE-encoded stream.
//!
//! # Example
//!
//! ```rust,ignore
//! use compress::zlib;
//! use std::fs::File;
//! use std::path::Path;
//!
//! let stream = File::open(&Path::new("path/to/file.flate")).unwrap();
//! let mut decompressed = Vec::new();
//! zlib::Decoder::new(stream).read_to_end(&mut decompressed);
//! ```
//!
//! # Related links
//!
//! * http://tools.ietf.org/html/rfc1950 - RFC that this implementation is based
//!   on

use std::io::{self, Read};
use super::byteorder::{BigEndian, ReadBytesExt};

use Adler32;
use flate;

/// Structure used to decode a ZLIB-encoded stream. The wrapped stream can be
/// re-acquired through the unwrap() method.
pub struct Decoder<R> {
    hash: Adler32,
    inner: flate::Decoder<R>,
    read_header: bool,
}

impl<R: Read> Decoder<R> {
    /// Creates a new ZLIB-stream decoder which will wrap the specified reader.
    /// This decoder also implements the `Reader` trait, and the underlying
    /// reader can be re-acquired through the `unwrap` method.
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            hash: Adler32::new(),
            inner: flate::Decoder::new(r),
            read_header: false,
        }
    }

    /// Destroys this decoder, returning the underlying reader.
    pub fn unwrap(self) -> R {
        self.inner.r
    }

    fn validate_header(&mut self) -> io::Result<()> {
        let cmf = try!(self.inner.r.read_u8());
        let flg = try!(self.inner.r.read_u8());
        if cmf & 0xf != 0x8 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported zlib stream format"
            ))
        }

        if cmf & 0xf0 != 0x70 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported zlib window size"
            ))
        }

        if flg & 0x20 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "unsupported initial dictionary in the output stream"
            ))
        }

        if ((cmf as u16) * 256 + (flg as u16)) % 31 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid zlib header checksum"
            ))
        }
        Ok(())
    }

    /// Tests if this stream has reached the EOF point yet.
    pub fn eof(&self) -> bool { self.inner.eof() }

    #[allow(dead_code)]
    fn reset(&mut self) {
        self.inner.reset();
        self.hash.reset();
        self.read_header = false;
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if !self.read_header {
            try!(self.validate_header());
            self.read_header = true;
        } else if self.inner.eof() {
            return Ok(0);
        }
        match self.inner.read(buf) {
            Ok(0) => {
                let cksum = try!(self.inner.r.read_u32::<BigEndian>());
                if cksum != self.hash.result() {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidInput,
                        "invalid checksum on zlib stream"
                    ))
                }
                else {
                    Ok(0)
                }
            }
            Ok(n) => {
                self.hash.feed(&buf[..n]);
                Ok(n)
            }
            Err(e) => Err(e)
        }
    }
}

#[cfg(test)]
#[allow(warnings)]
mod test {
    use std::io::{BufReader, BufWriter, Read, Write};
    use super::super::rand::{random, Rand};
    use super::super::byteorder::{LittleEndian, BigEndian, WriteBytesExt, ReadBytesExt};
    use std::str;
    use super::{Decoder};
    #[cfg(feature="unstable")]
    use test;

    fn test_decode(input: &[u8], output: &[u8]) {
        let mut d = Decoder::new(BufReader::new(input));
        let mut buf = Vec::new();

        if let Err(e) = d.read_to_end(&mut buf) {
            panic!("error reading: {}", e);
        }

        assert!(&buf[..] == output);
    }

    #[test]
    fn decode() {
        let reference = include_bytes!("data/test.txt");
        test_decode(include_bytes!("data/test.z.0"), reference);
        test_decode(include_bytes!("data/test.z.1"), reference);
        test_decode(include_bytes!("data/test.z.2"), reference);
        test_decode(include_bytes!("data/test.z.3"), reference);
        test_decode(include_bytes!("data/test.z.4"), reference);
        test_decode(include_bytes!("data/test.z.5"), reference);
        test_decode(include_bytes!("data/test.z.6"), reference);
        test_decode(include_bytes!("data/test.z.7"), reference);
        test_decode(include_bytes!("data/test.z.8"), reference);
        test_decode(include_bytes!("data/test.z.9"), reference);
    }

    #[test]
    fn large() {
        let reference = include_bytes!("data/test.large");
        test_decode(include_bytes!("data/test.large.z.5"), reference);
    }

    #[test]
    fn one_byte_at_a_time() {
        let input = include_bytes!("data/test.z.1");
        let mut d = Decoder::new(BufReader::new(&input[..]));
        assert!(!d.eof());
        let mut out = Vec::new();
        loop {
            match d.read_u8() {
                Ok(b) => out.push(b),
                Err(..) => break
            }
        }
        assert!(d.eof());
        assert!(&out[..] == &include_bytes!("data/test.txt")[..]);
    }

    #[test]
    fn random_byte_lengths() {
        let input = include_bytes!("data/test.z.1");
        let mut d = Decoder::new(BufReader::new(&input[..]));
        let mut out = Vec::new();
        let mut buf = [0u8; 40];
        loop {
            match d.read(&mut buf[..(1 + random::<usize>() % 40)]) {
                Ok(0) | Err(..) => break,
                Ok(n) => {
                    out.extend(buf[..n].iter().map(|b| *b));
                }
            }
        }
        assert!(&out[..] == &include_bytes!("data/test.txt")[..]);
    }

    //fn roundtrip(bytes: &[u8]) {
    //    let mut e = Encoder::new(MemWriter::new());
    //    e.write(bytes);
    //    let encoded = e.finish().unwrap();
    //
    //    let mut d = Decoder::new(BufReader::new(encoded));
    //    let decoded = d.read_to_end();
    //    assert_eq!(&decoded[..], bytes);
    //}
    //
    //#[test]
    //fn some_roundtrips() {
    //    roundtrip(bytes!("test"));
    //    roundtrip(bytes!(""));
    //    roundtrip(include_bytes!("data/test.txt"));
    //}

    #[cfg(feature="unstable")]
    #[bench]
    fn decompress_speed(bh: &mut test::Bencher) {
        let input = include_bytes!("data/test.z.9");
        let mut d = Decoder::new(BufReader::new(&input[..]));
        let mut output = [0u8; 65536];
        let mut output_size = 0;
        bh.iter(|| {
            d.inner.r = BufReader::new(input);
            d.reset();
            output_size = d.read(&mut output[..]).unwrap();
        });
        bh.bytes = output_size as u64;
    }
}
