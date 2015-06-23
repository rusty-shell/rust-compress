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
use std::io::{BufWriter, BufReader, Read, Write};
use compress::bwt;

// Encode some text
let text = "some text";
let mut e = bwt::Encoder::new(BufWriter::new(Vec::new()), 4 << 20);
e.write(text.as_bytes()).unwrap();
let (encoded, _) = e.finish();
let inner = encoded.into_inner().unwrap();

// Decode the encoded text
let mut d = bwt::Decoder::new(BufReader::new(&inner[..]), true);
let mut decoded = Vec::new();
d.read_to_end(&mut decoded).unwrap();

assert_eq!(&decoded[..], text.as_bytes());
```

# Credit

This is an original (mostly trivial) implementation.

*/

#![allow(missing_docs)]

extern crate num;

use std::{cmp, fmt, intrinsics, slice};
use std::iter::{self, Extend, repeat};
use std::io::{self, Read, Write};
use self::num::traits::{NumCast, ToPrimitive};

use super::byteorder::{self, LittleEndian, WriteBytesExt, ReadBytesExt};
use super::{byteorder_err_to_io, ReadExact};

pub mod dc;
pub mod mtf;

/// A base element for the transformation
pub type Symbol = u8;

pub const ALPHABET_SIZE: usize = 0x100;

/// Radix sorting primitive
pub struct Radix    {
    /// number of occurancies (frequency) per symbox
    pub freq    : [usize; ALPHABET_SIZE+1],
}

impl Radix  {
    /// create Radix sort instance
    pub fn new() -> Radix   {
        Radix   {
            freq : [0; ALPHABET_SIZE+1],
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
            self.freq[b as usize] += 1;
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
    pub fn place(&mut self, b: Symbol)-> usize   {
        let pos = self.freq[b as usize];
        assert!(self.freq[b as usize] < self.freq[(b as usize)+1],
            "Unable to place symbol {} at offset {}",
            b, pos);
        self.freq[b as usize] += 1;
        pos
    }

    /// shift frequences to the left
    /// allows the offsets to be re-used after all positions are obtained
    pub fn shift(&mut self) {
        assert_eq!( self.freq[ALPHABET_SIZE-1], self.freq[ALPHABET_SIZE] );
        for i in (0 .. ALPHABET_SIZE).rev()   {
            self.freq[i+1] = self.freq[i];
        }
        self.freq[0] = 0;
    }
}


/// Compute a suffix array from a given input string
/// Resulting suffixes are guaranteed to be alphabetically sorted
/// Run time: O(N^3), memory: N words (suf_array) + ALPHABET_SIZE words (Radix)
pub fn compute_suffixes<SUF: NumCast + ToPrimitive + fmt::Debug>(input: &[Symbol], suf_array: &mut [SUF]) {
    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    debug!("SA compute input: {:?}", input);
    debug!("radix offsets: {:?}", &radix.freq[..]);

    for (i,&ch) in input.iter().enumerate() {
        let p = radix.place(ch);
        suf_array[p] = NumCast::from(i).unwrap();
    }

    // bring the original offsets back
    radix.shift();

    for i in 0..ALPHABET_SIZE {
        let lo = radix.freq[i];
        let hi = radix.freq[i+1];
        if lo == hi {
            continue;
        }
        let slice = &mut suf_array[lo..hi];
        debug!("\tsorting group [{}-{}) for symbol {}", lo, hi, i);
        slice.sort_by(|a,b| {
            input[(a.to_usize().unwrap())..].cmp(&input[(b.to_usize().unwrap())..])
        });
    }

    debug!("sorted SA: {:?}", suf_array);
}

/// An iterator over BWT output
pub struct TransformIterator<'a, SUF: 'a> {
    input      : &'a [Symbol],
    suf_iter   : iter::Enumerate<slice::Iter<'a,SUF>>,
    origin     : Option<usize>,
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
    pub fn get_origin(&self) -> usize {
        self.origin.unwrap()
    }
}

impl<'a, SUF: ToPrimitive + 'a> Iterator for TransformIterator<'a, SUF> {
    type Item = Symbol;
    fn next(&mut self) -> Option<Symbol> {
        self.suf_iter.next().map(|(i,p)| {
            if p.to_usize().unwrap() == 0 {
                assert!( self.origin.is_none() );
                self.origin = Some(i);
                *self.input.last().unwrap()
            }else {
                self.input[p.to_usize().unwrap() - 1]
            }
        })
    }
}

/// Encode BWT of a given input, using the 'suf_array'
pub fn encode<'a, SUF: NumCast + ToPrimitive + fmt::Debug>(input: &'a [Symbol], suf_array: &'a mut [SUF]) -> TransformIterator<'a, SUF> {
    compute_suffixes(input, suf_array);
    TransformIterator::new(input, suf_array)
}

/// Transform an input block into the output slice, all-inclusive version.
/// Returns the index of the original string in the output matrix.
pub fn encode_simple(input: &[Symbol]) -> (Vec<Symbol>, usize) {
    let mut suf_array: Vec<usize> = repeat(0).take(input.len()).collect();
    let mut iter = encode(input, &mut suf_array[..]);
    let output: Vec<Symbol> = iter.by_ref().collect();
    (output, iter.get_origin())
}


/// Compute an inversion jump table, needed for BWT decoding
pub fn compute_inversion_table<SUF: NumCast + fmt::Debug>(input: &[Symbol], origin: usize, table: &mut [SUF]) {
    assert_eq!(input.len(), table.len());

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    table[radix.place(input[origin])] = NumCast::from(0).unwrap();
    for (i,&ch) in input[..origin].iter().enumerate() {
        table[radix.place(ch)] = NumCast::from(i+1).unwrap();
    }
    for (i,&ch) in input[(origin+1)..].iter().enumerate() {
        table[radix.place(ch)] = NumCast::from(origin+2+i).unwrap();
    }
    //table[-1] = origin;
    debug!("inverse table: {:?}", table)
}

/// An iterator over inverse BWT
/// Run time: O(N), memory: N words (table)
pub struct InverseIterator<'a, SUF: 'a> {
    input      : &'a [Symbol],
    table      : &'a [SUF],
    origin     : usize,
    current    : usize,
}

impl<'a, SUF> InverseIterator<'a, SUF> {
    /// create a new inverse BWT iterator with a given input, origin, and a jump table
    pub fn new(input: &'a [Symbol], origin: usize, table: &'a [SUF]) -> InverseIterator<'a, SUF> {
        debug!("inverse origin={:?}, input: {:?}", origin, input);
        InverseIterator {
            input: input,
            table: table,
            origin: origin,
            current: origin,
        }
    }
}

impl<'a, SUF: ToPrimitive> Iterator for InverseIterator<'a, SUF> {
    type Item = Symbol;

    fn next(&mut self) -> Option<Symbol> {
        if self.current == usize::max_value() {
            None
        } else {
            self.current = self.table[self.current].to_usize().unwrap().wrapping_sub(1);
            debug!("\tjumped to {}", self.current);

            let p = if self.current != usize::max_value() {
                self.current
            } else {
                self.origin
            };

            Some(self.input[p])
        }   
    }
}

/// Decode a BWT block, given it's origin, and using 'table' temporarily
pub fn decode<'a, SUF: NumCast + fmt::Debug>(input: &'a [Symbol], origin: usize, table: &'a mut [SUF]) -> InverseIterator<'a, SUF> {
    compute_inversion_table(input, origin, table);
    InverseIterator::new(input, origin, table)
}

/// A simplified BWT decode function, which allocates a temporary suffix array
pub fn decode_simple(input: &[Symbol], origin: usize) -> Vec<Symbol> {
    let mut suf: Vec<usize> = repeat(0).take(input.len()).collect();
    decode(input, origin, &mut suf[..]).take(input.len()).collect()
}

/// Decode without additional memory, can be greatly optimized
/// Run time: O(n^2), Memory: 0n
fn decode_minimal(input: &[Symbol], origin: usize, output: &mut [Symbol]) {
    assert_eq!(input.len(), output.len());
    if input.len() == 0 {
        assert_eq!(origin, 0);
    }

    let mut radix = Radix::new();
    radix.gather(input);
    radix.accumulate();

    let n = input.len();
    (0..n).fold(origin, |i,j| {
        let ch = input[i];
        output[n-j-1] = ch;
        let offset = &input[..i].iter().filter(|&k| *k==ch).count();
        radix.freq[ch as usize] + offset
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
    start  : usize,

    temp   : Vec<u8>,
    output : Vec<u8>,
    table  : Vec<usize>,

    header         : bool,
    max_block_size : usize,
    extra_memory   : bool,
}

impl<R: Read> Decoder<R> {
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

    fn read_header(&mut self) -> io::Result<()> {
        match self.r.read_u32::<LittleEndian>() {
            Ok(size) => {
                self.max_block_size = size as usize;
                debug!("max size: {}", self.max_block_size);
                Ok(())
            },
            Err(e) => Err(byteorder_err_to_io(e)),
        }
    }

    fn decode_block(&mut self) -> io::Result<bool> {
        let n = match self.r.read_u32::<LittleEndian>() {
            Ok(n) => n as usize,
            Err(byteorder::Error::Io(e)) => return Err(e),
            Err(..) => return Ok(false) // EOF
        };

        self.temp.truncate(0);
        self.temp.reserve(n);
        try!(self.r.push_exactly(n as u64, &mut self.temp));

        let origin = try!(self.r.read_u32::<LittleEndian>()) as usize;
        self.output.truncate(0);
        self.output.reserve(n);

        if self.extra_memory    {
            self.table.truncate(0);
            self.table.extend((0..n).map(|_| 0));
            for ch in decode(&self.temp[..], origin, &mut self.table[..]) {
                self.output.push(ch);
            }
        }else   {
            self.output.extend((0..n).map(|_| 0));
            decode_minimal(&self.temp[..], origin, &mut self.output[..]);
        }

        self.start = 0;
        return Ok(true);
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::Result<usize> {
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
            unsafe { intrinsics::copy_nonoverlapping(
                &self.output[self.start],
                &mut dst[dst_len - amt],
                n,
            )};
            self.start += n;
            amt -= n;
        }

        Ok(dst_len - amt)
    }
}


/// This structure is used to compress a stream of bytes using the BWT.
/// This is a wrapper around an internal writer which bytes will be written to.
pub struct Encoder<W> {
    w: W,
    buf: Vec<u8>,
    suf: Vec<usize>,
    wrote_header: bool,
    block_size: usize,
}

impl<W: Write> Encoder<W> {
    /// Creates a new encoder which will have its output written to the given
    /// output stream. The output stream can be re-acquired by calling
    /// `finish()`
    /// 'block_size' is idealy as big as your input, unless you know for sure that
    /// the input consists of multiple parts of different nature. Often set as 4Mb.
    pub fn new(w: W, block_size: usize) -> Encoder<W> {
        Encoder {
            w: w,
            buf: Vec::new(),
            suf: Vec::new(),
            wrote_header: false,
            block_size: block_size,
        }
    }

    fn encode_block(&mut self) -> io::Result<()> {
        let n = self.buf.len();
        try!(self.w.write_u32::<LittleEndian>(n as u32));

        self.suf.truncate(0);
        self.suf.extend((0..n).map(|_| n));
        let w = &mut self.w;

        {
            let mut iter = encode(&self.buf[..], &mut self.suf[..]);
            for ch in iter.by_ref() {
                try!(w.write_u8(ch));
            }

            try!(w.write_u32::<LittleEndian>(iter.get_origin() as u32));
        }
        self.buf.truncate(0);

        Ok(())
    }

    /// This function is used to flag that this session of compression is done
    /// with. The stream is finished up (final bytes are written), and then the
    /// wrapped writer is returned.
    pub fn finish(mut self) -> (W, io::Result<()>) {
        let result = self.flush();
        (self.w, result)
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, mut buf: &[u8]) -> io::Result<usize> {
        if !self.wrote_header {
            try!(self.w.write_u32::<LittleEndian>(self.block_size as u32));
            self.wrote_header = true;
        }

        while buf.len() > 0 {
            let amt = cmp::min( self.block_size - self.buf.len(), buf.len() );
            self.buf.extend(buf[..amt].iter().map(|b| *b));

            if self.buf.len() == self.block_size {
                try!(self.encode_block());
            }
            buf = &buf[amt..];
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
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
    use std::io::{BufReader, BufWriter, Read, Write};
    #[cfg(feature="unstable")]
    use test::Bencher;
    use super::{Decoder, Encoder};

    fn roundtrip(bytes: &[u8], extra_mem: bool) {
        let mut e = Encoder::new(BufWriter::new(Vec::new()), 1<<10);
        e.write(bytes).unwrap();
        let (e, err) = e.finish();
        err.unwrap();
        let encoded = e.into_inner().unwrap();

        let mut d = Decoder::new(BufReader::new(&encoded[..]), extra_mem);
        let mut decoded = Vec::new();
        d.read_to_end(&mut decoded).unwrap();
        assert_eq!(&decoded[..], bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(b"test", true);
        roundtrip(b"", true);
        roundtrip(include_bytes!("../data/test.txt"), true);
    }

    #[test]
    fn decode_minimal() {
        roundtrip(b"abracadabra", false);
    }

    #[cfg(feature="unstable")]
    #[bench]
    fn decode_speed(bh: &mut Bencher) {
        use std::iter::repeat;
        use super::{encode, decode};

        let input = include_bytes!("../data/test.txt");
        let n = input.len();
        let mut suf: Vec<u16> = repeat(0).take(n).collect();
        let (output, origin) = {
            let mut to_iter = encode(input, &mut suf[..]);
            let out: Vec<u8> = to_iter.by_ref().collect();
            (out, to_iter.get_origin())
        };

        bh.iter(|| {
            let from_iter = decode(&output[..], origin, &mut suf[..]);
            from_iter.last().unwrap();
        });
        bh.bytes = n as u64;
    }
}
