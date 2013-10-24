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
use std::rt::io::extensions::{ReaderUtil, ReaderByteConversions};
use std::vec;
use std::num;

#[deriving(Eq)]
enum State {
    StreamStart,
    BlockStart,
    BlockEnd,

    // raw blocks of uncompressed data
    RawBlock(u32),

    // blocks of compressed data (no headers or checksums)
    DataStart,
    NeedsConsumption(u8, uint),
    DecompressStart(u8),
    Decompress(uint),

    EndOfStream,
    Cleanup,
}

enum ConsumptionResult {
    KeepGoing(uint),
    Consumed(uint),
    SomethingBad,
}

static BUF_SIZE: uint = 128 << 10;
static FLUSH_SIZE: uint = 1 << 16;
static MAGIC: u32 = 0x184d2204;

/// A decoder of an underlying lz4 stream. This is the decompressor used to
/// inflate an underlying stream.
pub struct Decoder<R> {
    /// This is the stream which is owned by this decoder. It may be moved out
    /// of, but the operation would also invalidate the `Decoder` instance.
    r: R,

    priv state: State,
    priv end: uint,
    priv start: uint,
    priv buf: ~[u8],
    priv remaining_bytes: uint,

    // metadata from the stream descriptor
    priv blk_checksum: bool,
    priv stream_checksum: bool,
}

impl<R: io::Reader + io::Seek> Decoder<R> {
    /// Creates a new decoder which will decompress the LZ4-encoded stream
    /// which will be read from `r`. This decoder will consume ownership of the
    /// reader, but it is accessible via the `r` field on the object returned.
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            r: r,
            state: StreamStart,
            start: 0,
            end: 0,
            buf: vec::from_elem(BUF_SIZE, 0u8),
            blk_checksum: false,
            stream_checksum: false,
            remaining_bytes: 0,
        }
    }

    fn inner_byte(&mut self) -> Option<u8> {
        match self.r.read_byte() {
            Some(b) => {
                self.remaining_bytes -= 1;
                Some(b)
            }
            None => None,
        }
    }

    fn inner_u16(&mut self) -> u16 {
        self.remaining_bytes -= 2;
        self.r.read_le_u16_()
    }

    fn inner_read(&mut self, dst: &mut [u8]) -> Option<uint> {
        match self.r.read(dst) {
            Some(n) => {
                self.remaining_bytes -= n;
                Some(n)
            }
            None => None,
        }
    }

    fn read_len(&mut self, code: u8) -> Option<uint> {
        let mut ret = code as uint;
        assert!(code < 16);
        if code == 0xf {
            loop {
                match self.inner_byte() {
                    None => return None,
                    Some(255) => ret += 255,
                    Some(n) => {
                        ret += n as uint;
                        break
                    }
                }
            }
        }
        return Some(ret);
    }

    fn consume(&mut self, len: uint, dst: &mut [u8]) -> ConsumptionResult {
        let in_output = self.flush(len, dst);
        if self.end + len > self.buf.len() {
            return KeepGoing(in_output);
        }

        let buf = unsafe {
            use std::cast;
            cast::transmute_copy(&self.buf.mut_slice(self.end, self.end + len))
        };
        match self.inner_read(buf) {
            Some(n) => {
                self.end += n;
                Consumed(in_output)
            }
            None => SomethingBad
        }
    }

    fn flush(&mut self, len: uint, dst: &mut [u8]) -> uint {
        if self.end + len > self.buf.len() {
            assert!(self.start > FLUSH_SIZE);
            let s = self.start - FLUSH_SIZE;

            // Copy bytes out into the destination
            let fill = num::min(dst.len(), s);
            info!("flushing out {}", fill);
            vec::bytes::copy_memory(dst, self.buf, fill);

            // Slide all the bytes back down in the internal buffer
            for i in range(0, fill) {
                self.buf[i] = self.buf[fill + i];
            }

            self.end -= fill;
            self.start -= fill;
            return fill;
        }
        return 0;
    }

    fn cp(&mut self, len: uint, decr: uint, dst: &mut [u8]) -> ConsumptionResult {
        let in_output = self.flush(len, dst);
        if self.end + len > self.buf.len() {
            return KeepGoing(in_output);
        }

        for i in range(0, len) {
            self.buf[self.end + i] = self.buf[self.start + i];
        }

        self.end += len;
        self.start += len - decr;
        return Consumed(in_output);
    }

    fn exec(&mut self, mut state: State, buf: &mut [u8]) -> Option<uint> {
        let mut offset = 0;
        loop {
            debug!("state: {:?} {:x} {:x} at {}", state, self.start, self.end,
                   self.r.tell());
            match state {
                StreamStart => {
                    match self.header() {
                        Some(()) => {}
                        None => return None,
                    }
                    state = BlockStart;
                }

                BlockStart => {
                    state = match self.r.read_le_u32_() {
                        0 => EndOfStream,
                        n if n & 0x80000000 != 0 => RawBlock(n & 0x7fffffff),
                        n => {
                            self.remaining_bytes = n as uint;
                            info!("bytes left {}", self.remaining_bytes);
                            DataStart
                        }
                    }
                }

                DataStart => {
                    //info!("remaining: {}", self.remaining_bytes as int);
                    assert!(self.remaining_bytes as int > 0);
                    let code = match self.inner_byte() {
                        Some(i) => i,
                        None => return None,
                    };
                    debug!("at: {:x} {} {:x}", self.r.tell(),
                           self.remaining_bytes as int, code);

                    // XXX: I/O errors
                    let len = match self.read_len(code >> 4) {
                        Some(l) => l, None => return None,
                    };
                    state = NeedsConsumption(code, len);
                }

                NeedsConsumption(code, len) => {
                    match self.consume(len, buf.mut_slice_from(offset)) {
                        // XXX: I/O error
                        SomethingBad => return None,
                        KeepGoing(amt_written) => {
                            self.state = state;
                            return Some(amt_written + offset);
                        }
                        Consumed(amt_written) => {
                            offset += amt_written;
                            if self.remaining_bytes == 0 {
                                state = BlockStart;
                            } else {
                                state = DecompressStart(code);
                            }
                        }
                    }
                }

                DecompressStart(code) => {
                    // XXX: I/O errors
                    let back = self.inner_u16();
                    debug!("got back: {}", back);
                    self.start = self.end - back as uint;

                    let len = match self.read_len(code & 0xf) {
                        Some(l) => l, None => return None,
                    };

                    state = Decompress(len);
                }

                Decompress(len) => {
                    let literal = self.end - self.start;
                    if literal < 4 {
                        static DECR: [uint, ..4] = [0, 3, 2, 3];
                        match self.cp(4, DECR[literal],
                                      buf.mut_slice_from(offset)) {
                            SomethingBad => unreachable!(),
                            KeepGoing(amt_written) => {
                                self.state = state;
                                return Some(amt_written + offset);
                            }
                            Consumed(amt_written) => offset += amt_written,
                        }
                    } else {
                        len += 4;
                    }
                    match self.cp(len, 0, buf.mut_slice_from(offset)) {
                        SomethingBad => unreachable!(),
                        KeepGoing(amt_written) => {
                            self.state = state;
                            return Some(amt_written + offset);
                        }
                        Consumed(amt_written) => {
                            offset += amt_written;
                            state = DataStart;
                        }
                    }
                }

                RawBlock(remaining) => {
                    let dst = buf.mut_slice_from(offset);
                    if dst.len() == 0 {
                        self.state = state;
                        return Some(offset);
                    }
                    if remaining == 0 {
                        state = BlockEnd;
                        continue
                    }
                    let amt = num::min(remaining as uint, dst.len());
                    match self.r.read(dst.mut_slice(offset, offset + amt)) {
                        Some(n) => {
                            offset += n;
                            state = RawBlock(remaining - n as u32);
                        }
                        None => return None
                    }
                }

                BlockEnd => {
                    // XXX: implement checksums
                    if self.blk_checksum {
                        let cksum = self.r.read_le_u32_();
                        debug!("ignoring block cksum: {:?}", cksum);
                    }
                    state = BlockStart;
                }

                EndOfStream => {
                    // XXX: implement checksums
                    if self.stream_checksum {
                        let cksum = self.r.read_le_u32_();
                        debug!("ignoring stream checksum: {:?}", cksum);
                    }
                    state = Cleanup;
                    self.start = 0;
                }

                Cleanup => {
                    let dst = buf.mut_slice_from(offset);
                    let src = self.buf.slice(self.start, self.end);
                    let amt = num::min(dst.len(), src.len());
                    if amt == 0 { return None }
                    vec::bytes::copy_memory(dst, src, amt);
                    self.start += amt;
                    self.state = Cleanup;
                    return Some(amt + offset);
                }
            }
        }
    }

    fn header(&mut self) -> Option<()> {
        // Make sure the magic number is what's expected.
        if self.r.read_le_u32_() != MAGIC { return None }

        let flg = match self.r.read_byte() { Some(i) => i, None => return None };
        let bd = match self.r.read_byte() { Some(i) => i, None => return None };

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

        // XXX: implement checksums
        let cksum = self.r.read_byte();
        debug!("ignoring header checksum: {:?}", cksum);
        return Some(());
    }
}

impl<R: io::Reader + io::Seek> io::Reader for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> Option<uint> {
        info!("reading {}", buf.len());
        let out = self.exec(self.state, buf);
        info!("got {:?}", out);
        out
    }

    fn eof(&mut self) -> bool { false }
}

#[cfg(test)]
mod test {
    use super::Decoder;
    use std::rt::io::extensions::ReaderUtil;
    use std::rt::io::mem::BufReader;
    use std::rt::io::Reader;
    use std::rand;

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
        let mut out = ~[];
        loop {
            match d.read_byte() {
                Some(b) => out.push(b),
                None => break
            }
        }
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
}
