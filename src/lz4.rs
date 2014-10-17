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

use std::io;
use std::cmp;
use std::slice;
use std::vec::Vec;

const MAGIC: u32 = 0x184d2204;

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
                    static DECR: [uint, ..4] = [0, 3, 2, 3];
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
            *self.output.get_mut(end + i) = (*self.output)[self.start + i];
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

        let mut bits = [0, ..3];
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

        static MAX_SIZES: [uint, ..8] =
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
        let reference = include_bin!("data/test.txt");
        test_decode(include_bin!("data/test.lz4.1"), reference);
        test_decode(include_bin!("data/test.lz4.2"), reference);
        test_decode(include_bin!("data/test.lz4.3"), reference);
        test_decode(include_bin!("data/test.lz4.4"), reference);
        test_decode(include_bin!("data/test.lz4.5"), reference);
        test_decode(include_bin!("data/test.lz4.6"), reference);
        test_decode(include_bin!("data/test.lz4.7"), reference);
        test_decode(include_bin!("data/test.lz4.8"), reference);
        test_decode(include_bin!("data/test.lz4.9"), reference);
    }

    #[test]
    fn one_byte_at_a_time() {
        let input = include_bin!("data/test.lz4.1");
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
        assert!(out.as_slice() == include_bin!("data/test.txt"));
    }

    #[test]
    fn random_byte_lengths() {
        let input = include_bin!("data/test.lz4.1");
        let mut d = Decoder::new(BufReader::new(input));
        let mut out = Vec::new();
        let mut buf = [0u8, ..40];
        loop {
            match d.read(buf.slice_to_mut(1 + rand::random::<uint>() % 40)) {
                Ok(n) => {
                    out.push_all(buf.slice_to(n));
                }
                Err(..) => break
            }
        }
        assert!(out.as_slice() == include_bin!("data/test.txt"));
    }

    fn roundtrip(bytes: &[u8]) {
        let mut e = Encoder::new(MemWriter::new());
        e.write(bytes).unwrap();
        let (e, err) = e.finish();
        err.unwrap();
        let encoded = e.unwrap();

        let mut d = Decoder::new(BufReader::new(encoded.as_slice()));
        let decoded = d.read_to_end().unwrap();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(b"test");
        roundtrip(b"");
        roundtrip(include_bin!("data/test.txt"));
    }

    #[bench]
    fn decompress_speed(bh: &mut test::Bencher) {
        let input = include_bin!("data/test.lz4.9");
        let mut d = Decoder::new(BufReader::new(input));
        let mut output = [0u8, ..65536];
        let mut output_size = 0;
        bh.iter(|| {
            d.r = BufReader::new(input);
            d.reset();
            output_size = d.read(output).unwrap();
        });
        bh.bytes = output_size as u64;
    }
}
