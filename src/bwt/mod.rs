/*!

BWT (Burrows-Wheeler Transform) forward and backward transformation. Requires `bwt` feature, enabled by default

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

#![allow(missing_docs)]

use std::{cmp, fmt, io, iter, slice};
use std::num::NumCast;


pub mod dc;
pub mod mtf;

/// A base element for the transformation
pub type Symbol = u8;

pub const ALPHABET_SIZE: uint = 0x100;

/// Radix sorting primitive
pub struct Radix    {
    /// number of occurancies (frequency) per symbox
    pub freq    : [uint, ..ALPHABET_SIZE+1],
}

impl Radix  {
    /// create Radix sort instance
    pub fn new() -> Radix   {
        Radix   {
            freq : [0, ..ALPHABET_SIZE+1],
        }
    }

    /// reset counters
    /// allows the struct to be re-used
    pub fn reset(&mut self) {
        for fr in self.freq.iter_mut()   {
            *fr = 0;
        }
    }

    /// count elements in the input
    pub fn gather(&mut self, input: &[Symbol])  {
        for &b in input.iter()  {
            self.freq[b as uint] += 1;
        }
    }

    /// build offset table
    pub fn accumulate(&mut self)    {
        let mut n = 0;
        for freq in self.freq.iter_mut() {
            let f = *freq;
            *freq = n;
            n += f;
        }
    }

    /// return next byte position, advance it internally
    pub fn place(&mut self, b: Symbol)-> uint   {
        let pos = self.freq[b as uint];
        assert!(self.freq[b as uint] < self.freq[(b as uint)+1],
            "Unable to place symbol {} at offset {}",
            b, pos);
        self.freq[b as uint] += 1;
        pos
    }

    /// shift frequences to the left
    /// allows the offsets to be re-used after all positions are obtained
    pub fn shift(&mut self) {
        assert_eq!( self.freq[ALPHABET_SIZE-1], self.freq[ALPHABET_SIZE] );
        for i in iter::range_inclusive(1,ALPHABET_SIZE).rev()   {
            self.freq[i] = self.freq[i-1];
        }
        self.freq[0] = 0;
    }
}


/// Compute a suffix array from a given input string
/// Resulting suffixes are guaranteed to be alphabetically sorted
/// Run time: O(N^3), memory: N words (suf_array) + ALPHABET_SIZE words (Radix)
pub fn compute_suffixes<SUF: NumCast + ToPrimitive + fmt::Show>(input: &[Symbol], suf_array: &mut [SUF]) {
    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    debug!("SA compute input: {}", input);
    debug!("radix offsets: {}", radix.freq.as_slice());

    for (i,&ch) in input.iter().enumerate() {
        let p = radix.place(ch);
        suf_array[p] = NumCast::from(i).unwrap();
    }

    // bring the original offsets back
    radix.shift();

    for i in range(0, ALPHABET_SIZE)   {
        let lo = radix.freq[i];
        let hi = radix.freq[i+1];
        if lo == hi {
            continue
        }
        let slice = suf_array.slice_mut(lo,hi);
        debug!("\tsorting group [{}-{}) for symbol {}", lo, hi, i);
        slice.sort_by(|a,b| {
            iter::order::cmp(
                input.slice_from(a.to_uint().unwrap()).iter(),
                input.slice_from(b.to_uint().unwrap()).iter())
        });
    }

    debug!("sorted SA: {}", suf_array);
}

/// An iterator over BWT output
pub struct TransformIterator<'a, SUF: 'a> {
    input      : &'a [Symbol],
    suf_iter   : iter::Enumerate<slice::Items<'a,SUF>>,
    origin     : Option<uint>,
}

impl<'a, SUF> TransformIterator<'a, SUF> {
    /// create a new BWT iterator from the suffix array
    pub fn new(input: &'a [Symbol], suffixes: &'a [SUF]) -> TransformIterator<'a, SUF> {
        TransformIterator {
            input: input,
            suf_iter: suffixes.iter().enumerate(),
            origin: None,
        }
    }

    /// return the index of the original string
    pub fn get_origin(&self) -> uint {
        self.origin.unwrap()
    }
}

impl<'a, SUF: ToPrimitive + 'a> Iterator<Symbol> for TransformIterator<'a, SUF> {
    fn next(&mut self) -> Option<Symbol> {
        self.suf_iter.next().map(|(i,p)| {
            if p.to_uint().unwrap() == 0 {
                assert!( self.origin.is_none() );
                self.origin = Some(i);
                *self.input.last().unwrap()
            }else {
                self.input[p.to_uint().unwrap() - 1]
            }
        })
    }
}

/// Encode BWT of a given input, using the 'suf_array'
pub fn encode<'a, SUF: NumCast + ToPrimitive + fmt::Show>(input: &'a [Symbol], suf_array: &'a mut [SUF]) -> TransformIterator<'a, SUF> {
    compute_suffixes(input, suf_array);
    TransformIterator::new(input, suf_array)
}

/// Transform an input block into the output slice, all-inclusive version.
/// Returns the index of the original string in the output matrix.
pub fn encode_simple(input: &[Symbol]) -> (Vec<Symbol>, uint) {
    let mut suf_array = Vec::from_elem(input.len(), 0u);
    let mut iter = encode(input, suf_array.as_mut_slice());
    let output: Vec<Symbol> = iter.by_ref().collect();
    (output, iter.get_origin())
}


/// Compute an inversion jump table, needed for BWT decoding
pub fn compute_inversion_table<SUF: NumCast + fmt::Show>(input: &[Symbol], origin: uint, table: &mut [SUF]) {
    assert_eq!(input.len(), table.len());

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    table[radix.place(input[origin])] = NumCast::from(0i).unwrap();
    for (i,&ch) in input.slice_to(origin).iter().enumerate() {
        table[radix.place(ch)] = NumCast::from(i+1).unwrap();
    }
    for (i,&ch) in input.slice_from(origin+1).iter().enumerate() {
        table[radix.place(ch)] = NumCast::from(origin+2+i).unwrap();
    }
    //table[-1] = origin;
    debug!("inverse table: {}", table)
}

/// An iterator over inverse BWT
/// Run time: O(N), memory: N words (table)
pub struct InverseIterator<'a, SUF: 'a> {
    input      : &'a [Symbol],
    table      : &'a [SUF],
    origin     : uint,
    current    : uint,
}

impl<'a, SUF> InverseIterator<'a, SUF> {
    /// create a new inverse BWT iterator with a given input, origin, and a jump table
    pub fn new(input: &'a [Symbol], origin: uint, table: &'a [SUF]) -> InverseIterator<'a, SUF> {
        debug!("inverse origin={}, input: {}", origin, input);
        InverseIterator {
            input: input,
            table: table,
            origin: origin,
            current: origin,
        }
    }
}

impl<'a, SUF: ToPrimitive> Iterator<Symbol> for InverseIterator<'a, SUF> {
    fn next(&mut self) -> Option<Symbol> {
        if self.current == -1 {
            None
        }else {
            self.current = self.table[self.current].to_uint().unwrap() - 1;
            debug!("\tjumped to {}", self.current);
            let p = if self.current!=-1 {
                self.current
            }else {
                self.origin
            };
            Some(self.input[p])
        }
    }
}

/// Decode a BWT block, given it's origin, and using 'table' temporarily
pub fn decode<'a, SUF: NumCast + fmt::Show>(input: &'a [Symbol], origin: uint, table: &'a mut [SUF]) -> InverseIterator<'a, SUF> {
    compute_inversion_table(input, origin, table);
    InverseIterator::new(input, origin, table)
}

/// A simplified BWT decode function, which allocates a temporary suffix array
pub fn decode_simple(input: &[Symbol], origin: uint) -> Vec<Symbol> {
    let mut suf = Vec::from_elem(input.len(), 0 as uint);
    decode(input, origin, suf.as_mut_slice()).take(input.len()).collect()
}

/// Decode without additional memory, can be greatly optimized
/// Run time: O(n^2), Memory: 0n
fn decode_minimal(input: &[Symbol], origin: uint, output: &mut [Symbol]) {
    assert_eq!(input.len(), output.len());
    if input.len() == 0 {
        assert_eq!(origin, 0);
    }

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    let n = input.len();
    range(0,n).fold(origin, |i,j| {
        let ch = input[i];
        output[n-j-1] = ch;
        let offset = input.slice_to(i).iter().filter(|&k| *k==ch).count();
        radix.freq[ch as uint] + offset
    });
}


/// This structure is used to decode a stream of BWT blocks. This wraps an
/// internal reader which is read from when this decoder's read method is
/// called.
pub struct Decoder<R> {
    /// The internally wrapped reader. This is exposed so it may be moved out
    /// of. Note that if data is read from the reader while decoding is in
    /// progress the output stream will get corrupted.
    pub r: R,
    start  : uint,

    temp   : Vec<u8>,
    output : Vec<u8>,
    table  : Vec<uint>,

    header         : bool,
    max_block_size : uint,
    extra_memory   : bool,
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
            temp: Vec::new(),
            output: Vec::new(),
            table: Vec::new(),
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
        try!(self.r.push_at_least(n, n, &mut self.temp));

        let origin = try!(self.r.read_le_u32()) as uint;
        self.output.truncate(0);
        self.output.reserve(n);

        if self.extra_memory    {
            self.table.truncate(0);
            self.table.grow_fn(n, |_| 0);
            for ch in decode(self.temp.as_slice(), origin, self.table.as_mut_slice()) {
                self.output.push(ch);
            }
        }else   {
            self.output.grow_fn(n, |_| 0);
            decode_minimal(self.temp.as_slice(), origin, self.output.as_mut_slice());
        }

        self.start = 0;
        return Ok(true);
    }
}

impl<R: Reader> Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        if !self.header {
            try!(self.read_header());
            self.header = true;
        }
        let mut amt = dst.len();
        let dst_len = amt;

        while amt > 0 {
            if self.output.len() == self.start {
                let keep_going = try!(self.decode_block());
                if !keep_going {
                   break
                }
            }
            let n = cmp::min(amt, self.output.len() - self.start);
            slice::bytes::copy_memory(
                dst.slice_from_mut(dst_len - amt),
                self.output.slice(self.start, self.start + n)
                );
            self.start += n;
            amt -= n;
        }

        if dst_len == amt {
            Err(io::standard_error(io::EndOfFile))
        } else {
            Ok(dst_len - amt)
        }
    }
}


/// This structure is used to compress a stream of bytes using the BWT.
/// This is a wrapper around an internal writer which bytes will be written to.
pub struct Encoder<W> {
    w: W,
    buf: Vec<u8>,
    suf: Vec<uint>,
    wrote_header: bool,
    block_size: uint,
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
            buf: Vec::new(),
            suf: Vec::new(),
            wrote_header: false,
            block_size: block_size,
        }
    }

    fn encode_block(&mut self) -> io::IoResult<()> {
        let n = self.buf.len();
        try!(self.w.write_le_u32(n as u32));

        self.suf.truncate(0);
        self.suf.grow_fn(n, |_| n);
        let w = &mut self.w;

        {
            let mut iter = encode(self.buf.as_slice(), self.suf.as_mut_slice());
            for ch in iter {
                try!(w.write_u8(ch));
            }

            try!(w.write_le_u32(iter.get_origin() as u32));
        }
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
            try!(self.w.write_le_u32(self.block_size as u32));
            self.wrote_header = true;
        }

        while buf.len() > 0 {
            let amt = cmp::min( self.block_size - self.buf.len(), buf.len() );
            self.buf.push_all( buf.slice_to(amt) );

            if self.buf.len() == self.block_size {
                try!(self.encode_block());
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
    use std::io::{BufReader, MemWriter};
    use test::Bencher;
    use super::{encode, decode, Decoder, Encoder};

    fn roundtrip(bytes: &[u8], extra_mem: bool) {
        let mut e = Encoder::new(MemWriter::new(), 1<<10);
        e.write(bytes).unwrap();
        let (e, err) = e.finish();
        err.unwrap();
        let encoded = e.into_inner();

        let mut d = Decoder::new(BufReader::new(encoded.as_slice()), extra_mem);
        let decoded = d.read_to_end().unwrap();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(b"test", true);
        roundtrip(b"", true);
        roundtrip(include_bin!("../data/test.txt"), true);
    }

    #[test]
    fn decode_minimal() {
        roundtrip(b"abracadabra", false);
    }

    #[bench]
    fn decode_speed(bh: &mut Bencher) {
        let input = include_bin!("../data/test.txt");
        let n = input.len();
        let mut suf = Vec::from_elem(n, 0u16);
        let (output, origin) = {
            let mut to_iter = encode(input, suf.as_mut_slice());
            let out: Vec<u8> = to_iter.by_ref().collect();
            (out, to_iter.get_origin())
        };
        bh.iter(|| {
            let from_iter = decode(output.as_slice(), origin, suf.as_mut_slice());
            from_iter.last().unwrap();
        });
        bh.bytes = n as u64;
    }
}
