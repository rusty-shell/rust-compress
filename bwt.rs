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
# #[allow(unused_must_use)];
use std::io::{MemWriter, MemReader};
use compress::bwt;

// Encode some text
let text = "some text";
let mut e = bwt::Encoder::new(MemWriter::new(), 4 << 20);
e.write_str(text);
let (encoded, _) = e.finish();

// Decode the encoded text
let mut d = bwt::Decoder::new(MemReader::new(encoded.unwrap()), true);
let decoded = d.read_to_end().unwrap();

assert_eq!(decoded.as_slice(), text.as_bytes());
```

# Credit

This is an original (mostly trivial) implementation.

*/

use std::{io, iter, num, vec};

pub static total_symbols: uint = 0x100;

/// Radix sorting primitive
pub struct Radix    {
    /// number of occurancies (frequency) per symbox
    freq    : [uint, ..total_symbols+1],
}

impl Radix  {
    /// create Radix sort instance
    pub fn new() -> Radix   {
        Radix   {
            freq : [0, ..total_symbols+1],
        }
    }

    /// reset counters
    /// allows the struct to be re-used
    pub fn reset(&mut self) {
        for fr in self.freq.mut_iter()   {
            *fr = 0;
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
        for freq in self.freq.mut_iter() {
            let f = *freq;
            *freq = n;
            n += f;
        }
    }

    /// return next byte position, advance it internally
    pub fn place(&mut self, b: u8)-> uint   {
        let pos = self.freq[b];
        assert!(self.freq[b] < self.freq[(b as uint)+1],
            "Unable to place symbol {} at offset {}",
            b, pos);
        self.freq[b] += 1;
        pos
    }

    /// shift frequences to the left
    /// allows the offsets to be re-used after all positions are obtained
    pub fn shift(&mut self) {
        assert_eq!( self.freq[total_symbols-1], self.freq[total_symbols] );
        for i in iter::range_inclusive(1,total_symbols).rev()   {
            self.freq[i] = self.freq[i-1];
        }
        self.freq[0] = 0;
    }
}


/// Stand-alone encoding/decoding methods, operating on single blocks.
pub type Suffix = uint;

/// Encode an input block and call 'fn_out' on each output byte, using 'suf' array temporarily.
/// Returns the index of the original string in the output matrix.
/// Run time: O(n^3), memory: 4n
pub fn encode_brute(input: &[u8], suf: &mut [Suffix], fn_out: |u8|) -> Suffix {
    assert_eq!(suf.len(), input.len());
    if input.is_empty() { return 0 }

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    debug!("encode input: {:?}", input);
    debug!("radix offsets: {:?}", radix.freq);

    for (i,&ch) in input.iter().enumerate() {
        let p = radix.place(ch);
        suf[p] = i;
    }

    // bring the original offsets back
    radix.shift();

    for i in range(0,total_symbols)   {
        let lo = radix.freq[i];
        let hi = radix.freq[i+1];
        if lo == hi {
            continue
        }
        let slice = suf.mut_slice(lo,hi);
        debug!("sorting group [{}-{}) for symbol {}", lo, hi, i);
        slice.sort_by(|&a,&b| {
            iter::order::cmp(
                input.slice_from(a).iter(),
                input.slice_from(b).iter())
        });
    }

    debug!("encode suf: {:?}", suf);

    // the alphabetically first suffix is always $
    let mut origin = None::<Suffix>;
    fn_out(*input.last().unwrap());

    for (i,&p) in suf.iter().enumerate() {
        if p==0 {
            assert!( origin.is_none() );
            origin = Some(i+1); // yielding $
        }else   {
            fn_out(input[p-1]);
        }
    }

    assert!( origin.is_some() );
    origin.unwrap()
}

/// Encode an input block into the output slice, using 'suf' array temporarily.
/// Returns the index of the original string in the output matrix.
pub fn encode_mem(input: &[u8], suf: &mut [Suffix], output: &mut [u8]) -> Suffix {
    let mut size = 0u;
    let origin = encode_brute(input, suf, |ch| {output[size] = ch; size+=1;});
    assert_eq!(size, input.len());
    origin
}

/// Decode in a standard fashion, calling 'fn_out' on every output symbol,
/// and using 'suf' array temporarily.
/// Run time: O(n), memory: 4n
pub fn decode_std(input: &[u8], origin: Suffix, suf: &mut [Suffix], fn_out: |u8|) {
    assert_eq!(input.len(), suf.len())
    if input.is_empty() {
        assert_eq!(origin, 0);
        return
    }

    debug!("decode origin={}, input: {:?}", origin, input)

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    // the input stream virtually has $ inserted at origin
    for (i,&ch) in input.iter().enumerate() {
        let p = radix.place(ch);
        suf[p] = if i<origin {i} else {i+1};
    }
    //suf[-1] = origin;

    debug!("decode table: {:?}",suf)

    let mut i = origin;
    for _ in input.iter() {
        assert!(i!=0, "Invalid BWT stream, origin={}", origin);
        i = suf[i-1];
        debug!("\tjumped to {}", i);
        let p = if i>origin {i-1} else {i};
        fn_out(input[p]);
    }
    assert_eq!(i, 0);
}

/// Decode into output slice
pub fn decode_mem(input: &[u8], origin: Suffix, suf: &mut [Suffix], output: &mut [u8]) {
    let mut size = 0u;
    decode_std(input, origin, suf, |ch| {output[size] = ch; size+=1;});
    assert_eq!(size, input.len());
}

/// Decode without additional memory, can be greatly optimized
/// Run time: O(n^2), Memory: 0n
fn decode_minimal(input: &[u8], origin: Suffix, output: &mut [u8]) {
    assert_eq!(input.len(), output.len());

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    let n = input.len();
    let mut i = 0;
    for j in range(0,n) {
        let ch = input[if i>origin {i-1} else {i}];
        output[n-1-j] = ch;
        i = (if i>origin {0} else {1}) + radix.freq[ch] +
            input.slice_to(i).iter().count(|&k| k==ch);
    }
    assert_eq!(i, origin);
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
    /// 'extra_mem' switch allows allocating extra N words of memory for better performance
    pub fn new(r: R, extra_mem: bool) -> Decoder<R> {
        Decoder {
            r: r,
            start: 0,
            temp: ~[],
            output: ~[],
            table: ~[],
            header: false,
            max_block_size: 0,
            extra_memory: extra_mem,
        }
    }

    /// Resets this decoder back to its initial state. Note that the underlying
    /// stream is not seeked on or has any alterations performed on it.
    pub fn reset(&mut self) {
        self.header = false;
        self.start = 0;
    }

    fn read_header(&mut self) -> io::IoResult<()> {
        match self.r.read_le_u32() {
            Ok(size) => {
                self.max_block_size = size as uint;
                debug!("max size: {}", self.max_block_size);
                Ok(())
            },
            Err(e) => Err(e),
        }
    }

    fn decode_block(&mut self) -> io::IoResult<bool> {
        let n = match self.r.read_le_u32() {
            Ok(n) => n as uint,
            Err(ref e) if e.kind == io::EndOfFile => return Ok(false),
            Err(e) => return Err(e),
        };
        if n == 0 { return Ok(false) }

        self.temp.truncate(0);
        self.temp.reserve(n);
        if_ok!(self.r.push_bytes(&mut self.temp, n));

        let origin = if_ok!(self.r.read_le_u32()) as uint;
        self.output.truncate(0);
        self.output.grow_fn(n, |_| 0);  //Option: do not initialize

        if self.extra_memory    {
            self.table.truncate(0);
            self.table.grow_fn(n, |_| 0);
            decode_mem(self.temp, origin, self.table, self.output);
        }else   {
            decode_minimal(self.temp, origin, self.output);
        }

        self.start = 0;
        return Ok(true);
    }
}

impl<R: Reader> Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        if !self.header {
            if_ok!(self.read_header());
            self.header = true;
        }
        let mut amt = dst.len();
        let len = amt;

        while amt > 0 {
            if self.output.len() == self.start {
                let keep_going = if_ok!(self.decode_block());
                if !keep_going {
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

        if len == amt {
            Err(io::standard_error(io::EndOfFile))
        } else {
            Ok(len - amt)
        }
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

    fn encode_block(&mut self) -> io::IoResult<()> {
        let n = self.buf.len();
        if_ok!(self.w.write_le_u32(n as u32));

        self.suf.truncate(0);
        self.suf.grow_fn(n, |_| n);
        let w = &mut self.w;

        let origin = encode_brute(self.buf, self.suf, |ch| w.write_u8(ch).unwrap());

        if_ok!(w.write_le_u32(origin as u32));
        self.buf.truncate(0);

        Ok(())
    }

    /// This function is used to flag that this session of compression is done
    /// with. The stream is finished up (final bytes are written), and then the
    /// wrapped writer is returned.
    pub fn finish(mut self) -> (W, io::IoResult<()>) {
        let result = self.flush();
        (self.w, result)
    }
}

impl<W: Writer> Writer for Encoder<W> {
    fn write(&mut self, mut buf: &[u8]) -> io::IoResult<()> {
        if !self.wrote_header {
            if_ok!(self.w.write_le_u32(self.block_size as u32));
            self.wrote_header = true;
        }

        while buf.len() > 0 {
            let amt = num::min( self.block_size - self.buf.len(), buf.len() );
            self.buf.push_all( buf.slice_to(amt) );

            if self.buf.len() == self.block_size {
                if_ok!(self.encode_block());
            }
            buf = buf.slice_from(amt);
        }
        Ok(())
    }

    fn flush(&mut self) -> io::IoResult<()> {
        let ret = if self.buf.len() > 0 {
            self.encode_block()
        } else {
            Ok(())
        };
        ret.and(self.w.flush())
    }
}


#[cfg(test)]
mod test {
    use extra::test;
    use std::io::{BufReader, MemWriter};
    use std::vec;
    use super::{encode_mem, decode_std, Suffix, Decoder, Encoder};

    fn roundtrip(bytes: &[u8]) {
        let mut e = Encoder::new( MemWriter::new(), 1<<10 );
        e.write(bytes).unwrap();
        let (e, err) = e.finish();
        err.unwrap();
        let encoded = e.unwrap();

        let mut d = Decoder::new( BufReader::new(encoded), true );
        let decoded = d.read_to_end().unwrap();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(bytes!("test"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("data/test.txt"));
    }

    #[bench]
    fn decode_speed(bh: &mut test::BenchHarness) {
        let input = include_bin!("data/test.txt");
        let n = input.len();
        let mut suf = vec::from_elem(n, 0 as Suffix);
        let mut output = vec::from_elem(n, 0u8);
        let origin = encode_mem(input, suf, output);
        bh.iter(|| {
            decode_std(output, origin, suf, |_| ());
        });
        bh.bytes = n as u64;
    }
}
