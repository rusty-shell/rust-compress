/*!

LZ4 Decompression and Compression. Requires `lz4` feature, enabled by default

This module contains an implementation in Rust of decompression and compression
of LZ4-encoded streams. These are exposed as a standard `Reader` and `Writer`
interfaces wrapping an underlying stream.

# Example

```rust
use compress::lz4;
use std::io::File;

let stream = File::open(&Path::new("path/to/file.lz4"));
let decompressed = lz4::Decoder::new(stream).read_to_end();
```

# Credit

This implementation is largely based on Branimir Karadžić's implementation which
can be found at https://github.com/bkaradzic/go-lz4.

*/

use std::cmp;
use std::io;
use std::iter::repeat;
use std::slice;
use std::vec::Vec;

const MAGIC: u32 = 0x184d2204;

const ML_BITS: u32 = 4;
const ML_MASK: u32 = (1 << ML_BITS as uint) - 1;
const RUN_BITS: u32 = 8 - ML_BITS;
const RUN_MASK: u32 = (1 << RUN_BITS as uint) - 1;

const MIN_MATCH: u32 = 4;
const HASH_LOG: u32 = 17;
const HASH_TABLE_SIZE: u32 = 1 << (HASH_LOG as uint);
const HASH_SHIFT: u32 = (MIN_MATCH * 8) - HASH_LOG;
const INCOMPRESSIBLE: u32 = 128;
const UNINITHASH: u32 = 0x88888888;
const MAX_INPUT_SIZE: u32 = 0x7e000000;

struct BlockDecoder<'a> {
    input: &'a [u8],
    output: &'a mut Vec<u8>,
    cur: uint,

    start: uint,
    end: uint,
}

impl<'a> BlockDecoder<'a> {
    /// Decodes this block of data from 'input' to 'output', returning the
    /// number of valid bytes in the output.
    fn decode(&mut self) -> uint {
        while self.cur < self.input.len() {
            let code = self.bump();
            debug!("block with code: {:x}", code);
            // Extract a chunk of data from the input to the output.
            {
                let len = self.length(code >> 4);
                debug!("consume len {}", len);
                let end = self.end;
                self.grow_output(end + len);
                slice::bytes::copy_memory(self.output.slice_from_mut(end),
                                          self.input.slice(self.cur, self.cur + len));
                self.end += len;
                self.cur += len;
            }
            if self.cur == self.input.len() { break }

            // Read off the next i16 offset
            {
                let back = (self.bump() as uint) | ((self.bump() as uint) << 8);
                debug!("found back {}", back);
                self.start = self.end - back;
            }

            // Slosh around some bytes now
            {
                let mut len = self.length(code & 0xf);
                let literal = self.end - self.start;
                if literal < 4 {
                    static DECR: [uint; 4] = [0, 3, 2, 3];
                    self.cp(4, DECR[literal]);
                } else {
                    len += 4;
                }
                self.cp(len, 0);
            }
        }
        return self.end;
    }

    fn length(&mut self, code: u8) -> uint {
        let mut ret = code as uint;
        if code == 0xf {
            loop {
                let tmp = self.bump();
                ret += tmp as uint;
                if tmp != 0xff { break }
            }
        }
        return ret;
    }

    fn bump(&mut self) -> u8 {
        let ret = self.input[self.cur];
        self.cur += 1;
        return ret;
    }

    #[inline]
    fn cp(&mut self, len: uint, decr: uint) {
        let end = self.end;
        self.grow_output(end + len);
        for i in range(0, len) {
            self.output[end + i] = (*self.output)[self.start + i];
        }

        self.end += len;
        self.start += len - decr;
    }

    // Extends the output vector to a target number of bytes (in total), but
    // does not actually initialize the new data. The length of the vector is
    // updated, but the bytes will all have undefined values. It is assumed that
    // the next operation is to pave over these bytes (so the initialization is
    // unnecessary).
    #[inline]
    fn grow_output(&mut self, target: uint) {
        if self.output.capacity() < target {
            debug!("growing {} to {}", self.output.capacity(), target);
            self.output.reserve(target);
        }
        unsafe {
            self.output.set_len(target);
        }
    }
}

struct BlockEncoder<'a> {
    input: &'a [u8],
    output: &'a mut Vec<u8>,
    hash_table: Vec<u32>,
    pos: u32,
    anchor: u32,
    dest_pos: u32
}

/// Returns maximum possible size of compressed output
/// given source size
pub fn compression_bound(size: u32) -> Option<u32> {
    if size > MAX_INPUT_SIZE {
        None
    } else {
        Some(size + (size / 255) + 16 + 4)
    }
}

impl<'a> BlockEncoder<'a> {
    #[inline(always)]
    fn seq_at(&self, pos: u32) -> u32 {
        (self.input[pos as uint + 3] as u32) << 24
            | (self.input[pos as uint + 2] as u32) << 16
            | (self.input[pos as uint + 1] as u32) << 8
            | (self.input[pos as uint] as u32)
    }

    fn write_literals(&mut self, len: u32, ml_len: u32, pos: u32) {
        let mut ln = len;

        let code = if ln > RUN_MASK - 1 { RUN_MASK as u8 } else { ln as u8 };

        if ml_len > ML_MASK - 1 {
            self.output[self.dest_pos as uint] = (code << ML_BITS as uint) + ML_MASK as u8;
        } else {
            self.output[self.dest_pos as uint] = (code << ML_BITS as uint) + ml_len as u8;
        }

        self.dest_pos += 1;

        if code == RUN_MASK as u8 {
            ln -= RUN_MASK;
            while ln > 254 {
                self.output[self.dest_pos as uint] = 255;
                self.dest_pos += 1;
                ln -= 255;
            }

            self.output[self.dest_pos as uint] = ln as u8;
            self.dest_pos += 1;
        }

        // FIXME: find out why slicing syntax fails tests
        //self.output[self.dest_pos as uint .. (self.dest_pos + len) as uint] = self.input[pos as uint.. (pos + len) as uint];
        for i in range(0, len as uint) {
            self.output[self.dest_pos as uint + i] = self.input[pos as uint + i];
        }

        self.dest_pos += len;
    }

    fn encode(&mut self) -> u32 {
        let input_len = self.input.len() as u32;

        match compression_bound(input_len) {
            None => 0,
            Some(out_size) => {
                let additional = out_size as uint - self.output.capacity();
                self.output.reserve(additional);
                unsafe {self.output.set_len(out_size as uint); }

                let mut step = 1u32;
                let mut limit = INCOMPRESSIBLE;

                loop {
                    if self.pos + 12 > input_len {
                        let tmp = self.anchor;
                        self.write_literals(self.input.len() as u32 - tmp, 0, tmp);
                        unsafe { self.output.set_len(self.dest_pos as uint) };
                        return self.dest_pos;
                    }

                    let seq = self.seq_at(self.pos);

                    let hash = (seq * 2654435761) >> (HASH_SHIFT as uint);
                    let mut r = self.hash_table[hash as uint] + UNINITHASH;
                    self.hash_table[hash as uint] = self.pos - UNINITHASH;

                    if ((self.pos - r) >> 16) != 0 || seq != self.seq_at(r) {
                        if self.pos - self.anchor > limit {
                            limit = limit << 1;
                            step += 1 + (step >> 2);
                        }
                        self.pos += step;
                        continue;
                    }

                    if step > 1 {
                        self.hash_table[hash as uint] = r - UNINITHASH;
                        self.pos -= step - 1;
                        step = 1;
                        continue;
                    }

                    limit = INCOMPRESSIBLE;

                    let ln = self.pos - self.anchor;
                    let back = self.pos - r;
                    let anchor = self.anchor;

                    self.pos += MIN_MATCH;
                    r += MIN_MATCH;
                    self.anchor = self.pos;

                    while (self.pos < input_len - 5) && self.input[self.pos as uint] == self.input[r as uint] {
                        self.pos += 1;
                        r += 1
                    }

                    let mut ml_len = self.pos - self.anchor;

                    self.write_literals(ln, ml_len, anchor);
                    self.output[self.dest_pos as uint] = back as u8;
                    self.output[self.dest_pos as uint + 1] = (back >> 8) as u8;
                    self.dest_pos += 2;

                    if ml_len > ML_MASK - 1 {
                        ml_len -= ML_MASK;
                        while ml_len > 254 {
                            ml_len -= 255;

                            self.output[self.dest_pos as uint] = 255;
                            self.dest_pos += 1;
                        }

                        self.output[self.dest_pos as uint] = ml_len as u8;
                        self.dest_pos += 1;
                    }

                    self.anchor = self.pos;
                }
            }
        }
    }
}

/// This structure is used to decode a stream of LZ4 blocks. This wraps an
/// internal reader which is read from when this decoder's read method is
/// called.
pub struct Decoder<R> {
    /// The internally wrapped reader. This is exposed so it may be moved out
    /// of. Note that if data is read from the reader while decoding is in
    /// progress the output stream will get corrupted.
    pub r: R,

    temp: Vec<u8>,
    output: Vec<u8>,

    start: uint,
    end: uint,
    eof: bool,

    header: bool,
    blk_checksum: bool,
    stream_checksum: bool,
    max_block_size: uint,
}

impl<R: Reader> Decoder<R> {
    /// Creates a new decoder which will read data from the given stream. The
    /// inner stream can be re-acquired by moving out of the `r` field of this
    /// structure.
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            r: r,
            temp: Vec::new(),
            output: Vec::new(),
            header: false,
            blk_checksum: false,
            stream_checksum: false,
            start: 0,
            end: 0,
            eof: false,
            max_block_size: 0,
        }
    }

    /// Resets this decoder back to its initial state. Note that the underlying
    /// stream is not seeked on or has any alterations performed on it.
    pub fn reset(&mut self) {
        self.header = false;
        self.eof = false;
        self.start = 0;
        self.end = 0;
    }

    fn read_header(&mut self) -> io::IoResult<()> {
        // Make sure the magic number is what's expected.
        if try!(self.r.read_le_u32()) != MAGIC {
            return Err(io::standard_error(io::InvalidInput))
        }

        let mut bits = [0; 3];
        try!(self.r.read(bits.slice_to_mut(2)));
        let flg = bits[0];
        let bd = bits[1];

        // bits 7/6, the version number. Right now this must be 1
        if (flg >> 6) != 0b01 {
            return Err(io::standard_error(io::InvalidInput))
        }
        // bit 5 is the "block independence", don't care about this yet
        // bit 4 is whether blocks have checksums or not
        self.blk_checksum = (flg & 0x10) != 0;
        // bit 3 is whether there is a following stream size
        let stream_size = (flg & 0x08) != 0;
        // bit 2 is whether there is a stream checksum
        self.stream_checksum = (flg & 0x04) != 0;
        // bit 1 is reserved
        // bit 0 is whether there is a preset dictionary
        let preset_dictionary = (flg & 0x01) != 0;

        static MAX_SIZES: [uint; 8] =
            [0, 0, 0, 0, // all N/A
             64 << 10,   // 64KB
             256 << 10,  // 256 KB
             1 << 20,    // 1MB
             4 << 20];   // 4MB

        // bit 7 is reserved
        // bits 6-4 are the maximum block size
        let max_block_size = MAX_SIZES[(bd >> 4) as uint & 0x7];
        // bits 3-0 are reserved

        // read off other portions of the stream
        let size = if stream_size {
            Some(try!(self.r.read_le_u64()))
        } else {
            None
        };
        assert!(!preset_dictionary, "preset dictionaries not supported yet");

        debug!("blk: {}", self.blk_checksum);
        debug!("stream: {}", self.stream_checksum);
        debug!("max size: {}", max_block_size);
        debug!("stream size: {}", size);

        self.max_block_size = max_block_size;

        // XXX: implement checksums
        let cksum = try!(self.r.read_byte());
        debug!("ignoring header checksum: {}", cksum);
        return Ok(());
    }

    fn decode_block(&mut self) -> io::IoResult<bool> {
        match try!(self.r.read_le_u32()) {
            // final block, we're done here
            0 => return Ok(false),

            // raw block to read
            n if n & 0x80000000 != 0 => {
                let amt = (n & 0x7fffffff) as uint;
                self.output.truncate(0);
                self.output.reserve(amt);
                try!(self.r.push_at_least(amt, amt, &mut self.output));
                self.start = 0;
                self.end = amt;
            }

            // actual block to decompress
            n => {
                let n = n as uint;
                self.temp.truncate(0);
                self.temp.reserve(n);
                try!(self.r.push_at_least(n, n, &mut self.temp));

                let target = cmp::min(self.max_block_size, 4 * n / 3);
                self.output.truncate(0);
                self.output.reserve(target);
                let mut decoder = BlockDecoder {
                    input: self.temp.slice_to(n),
                    output: &mut self.output,
                    cur: 0,
                    start: 0,
                    end: 0,
                };
                self.start = 0;
                self.end = decoder.decode();
            }
        }

        if self.blk_checksum {
            let cksum = try!(self.r.read_le_u32());
            debug!("ignoring block checksum {}", cksum);
        }
        return Ok(true);
    }

    /// Tests whether the end of this LZ4 stream has been reached
    pub fn eof(&mut self) -> bool { return self.eof }
}

impl<R: Reader> Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        if self.eof { return Err(io::standard_error(io::EndOfFile)) }
        if !self.header {
            try!(self.read_header());
            self.header = true;
        }
        let mut amt = dst.len();
        let len = amt;

        while amt > 0 {
            if self.start == self.end {
                let keep_going = try!(self.decode_block());
                if !keep_going {
                    self.eof = true;
                    break;
                }
            }
            let n = cmp::min(amt, self.end - self.start);
            slice::bytes::copy_memory(dst.slice_from_mut(len - amt),
                                    self.output.slice(self.start, self.start + n));
            self.start += n;
            amt -= n;
        }

        return Ok(len - amt);
    }
}

/// This structure is used to compress a stream of bytes using the LZ4
/// compression algorithm. This is a wrapper around an internal writer which
/// bytes will be written to.
pub struct Encoder<W> {
    w: W,
    buf: Vec<u8>,
    tmp: Vec<u8>,
    wrote_header: bool,
    limit: uint,
}

impl<W: Writer> Encoder<W> {
    /// Creates a new encoder which will have its output written to the given
    /// output stream. The output stream can be re-acquired by calling
    /// `finish()`
    ///
    /// NOTE: compression isn't actually implemented just yet, this is just a
    /// skeleton of a future implementation.
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            w: w,
            wrote_header: false,
            buf: Vec::with_capacity(1024),
            tmp: Vec::new(),
            limit: 256 * 1024,
        }
    }

    fn encode_block(&mut self) -> io::IoResult<()> {
        self.tmp.truncate(0);
        if self.compress() {
            try!(self.w.write_le_u32(self.tmp.len() as u32));
            try!(self.w.write(self.tmp.as_slice()))
        } else {
            try!(self.w.write_le_u32((self.buf.len() as u32) | 0x80000000));
            try!(self.w.write(self.buf.as_slice()))
        }
        self.buf.truncate(0);
        Ok(())
    }

    fn compress(&mut self) -> bool {
        false
    }

    /// This function is used to flag that this session of compression is done
    /// with. The stream is finished up (final bytes are written), and then the
    /// wrapped writer is returned.
    pub fn finish(mut self) -> (W, io::IoResult<()>) {
        let result = self.flush();
        let result = result.and(self.w.write_le_u32(0));
        // XXX: this checksum is wrong
        let result = result.and(self.w.write_le_u32(0));
        (self.w, result)
    }
}

impl<W: Writer> Writer for Encoder<W> {
    fn write(&mut self, mut buf: &[u8]) -> io::IoResult<()> {
        if !self.wrote_header {
            try!(self.w.write_le_u32(MAGIC));
            // version 01, turn on block independence, but turn off
            // everything else (we have no checksums right now).
            try!(self.w.write_u8(0b01_100000));
            // Maximum block size is 256KB
            try!(self.w.write_u8(0b0_101_0000));
            // XXX: this checksum is just plain wrong.
            try!(self.w.write_u8(0));
            self.wrote_header = true;
        }

        while buf.len() > 0 {
            let amt = cmp::min(self.limit - self.buf.len(), buf.len());
            self.buf.push_all(buf.slice_to(amt));

            if self.buf.len() == self.limit {
                try!(self.encode_block());
            }
            buf = buf.slice_from(amt);
        }
        Ok(())
    }

    fn flush(&mut self) -> io::IoResult<()> {
        if self.buf.len() > 0 {
            try!(self.encode_block());
        }
        self.w.flush()
    }
}


/// Decodes pure LZ4 block into output. Returns count of bytes
/// processed.
pub fn decode_block(input: &[u8], output: &mut Vec<u8>) -> uint {
    let mut b = BlockDecoder {
        input: input,
        output: output,
        cur: 0,
        start: 0,
        end: 0
    };
    b.decode()
}


/// Encodes input into pure LZ4 block. Return count of bytes
/// processed.
pub fn encode_block(input: &[u8], output: &mut Vec<u8>) -> uint {
    let mut encoder = BlockEncoder {
        input: input,
        output: output,
        hash_table: repeat(0).take(HASH_TABLE_SIZE as uint).collect(),
        pos: 0,
        anchor: 0,
        dest_pos: 0
    };

    encoder.encode() as uint
}

#[cfg(test)]
mod test {
    use std::io::{BufReader, MemWriter};
    use std::rand;
    use super::{Decoder, Encoder};
    use test;

    fn test_decode(input: &[u8], output: &[u8]) {
        let mut d = Decoder::new(BufReader::new(input));

        let got = d.read_to_end().unwrap();
        assert!(got.as_slice() == output);
    }

    #[test]
    fn decode() {
        let reference = include_bytes!("data/test.txt");
        test_decode(include_bytes!("data/test.lz4.1"), reference);
        test_decode(include_bytes!("data/test.lz4.2"), reference);
        test_decode(include_bytes!("data/test.lz4.3"), reference);
        test_decode(include_bytes!("data/test.lz4.4"), reference);
        test_decode(include_bytes!("data/test.lz4.5"), reference);
        test_decode(include_bytes!("data/test.lz4.6"), reference);
        test_decode(include_bytes!("data/test.lz4.7"), reference);
        test_decode(include_bytes!("data/test.lz4.8"), reference);
        test_decode(include_bytes!("data/test.lz4.9"), reference);
    }

    #[test]
    fn raw_encode_block() {
        let data = include_bytes!("data/test.txt");
        let mut encoded = Vec::new();

        super::encode_block(data, &mut encoded);
        let mut decoded = Vec::new();

        super::decode_block(encoded.as_slice(), &mut decoded);

        assert_eq!(data, decoded.as_slice());
    }

    #[test]
    fn one_byte_at_a_time() {
        let input = include_bytes!("data/test.lz4.1");
        let mut d = Decoder::new(BufReader::new(input));
        assert!(!d.eof());
        let mut out = Vec::new();
        loop {
            match d.read_byte() {
                Ok(b) => out.push(b),
                Err(..) => break
            }
        }
        assert!(d.eof());
        assert!(out.as_slice() == include_bytes!("data/test.txt"));
    }

    #[test]
    fn random_byte_lengths() {
        let input = include_bytes!("data/test.lz4.1");
        let mut d = Decoder::new(BufReader::new(input));
        let mut out = Vec::new();
        let mut buf = [0u8; 40];
        loop {
            match d.read(buf.slice_to_mut(1 + rand::random::<uint>() % 40)) {
                Ok(n) => {
                    out.push_all(buf.slice_to(n));
                }
                Err(..) => break
            }
        }
        assert!(out.as_slice() == include_bytes!("data/test.txt"));
    }

    fn roundtrip(bytes: &[u8]) {
        let mut e = Encoder::new(MemWriter::new());
        e.write(bytes).unwrap();
        let (e, err) = e.finish();
        err.unwrap();
        let encoded = e.into_inner();

        let mut d = Decoder::new(BufReader::new(encoded.as_slice()));
        let decoded = d.read_to_end().unwrap();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(b"test");
        roundtrip(b"");
        roundtrip(include_bytes!("data/test.txt"));
    }

    #[bench]
    fn decompress_speed(bh: &mut test::Bencher) {
        let input = include_bytes!("data/test.lz4.9");
        let mut d = Decoder::new(BufReader::new(input));
        let mut output = [0u8; 65536];
        let mut output_size = 0;
        bh.iter(|| {
            d.r = BufReader::new(input);
            d.reset();
            output_size = d.read(&mut output).unwrap();
        });
        bh.bytes = output_size as u64;
    }
}
