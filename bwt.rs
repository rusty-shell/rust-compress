/*!

BWT (Burrows-Wheeler Transform) forward and backward transformation

This module contains a bruteforce implementation of BWT encoding in Rust as well as standard decoding.
These are exposed as a standard `Reader` and `Writer` interfaces wrapping an underlying stream.

BWT output stream places together symbols with similar leading contexts. This reshaping of the entropy
allows further stages to deal with repeated sequences of symbols for better compression.

Typical compression schemes are:
BWT + RLE (+ EC)
RLE + BWT + MTF + RLE + EC  : bzip2
BWT + DC + EC               : ybs

Where the stage families are:
BWT: BWT (Burrows-Wheeler Transform), ST (Shindler transform)
RLE: RLE (Run-Length Encoding)
MTF: MTF (Move-To-Front), WFC (Weighted Frequency Coding)
DC: DC (Distance Coding), IF (Inverse Frequencies)
EC (Entropy Coder): Huffman, Arithmetic, RC (Range Coder)


# Example

```rust
let stream = Path::new("path/to/file.bwt");
let decompressed = bwt::Decoder::new(stream,true).read_to_end();
```

# Credit

This is an original (mostly trivial) implementation.

*/

use std::{iter, num, vec};

static MAGIC    : u32   = 0x74776272;	//=rbwt

/// Radix sorting primitive
pub struct Radix    {
    /// number of occurancies (frequency) per symbox
    freq    : [uint,..0x101],
}

impl Radix  {
    /// create Radix sort instance
    pub fn new() -> Radix   {
        Radix   {
            freq : [0,..0x101],
        }
    }

    /// reset counters
    /// allows the struct to be re-used
    pub fn reset(&mut self) {
        for i in range(0,0x101)   {
            self.freq[i] = 0;
        }
    }

    /// count elements in the input
    pub fn gather(&mut self, input: &[u8])  {
        for &b in input.iter()  {
            self.freq[b] += 1;
        }
    }

    /// build offset table
    pub fn accumulate(&mut self)    {
        let mut n = 0;
        for i in range(0,0x100)   {
            let f = self.freq[i];
            self.freq[i] = n;
            n += f;
        }
        self.freq[0x100] = n;
    }

    /// return next byte position
    pub fn position(&mut self, b: u8)-> uint   {
        let pos = self.freq[b];
        self.freq[b] += 1;
        assert!( self.freq[b] <= self.freq[b+1] );
        pos
    }

    /// shift frequences to the left
    /// allows the offsets to be re-used after all positions are obtained
    pub fn shift(&mut self) {
        assert_eq!( self.freq[0x100], self.freq[0x100] );
        for i in iter::range_inclusive(1,0x100).rev()   {
            self.freq[i] = self.freq[i-1];
        }
        self.freq[0] = 0;
    }
}


/// This structure is used to decode a stream of BWT blocks. This wraps an
/// internal reader which is read from when this decoder's read method is
/// called.
pub struct Decoder<R> {
    /// The internally wrapped reader. This is exposed so it may be moved out
    /// of. Note that if data is read from the reader while decoding is in
    /// progress the output stream will get corrupted.
    r: R,
    priv start  : uint,

    priv temp   : ~[u8],
    priv output : ~[u8],
    priv table  : ~[uint],

    priv header         : bool,
    priv max_block_size : uint,
    priv extra_memory   : bool,
}

impl<R: Reader> Decoder<R> {
    /// Creates a new decoder which will read data from the given stream. The
    /// inner stream can be re-acquired by moving out of the `r` field of this
    /// structure.
    /// 'extra' switch allows allocating extra N words of memory for better performance
    pub fn new(r: R, extra: bool) -> Decoder<R> {
        Decoder {
            r: r,
            start: 0,
            temp: ~[],
            output: ~[],
            table: ~[],
            header: false,
            max_block_size: 0,
            extra_memory: extra,
        }
    }

    /// Resets this decoder back to its initial state. Note that the underlying
    /// stream is not seeked on or has any alterations performed on it.
    pub fn reset(&mut self) {
        self.header = false;
        self.start = 0;
    }

    fn read_header(&mut self) -> Option<()> {
        if self.r.read_le_u32() != MAGIC { return None }
        self.max_block_size = self.r.read_le_u32() as uint;

        debug!("max size: {}", self.max_block_size);

        return Some(());
    }

    fn decode_block(&mut self) -> bool {
        let n = self.r.read_le_u32() as uint;
        if n==0 { return false }

        //TODO: insert a dummy $ to avoid an extra if later on
        self.temp.truncate(0);
        self.temp.reserve(n);
        self.r.push_bytes(&mut self.temp, n);

        let mut radix = Radix::new();
        radix.gather( self.temp );
        radix.accumulate();

        let origin = self.r.read_le_u32() as uint + 1;
        self.output.truncate(0);
        self.output.reserve(n);

        if self.extra_memory    {
            self.table.truncate(0);
            self.table.grow_fn(n+1, |_| 0);
            for i in range(0,n) {
                let b = self.temp[i];
                let p = radix.position(b);
                self.table[p+1] = if i<origin {i} else {i+1};
            }
            let mut i = origin;
            for _ in range(0, n) {
                i = self.table[i];
                let j = if i>origin {i-1} else {i};
                self.output.push( self.temp[j] );
            }
            assert_eq!(i, 0);
        }else   {
            self.output.grow_fn(n, |_| 0);
            let mut i = 0;
            for j in range(0,n) {
                let b = self.temp[if i>origin {i-1} else {i}];
                self.output[n-1-j] = b;
                i = (if i>origin {0} else {1}) + radix.freq[b] +
                    self.temp.slice_to(i).iter().count(|&k| k==b);
            }
            assert_eq!(i, origin);
        }

        self.start = 0;
        return true;
    }
}

impl<R: Reader> Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> Option<uint> {
        if !self.header {
            self.read_header();
            self.header = true;
        }
        let mut amt = dst.len();
        let len = amt;

        while amt > 0 {
            if self.output.len() == self.start {
                if !self.decode_block() {
                   break
                }
            }
            let n = num::min( amt, self.output.len() - self.start );
            vec::bytes::copy_memory(
                dst.mut_slice_from(len - amt),
                self.output.slice_from(self.start)
                );
            self.start += n;
            amt -= n;
        }

        if len == amt {None} else {Some(len - amt)}
    }
}


/// This structure is used to compress a stream of bytes using the BWT.
/// This is a wrapper around an internal writer which bytes will be written to.
pub struct Encoder<W> {
    priv w: W,
    priv buf: ~[u8],
    priv suf: ~[uint],
    priv wrote_header: bool,
    priv block_size: uint,
}

impl<W: Writer> Encoder<W> {
    /// Creates a new encoder which will have its output written to the given
    /// output stream. The output stream can be re-acquired by calling
    /// `finish()`
    /// 'block_size' is idealy as big as your input, unless you know for sure that
    /// the input consists of multiple parts of different nature. Often set as 4Mb.
    pub fn new(w: W, block_size: uint) -> Encoder<W> {
        Encoder {
            w: w,
            buf: ~[],
            suf: ~[],
            wrote_header: false,
            block_size: block_size,
        }
    }

    fn encode_block(&mut self) {
        let n = self.buf.len();
        self.w.write_le_u32(n as u32);

        let mut radix = Radix::new();
        radix.gather( self.buf );
        radix.accumulate();

        self.suf.truncate(0);
        self.suf.grow_fn(n, |_| n);
        for i in range(0,n) {
            let b = self.buf[i];
            let p = radix.position(b);
            self.suf[p] = i;
        }

        for i in range(0,256)   {
            let lo = radix.freq[i];
            let hi = radix.freq[i+1];
            let slice = self.suf.mut_slice(lo,hi);
            slice.sort_by(|&a,&b| {
                iter::order::cmp(
                    self.buf.slice_from(a).iter(),
                    self.buf.slice_from(b).iter())
            });
        }

        let mut origin = n;
        self.w.write_u8( self.buf[n-1] );

        for i in range(0,n) {
            let s = self.suf[i];
            if s==0 {
                assert!( origin == n );
                origin = i;
            }else   {
                let b = self.buf[s-1];
                self.w.write_u8( b );
            }
        }
        assert!( origin != n );
        self.w.write_le_u32(origin as u32);
        self.buf.truncate(0);
    }

    /// This function is used to flag that this session of compression is done
    /// with. The stream is finished up (final bytes are written), and then the
    /// wrapped writer is returned.
    pub fn finish(mut self) -> W {
        self.flush();
        self.w
    }
}

impl<W: Writer> Writer for Encoder<W> {
    fn write(&mut self, mut buf: &[u8]) {
        if !self.wrote_header {
            self.w.write_le_u32(MAGIC);
            self.w.write_le_u32(self.block_size as u32);
            self.wrote_header = true;
        }

        while buf.len() > 0 {
            let amt = num::min( self.block_size - self.buf.len(), buf.len() );
            self.buf.push_all( buf.slice_to(amt) );

            if self.buf.len() == self.block_size {
                self.encode_block();
            }
            buf = buf.slice_from(amt);
        }
    }

    fn flush(&mut self) {
        if self.buf.len() > 0 {
            self.encode_block();
        }
    }
}


#[cfg(test)]
mod test {
    use extra::test;
    //use std::rand;
    use std::io::{BufReader, MemWriter};
    use super::{Decoder, Encoder};

    fn test_decode(input: &[u8], output: &[u8], extra_memory: bool) {
        let mut d = Decoder::new( BufReader::new(input), extra_memory );

        let got = d.read_to_end();
        assert!( got.as_slice() == output );
    }

    #[test]
    fn decode() {
        let reference = include_bin!("data/test.txt");
        test_decode(include_bin!("data/test.bwt"), reference, true);
        test_decode(include_bin!("data/test.bwt"), reference, false);
    }

    fn roundtrip(bytes: &[u8]) {
        let mut e = Encoder::new( MemWriter::new(), 1<<10 );
        e.write(bytes);
        let encoded = e.finish().unwrap();

        let mut d = Decoder::new( BufReader::new(encoded), true );
        let decoded = d.read_to_end();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(bytes!("test"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("data/test.txt"));
    }

    #[bench]
    fn decompress_speed(bh: &mut test::BenchHarness) {
        let input = include_bin!("data/test.bwt");
        let mut d = Decoder::new( BufReader::new(input), true );
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
