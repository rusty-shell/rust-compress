/*!

LZ4 Decompression

This module contains an implementation in Rust of decompression of LZ4-encoded
streams. This is exposed as a standard `Reader` interface wrapping an
underlying `Reader` interface. In this manner, the decompressor can be viewed
as a stream. It maintains its own internal buffer, but the size of this is
constant as the stream is read.

# Example

```rust
let stream = Path::new("path/to/file.lz4").open_reader(io::Open);
let decompressed = l4z::Decoder::new(stream).read_to_end();
```

# Credit

This implementation is largely based on Branimir Karadžić's implementation which
can be found at https://github.com/bkaradzic/go-lz4.

*/

use std::rt::io;
use std::rt::io::extensions::{ReaderUtil, ReaderByteConversions,
                              WriterByteConversions};
use std::vec;
use std::num;

static MAGIC: u32 = 0x184d2204;

pub struct BlockDecoder<'self> {
    input: &'self [u8],
    output: &'self mut ~[u8],
    cur: uint,

    start: uint,
    end: uint,
}

impl<'self> BlockDecoder<'self> {
    /// Decodes this block of data from 'input' to 'output', returning the
    /// number of valid bytes in the output.
    pub fn decode(&mut self) -> uint {
        while self.cur < self.input.len() {
            let code = self.bump();
            debug!("block with code: {:x}", code);
            // Extract a chunk of data from the input to the output.
            {
                let len = self.length(code >> 4);
                debug!("consume len {}", len);
                self.grow_output(self.end + len);
                vec::bytes::copy_memory(self.output.mut_slice_from(self.end),
                                        self.input.slice_from(self.cur), len);
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

    fn cp(&mut self, len: uint, decr: uint) {
        self.grow_output(self.end + len);
        for i in range(0, len) {
            self.output[self.end + i] = self.output[self.start + i];
        }

        self.end += len;
        self.start += len - decr;
    }

    fn grow_output(&mut self, target: uint) {
        if self.output.len() < target {
            debug!("growing to {}", target);
            let target = target - self.output.len();
            self.output.grow(target, &0);
        }
    }
}

pub struct Decoder<R> {
    r: R,

    priv temp: ~[u8],
    priv output: ~[u8],

    priv start: uint,
    priv end: uint,
    priv eof: bool,

    priv header: bool,
    priv blk_checksum: bool,
    priv stream_checksum: bool,
    priv max_block_size: uint,
}

impl<R: io::Reader> Decoder<R> {
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            r: r,
            temp: ~[],
            output: ~[],
            header: false,
            blk_checksum: false,
            stream_checksum: false,
            start: 0,
            end: 0,
            eof: false,
            max_block_size: 0,
        }
    }

    pub fn reset(&mut self) {
        self.header = false;
        self.eof = false;
        self.start = 0;
        self.end = 0;
    }

    fn read_header(&mut self) -> Option<()> {
        // Make sure the magic number is what's expected.
        if self.r.read_le_u32_() != MAGIC { return None }

        let mut bits = [0, ..3];
        if self.r.read(bits.mut_slice_to(2)).is_none() { return None }
        let flg = bits[0];
        let bd = bits[1];

        // bits 7/6, the version number. Right now this must be 1
        if (flg >> 6) != 0b01 { return None }
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
        let max_block_size = MAX_SIZES[(bd >> 4) & 0x7];
        // bits 3-0 are reserved

        // read off other portions of the stream
        let size = if stream_size {Some(self.r.read_le_u64_())} else {None};
        assert!(!preset_dictionary, "preset dictionaries not supported yet");

        debug!("blk: {}", self.blk_checksum);
        debug!("stream: {}", self.stream_checksum);
        debug!("max size: {}", max_block_size);
        debug!("stream size: {:?}", size);

        self.max_block_size = max_block_size;

        // XXX: implement checksums
        let cksum = self.r.read_byte();
        debug!("ignoring header checksum: {:?}", cksum);
        return Some(());
    }

    fn decode_block(&mut self) -> bool {
        match self.r.read_le_u32_() {
            // final block, we're done here
            0 => return false,

            // raw block to read
            n if n & 0x80000000 != 0 => {
                let amt = (n & 0x7fffffff) as uint;
                if self.output.len() < amt {
                    let grow = amt - self.output.len();
                    self.output.grow(grow, &0);
                }
                match self.r.read(self.output.mut_slice_to(amt)) {
                    Some(n) => assert_eq!(n, amt),
                    None => fail!(),
                }
                self.start = 0;
                self.end = amt;
            }

            // actual block to decompress
            n => {
                let n = n as uint;
                if self.temp.len() < n {
                    let grow = n - self.temp.len();
                    self.temp.grow(grow, &0);
                }
                assert!(n <= self.temp.len());
                match self.r.read(self.temp.mut_slice_to(n as uint)) {
                    Some(i) => assert_eq!(n, i),
                    None => fail!(),
                }
                let target = num::min(self.max_block_size, 4 * n / 3);
                if self.output.len() < target {
                    let grow = target - self.output.len();
                    self.output.grow(grow, &0);
                }
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
            let cksum = self.r.read_le_u32_();
            debug!("ignoring block checksum {:?}", cksum);
        }
        return true;
    }
}

impl<R: io::Reader> io::Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> Option<uint> {
        if self.eof { return None }
        if !self.header {
            self.read_header();
            self.header = true;
        }
        let mut amt = dst.len();
        let len = amt;

        while amt > 0 {
            if self.start == self.end {
                if !self.decode_block() {
                    self.eof = true;
                    break;
                }
            }
            let n = num::min(amt, self.end - self.start);
            vec::bytes::copy_memory(dst.mut_slice_from(len - amt),
                                    self.output.slice_from(self.start), n);
            self.start += n;
            amt -= n;
        }

        return Some(len - amt);
    }

    fn eof(&mut self) -> bool { return self.eof }
}

enum EncoderState {
    NothingWritten,
    NewBlock,
    Accumulate,
}

/// XXX: wut
pub struct Encoder<W> {
    w: W,

    priv state: EncoderState,
    priv buf: ~[u8],
    priv pos: uint,
}

impl<W: io::Writer> Encoder<W> {
    /// XXX: wut
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            w: w,
            state: NothingWritten,
            buf: vec::from_elem(10, 0u8),
            pos: 0,
        }
    }

    fn exec(&mut self, mut state: EncoderState, buf: &[u8]) {
        loop {
            match state {
                NothingWritten => {
                    self.w.write_le_u32_(MAGIC);
                    // version 01, turn on block independence, but turn off
                    // everything else (we have no checksums right now).
                    self.w.write_u8_(0b01_100000);
                    // Maximum block size is 256KB
                    self.w.write_u8_(0b0_101_0000);
                    // XXX: this checksum is just plain wrong.
                    self.w.write_u8_(0);
                    state = Accumulate;
                }

                Accumulate => {
                    let amt = num::min(buf.len(), self.buf.len() - self.pos);
                    assert!(amt > 0);
                    {
                        let dst = self.buf.mut_slice(self.pos, self.pos + amt);
                        vec::bytes::copy_memory(dst, buf.slice_to(amt), amt);
                    }

                    if self.pos == self.buf.len() {
                        state = NewBlock;
                    } else {
                        break
                    }
                }

                NewBlock => {
                }
            }
        }
    }
}

impl<W: io::Writer> io::Writer for Encoder<W> {
    fn write(&mut self, buf: &[u8]) {
        self.exec(self.state, buf)
    }

    fn flush(&mut self) {
    }
}

#[cfg(test)]
mod test {
    use extra::test;
    use std::rand;
    use std::rt::io::Reader;
    use std::rt::io::extensions::ReaderUtil;
    use std::rt::io::mem::BufReader;
    use super::Decoder;

    fn test_decode(input: &[u8], output: &[u8]) {
        let mut d = Decoder::new(BufReader::new(input));

        let got = d.read_to_end();
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
        let mut out = ~[];
        loop {
            match d.read_byte() {
                Some(b) => out.push(b),
                None => break
            }
        }
        assert!(d.eof());
        assert!(out.as_slice() == include_bin!("data/test.txt"));
    }

    #[test]
    fn random_byte_lengths() {
        let input = include_bin!("data/test.lz4.1");
        let mut d = Decoder::new(BufReader::new(input));
        let mut out = ~[];
        let mut buf = [0u8, ..40];
        loop {
            match d.read(buf.mut_slice_to(1 + rand::random::<uint>() % 40)) {
                Some(n) => {
                    out.push_all(buf.slice_to(n));
                }
                None => break
            }
        }
        assert!(out.as_slice() == include_bin!("data/test.txt"));
    }

    #[bench]
    fn decompress_speed(bh: &mut test::BenchHarness) {
        let input = include_bin!("data/test.lz4.9");
        let mut d = Decoder::new(BufReader::new(input));
        let mut output = [0u8, ..65536];
        do bh.iter {
            d.r = BufReader::new(input);
            d.reset();
            d.read(output);
        }
        bh.bytes = input.len() as u64;
    }
}
