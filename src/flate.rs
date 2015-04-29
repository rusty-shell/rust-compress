//! DEFLATE Compression and Decompression. Requires `flate` feature, enabled by default
//!
//! This module contains an implementation of the DEFLATE compression scheme.
//! This format is often used as the underpinning of other compression formats.
//!
//! # Example
//!
//! ```rust
//! use compress::flate;
//! use std::fs::File;
//!
//! let stream = File::open(&Path::new("path/to/file.flate"));
//! let decompressed = flate::Decoder::new(stream).read_to_end();
//! ```
//!
//! # Related links
//!
//! * http://tools.ietf.org/html/rfc1951 - RFC that this implementation is based
//!   on
//! * http://www.gzip.org/zlib/rfc-deflate.html - simplified version of RFC 1951
//!   used as a reference
//! * http://svn.ghostscript.com/ghostscript/trunk/gs/zlib/contrib/puff/puff.c -
//!   Much of this code is based on the puff.c implementation found here

use std::cmp;
use std::io::{self, Read};
use std::slice;
use std::vec::Vec;

use super::byteorder::{LittleEndian, ReadBytesExt};
use super::ReadExact;

const MAXBITS: usize = 15;
const MAXLCODES: u16 = 286;
const MAXDCODES: u16 = 30;
const MAXCODES: u16 = MAXLCODES + MAXDCODES;
const HISTORY: usize = 32 * 1024;

enum Error {
    HuffmanTreeTooLarge,
    InvalidBlockCode,
    InvalidHuffmanHeaderSymbol,
    InvalidHuffmanTree,
    InvalidHuffmanTreeHeader,
    InvalidHuffmanCode,
    InvalidStaticSize,
    NotEnoughBits,
}

fn error<T>(e: Error) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        match e {
            Error::HuffmanTreeTooLarge => "huffman tree too large",
            Error::InvalidBlockCode => "invalid block code",
            Error::InvalidHuffmanHeaderSymbol => "invalid huffman header symbol",
            Error::InvalidHuffmanTree => "invalid huffman tree",
            Error::InvalidHuffmanTreeHeader => "invalid huffman tree header",
            Error::InvalidHuffmanCode => "invalid huffman code",
            Error::InvalidStaticSize => "invalid static size",
            Error::NotEnoughBits => "not enough bits",
        }
    ))
}

struct HuffmanTree {
    /// An array which counts the number of codes which can be found at the
    /// index's bit length, or count[n] is the number of n-bit codes
    pub count: [u16; MAXBITS + 1],

    /// Symbols in this huffman tree in sorted order. This preserves the
    /// original huffman codes
    pub symbol: [u16; MAXCODES as usize],
}

impl HuffmanTree {
    /// Constructs a new huffman tree for decoding. If the given array has
    /// length N, then the huffman tree can be used to decode N symbols. Each
    /// entry in the array corresponds to the length of the nth symbol.
    fn construct(lens: &[u16]) -> io::Result<HuffmanTree> {
        let mut tree = HuffmanTree {
            count: [0; MAXBITS + 1],
            symbol: [0; MAXCODES as usize],
        };
        // Collect the lengths of all symbols
        for len in lens.iter() {
            tree.count[*len as usize] += 1;
        }
        // If there weren't actually any codes, then we're done
        if tree.count[0] as usize == lens.len() { return Ok(tree) }

        // Make sure that this tree is sane. Each bit gives us 2x more codes to
        // work with, but if the counts add up to greater than the available
        // amount, then this is an invalid table.
        let mut left = 1;
        for i in 1..(MAXBITS + 1) {
            left *= 2;
            left -= tree.count[i] as isize;
            if left < 0 { return error(Error::InvalidHuffmanTree) }
        }

        // Generate the offset of each length into the 'symbol' array
        let mut offs = [0; MAXBITS + 1];
        for i in 1..MAXBITS {
            offs[i + 1] = offs[i] + tree.count[i];
        }

        // Insert all symbols into the table, in sorted order using the `offs`
        // array generated above.
        for (sym, &len) in lens.iter().enumerate() {
            if len != 0 {
                tree.symbol[offs[len as usize] as usize] = sym as u16;
                offs[len as usize] += 1;
            }
        }
        return Ok(tree);
    }

    /// Decodes a codepoint from the buffer.
    ///
    /// This operates by reading bits as long as the code isn't found within the
    /// valid range of the codes itself. Remember the codepoints are all encoded
    /// by a sequence of lengths. The codepoint being decoded needs to figure
    /// out what lengths it's between, and then within that range we can index
    /// into the whole symbol array to pluck out the right symbol.
    fn decode<R: Read>(&self, s: &mut Decoder<R>) -> io::Result<u16> {
        // this could be a lot faster.
        let mut code = 0;
        let mut first = 0;
        let mut index = 0;
        for len in 1..(MAXBITS + 1) {
            code |= try!(s.bits(1));
            let count = self.count[len];
            if code < first + count {
                return Ok(self.symbol[(index + (code - first)) as usize])
            }
            index += count;
            first += count;
            first <<= 1;
            code <<= 1;
        }
        return error(Error::NotEnoughBits);
    }
}

#[cfg(genflate)]
fn main() {
    static FIXLCODES: usize = 388;
    let mut arr = [0; FIXLCODES];
    for i in 0..144 { arr[i] = 8; }
    for i in 144..256 { arr[i] = 9; }
    for i in 256..280 { arr[i] = 7; }
    for i in 280..288 { arr[i] = 8; }
    println!("{:?}", HuffmanTree::construct(arr[..FIXLCODES]));
    for i in 0..MAXDCODES { arr[i] = 5; }
    println!("{:?}", HuffmanTree::construct(arr[..MAXDCODES]));
}

/// The structure that is used to decode an LZ4 data stream. This wraps an
/// internal reader which is used as the source of all data.
pub struct Decoder<R> {
    /// Wrapped reader which is exposed to allow getting it back.
    pub r: R,

    output: Vec<u8>,
    outpos: usize,

    block: Vec<u8>,
    pos: usize,

    bitbuf: usize,
    bitcnt: usize,
    eof: bool,
}

impl<R: Read> Decoder<R> {
    /// Creates a new flate decoder which will read data from the specified
    /// source
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            r: r,
            output: Vec::with_capacity(HISTORY),
            outpos: 0,
            block: Vec::new(),
            pos: 0,
            bitbuf: 0,
            bitcnt: 0,
            eof: false,
        }
    }

    fn block(&mut self) -> io::Result<()> {
        self.pos = 0;
        self.block = Vec::with_capacity(4096);
        if try!(self.bits(1)) == 1 { self.eof = true; }
        match try!(self.bits(2)) {
            0 => self.statik(),
            1 => self.fixed(),
            2 => self.dynamic(),
            3 => error(Error::InvalidBlockCode),
            _ => unreachable!(),
        }
    }

    fn update_output(&mut self, mut from: usize) {
        let to = self.block.len();
        if to - from > HISTORY {
            from = to - HISTORY;
        }
        let amt = to - from;
        let remaining = HISTORY - self.outpos;
        let n = cmp::min(amt, remaining);
        if self.output.len() < HISTORY {
            self.output.push_all(&self.block[from..(from + n)]);
        } else {
            assert_eq!(self.output.len(), HISTORY);
            slice::bytes::copy_memory(&self.block[from..(from + n)],
                                      &mut self.output[self.outpos..]);
        }
        self.outpos += n;
        if n < amt {
            slice::bytes::copy_memory(&self.block[from..(from + n)],
                                      &mut self.output[..]);
            self.outpos = amt - n;
        }
    }

    fn statik(&mut self) -> io::Result<()> {
        let len = try!(self.r.read_u16::<LittleEndian>());
        let nlen = try!(self.r.read_u16::<LittleEndian>());
        if !nlen != len { return error(Error::InvalidStaticSize) }
        try!(self.r.push_exactly(len as u64, &mut self.block));
        self.update_output(0);
        self.bitcnt = 0;
        self.bitbuf = 0;
        Ok(())
    }

    // Bytes in the stream are LSB first, so the bitbuf is appended to from the
    // left and consumed from the right.
    fn bits(&mut self, cnt: usize) -> io::Result<u16> {
        while self.bitcnt < cnt {
            let byte = try!(self.r.read_u8());
            self.bitbuf |= (byte as usize) << self.bitcnt;
            self.bitcnt += 8;
        }
        let ret = self.bitbuf & ((1 << cnt) - 1);
        self.bitbuf >>= cnt;
        self.bitcnt -= cnt;
        return Ok(ret as u16);
    }

    fn codes(&mut self, lens: &HuffmanTree,
             dist: &HuffmanTree) -> io::Result<()> {
        // extra base length for codes 257-285
        static EXTRALENS: [u16; 29] = [
            3, 4, 5, 6, 7, 8, 9, 10, 11, 13, 15, 17, 19, 23, 27, 31, 35, 43, 51,
            59, 67, 83, 99, 115, 131, 163, 195, 227, 258
        ];
        // extra bits to read for codes 257-285
        static EXTRABITS: [u16; 29] = [
            0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 2, 2, 3, 3, 3, 3, 4, 4, 4,
            4, 5, 5, 5, 5, 0,
        ];
        // base offset for distance codes.
        static EXTRADIST: [u16; 30] = [
            1, 2, 3, 4, 5, 7, 9, 13, 17, 25, 33, 49, 65, 97, 129, 193, 257, 385,
            513, 769, 1025, 1537, 2049, 3073, 4097, 6145, 8193, 12289, 16385,
            24577,
        ];
        // number of bits to read for distance codes (to add to the offset)
        static EXTRADBITS: [u16; 30] = [
            0, 0, 0, 0, 1, 1, 2, 2, 3, 3, 4, 4, 5, 5, 6, 6, 7, 7, 8, 8, 9, 9,
            10, 10, 11, 11, 12, 12, 13, 13,
        ];
        let mut last_updated = 0;
        loop {
            let sym = try!(lens.decode(self));
            match sym {
                n if n < 256 => { self.block.push(sym as u8); }
                256 => break,
                n if n < 290 => {
                    // figure out len/dist that we're working with
                    let n = n - 257;
                    if n as usize > EXTRALENS.len() {
                        return error(Error::InvalidHuffmanCode)
                    }
                    let len = EXTRALENS[n as usize] +
                              try!(self.bits(EXTRABITS[n as usize] as usize));

                    let len = len as usize;

                    let dist = try!(dist.decode(self)) as usize;
                    let dist = EXTRADIST[dist] +
                               try!(self.bits(EXTRADBITS[dist] as usize));
                    let dist = dist as usize;

                    // update the output buffer with any data we haven't pushed
                    // into it yet
                    if last_updated != self.block.len() {
                        self.update_output(last_updated);
                        last_updated = self.block.len();
                    }

                    if dist > self.output.len() {
                        return error(Error::InvalidHuffmanCode)
                    }

                    // Perform the copy
                    self.block.reserve(dist);
                    let mut finger = if self.outpos >= dist {
                        self.outpos - dist
                    } else {
                        HISTORY - (dist - self.outpos)
                    };
                    let min = cmp::min(dist, len);
                    let start = self.block.len();
                    for _ in 0..min {
                        self.block.push(self.output[finger]);
                        finger = (finger + 1) % HISTORY;
                    }
                    for i in min..len {
                        let b = self.block[start + i - min];
                        self.block.push(b);
                    }
                }
                _ => return error(Error::InvalidHuffmanCode)
            }
        }
        self.update_output(last_updated);
        Ok(())
    }

    fn fixed(&mut self) -> io::Result<()> {
        // Generated by the main function above
        static LEN: HuffmanTree = HuffmanTree {
            count: [100, 0, 0, 0, 0, 0, 0, 24, 152, 112, 0, 0, 0, 0, 0, 0],
            symbol: [
                256, 257, 258, 259, 260, 261, 262, 263, 264, 265, 266, 267, 268,
                269, 270, 271, 272, 273, 274, 275, 276, 277, 278, 279, 0, 1, 2,
                3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20,
                21, 22, 23, 24, 25, 26, 27, 28, 29, 30, 31, 32, 33, 34, 35, 36,
                37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50, 51, 52,
                53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 63, 64, 65, 66, 67, 68,
                69, 70, 71, 72, 73, 74, 75, 76, 77, 78, 79, 80, 81, 82, 83, 84,
                85, 86, 87, 88, 89, 90, 91, 92, 93, 94, 95, 96, 97, 98, 99, 100,
                101, 102, 103, 104, 105, 106, 107, 108, 109, 110, 111, 112, 113,
                114, 115, 116, 117, 118, 119, 120, 121, 122, 123, 124, 125, 126,
                127, 128, 129, 130, 131, 132, 133, 134, 135, 136, 137, 138, 139,
                140, 141, 142, 143, 280, 281, 282, 283, 284, 285, 286, 287, 144,
                145, 146, 147, 148, 149, 150, 151, 152, 153, 154, 155, 156, 157,
                158, 159, 160, 161, 162, 163, 164, 165, 166, 167, 168, 169, 170,
                171, 172, 173, 174, 175, 176, 177, 178, 179, 180, 181, 182, 183,
                184, 185, 186, 187, 188, 189, 190, 191, 192, 193, 194, 195, 196,
                197, 198, 199, 200, 201, 202, 203, 204, 205, 206, 207, 208, 209,
                210, 211, 212, 213, 214, 215, 216, 217, 218, 219, 220, 221, 222,
                223, 224, 225, 226, 227, 228, 229, 230, 231, 232, 233, 234, 235,
                236, 237, 238, 239, 240, 241, 242, 243, 244, 245, 246, 247, 248,
                249, 250, 251, 252, 253, 254, 255, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0
            ]
        };
        static DIST: HuffmanTree = HuffmanTree {
            count: [0, 0, 0, 0, 0, 30, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0],
            symbol: [
                0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17,
                18, 19, 20, 21, 22, 23, 24, 25, 26, 27, 28, 29, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0, 0, 0, 0, 0
            ]
        };

        self.codes(&LEN, &DIST)
    }

    fn dynamic(&mut self) -> io::Result<()> {
        let hlit = try!(self.bits(5)) + 257; // number of length codes
        let hdist = try!(self.bits(5)) + 1;  // number of distance codes
        let hclen = try!(self.bits(4)) + 4;  // number of code length codes
        if hlit > MAXLCODES || hdist > MAXDCODES {
            return error(Error::HuffmanTreeTooLarge);
        }

        // Read off the code length codes, and then build the huffman tree which
        // is then used to decode the actual huffman tree for the rest of the
        // data.
        static ORDER: [usize; 19] = [
            16, 17, 18, 0, 8, 7, 9, 6, 10, 5, 11, 4, 12, 3, 13, 2, 14, 1, 15,
        ];
        let mut lengths = [0; 19];
        for i in 0..(hclen as usize) {
            lengths[ORDER[i]] = try!(self.bits(3));
        }
        let tree = try!(HuffmanTree::construct(&lengths));

        // Decode all of the length and distance codes in one go, we'll
        // partition them into two huffman trees later
        let mut lengths = [0; MAXCODES as usize];
        let mut i = 0;
        while i < hlit + hdist {
            let symbol = try!(tree.decode(self));
            match symbol {
                n if n < 16 => {
                    lengths[i as usize] = symbol;
                    i += 1;
                }
                16 if i == 0 => return error(Error::InvalidHuffmanHeaderSymbol),
                16 => {
                    let prev = lengths[i as usize - 1];
                    for _ in 0..(try!(self.bits(2)) + 3) {
                        lengths[i as usize] = prev;
                        i += 1;
                    }
                }
                // all codes start out as 0, so these just skip
                17 => { i += try!(self.bits(3)) + 3; }
                18 => { i += try!(self.bits(7)) + 11; }
                _ => return error(Error::InvalidHuffmanHeaderSymbol),
            }
        }
        if i > hlit + hdist { return error(Error::InvalidHuffmanTreeHeader) }

        // Use the decoded codes to construct yet another huffman tree
        let arr = &lengths[..(hlit as usize)];
        let lencode = try!(HuffmanTree::construct(arr));
        let arr = &lengths[(hlit as usize)..((hlit + hdist) as usize)];
        let distcode = try!(HuffmanTree::construct(arr));
        self.codes(&lencode, &distcode)
    }

    /// Returns whether this deflate stream has reached the EOF marker
    pub fn eof(&self) -> bool {
        self.eof && self.pos == self.block.len()
    }

    /// Resets this flate decoder. Note that this could corrupt an in-progress
    /// decoding of a stream.
    pub fn reset(&mut self) {
        self.bitbuf = 0;
        self.bitcnt = 0;
        self.eof = false;
        self.block = Vec::new();
        self.pos = 0;
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.pos == self.block.len() {
            if self.eof { return Ok(0) }
            try!(self.block());
        }
        let n = cmp::min(buf.len(), self.block.len() - self.pos);
        slice::bytes::copy_memory(&buf[..n],
                                  &mut self.block[self.pos..(self.pos + n)]);
        self.pos += n;
        Ok(n)
    }
}

#[cfg(test)]
#[allow(warnings)]
mod test {
    use std::io::{BufReader, BufWriter, Read, Write};
    use super::super::rand::{Rand, random};
    use super::super::byteorder::{LittleEndian, BigEndian, WriteBytesExt, ReadBytesExt};
    use std::str;
    use super::{Decoder};
    use test;

    // The input data for these tests were all generated from the zpipe.c
    // program found at http://www.zlib.net/zpipe.c and the zlib format has an
    // extra 2 bytes of header with an 4-byte checksum at the end.
    fn fixup<'a>(s: &'a [u8]) -> &'a [u8] {
        &s[2..(s.len() - 4)]
    }

    fn test_decode(input: &[u8], output: &[u8]) {
        let mut d = Decoder::new(BufReader::new(fixup(input)));
        let mut buf = Vec::new();
        d.read_to_end(&mut buf).unwrap();
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
        let mut d = Decoder::new(BufReader::new(fixup(input)));
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
        let mut d = Decoder::new(BufReader::new(fixup(input)));
        let mut out = Vec::new();
        let mut buf = [0u8; 40];
        loop {
            match d.read(&mut buf[..(1 + random::<usize>() % 40)]) {
                Ok(n) => {
                    out.push_all(&buf[..n]);
                }
                Err(..) => break
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

    #[bench]
    fn decompress_speed(bh: &mut test::Bencher) {
        let input = include_bytes!("data/test.z.9");
        let mut d = Decoder::new(BufReader::new(fixup(input)));
        let mut output = [0u8; 65536];
        let mut output_size = 0;
        bh.iter(|| {
            d.r = BufReader::new(fixup(input));
            d.reset();
            output_size = d.read(&mut output).unwrap();
        });
        bh.bytes = output_size as u64;
    }
}
