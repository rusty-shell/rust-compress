/*!

Arithmetic encoder/decoder using the Range Coder (RC) underneath.
Can be used in a general case of entropy coding stage. Supposed to be fast.

# Links

http://en.wikipedia.org/wiki/Arithmetic_coding
http://en.wikipedia.org/wiki/Range_encoding

# Example

```rust
let mut e = Encoder::new( MemWriter::new() );
e.write(bytes);
let encoded = e.finish().unwrap();
let mut d = Decoder::new( BufReader::new(encoded), bytes.len() );
let decoded = d.read_to_end();

```

# Credit

This is an original implementation.

*/

use std::{num, io, vec};

pub type Symbol = u8;
static symbol_bits: uint = 8;
static symbol_total: uint = 1<<symbol_bits;

pub type Border = u32;
static border_bits: uint = 32;
static border_excess: uint = border_bits-symbol_bits;
static border_symbol_mask: u32 = ((symbol_total-1) << border_excess) as u32;

/// Range Coder basic primitive
/// used for generic encoding/decoding of probabilities expressed as intervals
pub struct RC {
    // TODO: introduce a range struct
    priv low: Border,
    priv hai: Border,
    priv threshold: Border,
}

impl RC {
    /// Create a new instance
    /// will keep the active range below 'max_range'
    /// A typical value is 16k
    pub fn new(max_range: Border) -> RC {
        assert!(max_range > (symbol_total as Border));
        RC { low:0, hai:-1, threshold: max_range }
    }

    /// Process a given interval [from/total,to/total) into the current range
    /// Yields stabilized code symbols (bytes) into the 'fn_shift' function
    pub fn process(&mut self, total: Border, from: Border, to: Border, fn_shift: |Symbol|) {
        let range = (self.hai - self.low) / total;
        debug!("\t\tProcessing [{}-{})/{} with range {}", from, to, total, range);
        assert!(from < to);
        let mut lo = self.low + range*from;
        let mut hi = self.low + range*to;
        while hi < lo+self.threshold {
            if ((lo^hi) & border_symbol_mask) != 0 {
                let lim = hi & border_symbol_mask;
                if hi-lim >= lim-lo {lo=lim}
                else {hi=lim-1};
                assert!(lo < hi);
            }
            while ((lo^hi) & border_symbol_mask) == 0 {
                debug!("\t\tShifting on [{}-{}) to symbol {}", lo, hi, lo>>border_excess);
                fn_shift((lo>>border_excess) as Symbol);
                lo<<=symbol_bits; hi<<=symbol_bits;
                assert!(lo < hi);
            }
        }
        self.low = lo;
        self.hai = hi;
    }

    /// Query the value encoded by 'code' in range [0,total)
    pub fn query(&self, total: Border, code: Border) -> Border {
        debug!("\t\tQuerying code {} of total {} under range [{}-{})",
            code, total, self.low, self.hai);
        assert!(self.low <= code && code < self.hai)
        let range = (self.hai - self.low) / total;
        (code - self.low) / range
        //TODO: use better mul-div logic
    }

    /// Get the code tail and close the range
    /// used at the end of encoding
    pub fn get_code_tail(&mut self) -> Border {
        let tail = self.low;
        self.low = 0;
        self.hai = 0;
        tail
    }
}


pub type Frequency = u16;

/// Arithmetic encoder/decoder class
pub struct Ari {
    /// size of the encoding alphabet
    priv total: Frequency,
    /// value frequencies
    priv frequencies: ~[Frequency],
    /// a logarithm base 2 used for the frequency cut
    priv cut_log: uint,
    /// threshold value to trigget the frequency cut
    priv freq_threshold: Frequency,
}

impl Ari {
    /// Create a new Ari instance with frequencies initialized by a function
    pub fn new_custom(num_values: uint, fn_init: |uint|-> Frequency) -> Ari {
        let freq = vec::from_fn(num_values, fn_init);
        Ari {
            total: freq.iter().fold(0, |u,&f| u+f),
            frequencies: freq,
            cut_log: 1,
            freq_threshold: 1<<12,
        }
    }

    /// Create a new Ari instance with all frequencies being equal
    pub fn new_flat(num_values: uint) -> Ari {
        Ari::new_custom(num_values, |_| 1)
    }

    /// Encode 'value', using the range coder
    /// returns a list of output bytes
    pub fn encode(&mut self, value: uint, rc: &mut RC) -> ~[Symbol] {
        let lo = self.frequencies.slice_to(value).iter().fold(0, |u,&f| u+f);
        let hi = lo + self.frequencies[value];
        let mut accum: ~[Symbol] = ~[];
        debug!("\tEncoding value {} of range [{}-{}) with total {}", value, lo, hi, self.total);
        rc.process(self.total as Border, lo as Border, hi as Border, |byte| accum.push(byte));
        accum
    }

    /// Decode a value using given 'code' on the range coder
    /// Returns a (value, num_symbols_to_shift) pair
    pub fn decode(&mut self, code: Border, rc: &mut RC) -> (uint,uint) {
        let total = self.total as Border;
        let freq = rc.query(total, code) as Frequency;
        assert!(freq < self.total,
            "Invalid frequency offset {} requested for code {} under total {}",
            freq, code, total);
        let mut value = 0u;
        let mut lo = 0 as Frequency;
        let mut hi;
        while {hi=lo+self.frequencies[value]; hi} <= freq {
            lo = hi;
            value += 1;
        }
        debug!("\tDecoding value {} of offset {} with total {}", value, freq, total);
        let mut shift_bytes = 0u;
        rc.process(total, lo as Border, hi as Border, |_| shift_bytes+=1);
        (value,shift_bytes)
    }

    /// Adapt the frequency table in favor of given 'value'
    /// using 'add_log' and 'add_const' to produce the additive factor
    /// the higher 'add_log' is, the more concervative is the adaptation
    pub fn update(&mut self, value: uint, add_log: uint, add_const: Frequency) {
        let add = (self.total>>add_log) + add_const;
        assert!(add < self.freq_threshold);
        self.frequencies[value] += add;
        self.total += add;
        debug!("\tUpdating by adding {} to value {}", add, value);
        if self.total >= self.freq_threshold {
            debug!("\tDownscaling frequencies");
            self.total = 0;
            let roundup = (1<<self.cut_log) - 1;
            for freq in self.frequencies.mut_iter() {
                // preserve non-zero frequencies to remain positive
                *freq = (*freq+roundup) >> self.cut_log;
                self.total += *freq;
            }
        }
    }
}


/// Arithmetic Decoder
//NOTE: decoder currently needs to know the output size. This can be worked around
// by writing the size to the beginning of the stream. However, since Ari is
// typically used in conjunction with the higher-level compression model, the size
// can be known in advance.
pub struct Decoder<R> {
    /// The internally wrapped reader. This is exposed so it may be moved out
    /// of. Note that if data is read from the reader while decoding is in
    /// progress the output stream will get corrupted.
    r: R,
    priv output_left: uint,
    priv rc: RC,
    priv ari: Ari,
    priv code: Border,
    priv bytes_read: uint,
}

impl<R: Reader> Decoder<R> {
    /// Create a decoder on top of a given Reader
    /// requires the output size to be known
    pub fn new(r: R, out_size: uint) -> Decoder<R> {
        Decoder {
            r: r,
            output_left: out_size,
            rc: RC::new(1<<14),
            ari: Ari::new_flat(symbol_total),
            code: 0,
            bytes_read: 0,
        }
    }

    /// Start decoding by reading a full code word
    fn start(&mut self) -> io::IoResult<()> {
        assert!(border_bits == 32);
        self.code = if_ok!(self.r.read_be_u32());
        self.bytes_read += 4;
        Ok(())
    }
}

impl<R: Reader> Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        if self.output_left == 0 {
            return Err(io::standard_error(io::EndOfFile))
        }
        if self.bytes_read == 0 {
            if_ok!(self.start());
        }
        let write_len = num::min(dst.len(), self.output_left);
        for out_byte in dst.mut_slice_to(write_len).mut_iter() {
            let (byte,shift) = self.ari.decode(self.code, &mut self.rc);
            self.ari.update(byte, 10, 1);
            *out_byte = byte as u8;
            for _ in range(0,shift) {
                let byte = if_ok!(self.r.read_u8()) as Border;
                self.bytes_read += 1;
                self.code = (self.code<<8) + byte;
            }
        }
        self.output_left -= write_len;
        Ok(write_len)
    }
}

/// Arithmetic Encoder
pub struct Encoder<W> {
    priv w: W,
    priv rc: RC,
    priv ari: Ari,
}

impl<W: Writer> Encoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            w: w,
            rc: RC::new(1<<14),
            ari: Ari::new_flat(symbol_total),
        }
    }

    /// Finish decoding by writing the code tail word
    pub fn finish(mut self) -> (W, io::IoResult<()>) {
        assert!(border_bits == 32);
        let code = self.rc.get_code_tail();
        let result = self.w.write_be_u32(code);
        let result = result.and(self.w.flush());
        (self.w, result)
    }
}

impl<W: Writer> Writer for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::IoResult<()> {
        for byte in buf.iter() {
            let value = *byte as uint;
            let bytes = self.ari.encode(value, &mut self.rc);
            self.ari.update(value, 10, 1);
            if_ok!(self.w.write(bytes.as_slice()));
        }
        Ok(())
    }
}


#[cfg(test)]
mod test {
    //use extra::test;
    use std::io::{BufReader, MemWriter};
    use super::{Encoder,Decoder};

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip Ari of size {}", bytes.len());
        let mut e = Encoder::new( MemWriter::new() );
        e.write(bytes).unwrap();
        let (e, r) = e.finish();
        r.unwrap();
        let encoded = e.unwrap();
        debug!("Roundtrip input {:?} encoded {:?}", bytes, encoded);
        let mut d = Decoder::new( BufReader::new(encoded), bytes.len() );
        let decoded = d.read_to_end().unwrap();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(bytes!("abracadabra"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("../data/test.txt"));
    }
}
