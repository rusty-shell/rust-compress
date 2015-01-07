/*!

Arithmetic encoder/decoder using the Range encoder underneath. Requires `entropy` feature, enabled by default
Can be used in a general case of entropy coding stage. Supposed to be fast.

# Links

http://en.wikipedia.org/wiki/Arithmetic_coding
http://en.wikipedia.org/wiki/Range_encoding

# Example
```rust
# #[allow(unused_must_use)];
use std::io::{MemWriter, MemReader};
use compress::entropy::ari;

// Encode some text
let text = "some text";
let mut e = ari::ByteEncoder::new(MemWriter::new());
e.write_str(text);
let (encoded, _) = e.finish();

// Decode the encoded text
let mut d = ari::ByteDecoder::new(MemReader::new(encoded.unwrap()));
let decoded = d.read_to_end().unwrap();
```
# Credit

This is an original implementation.

*/

#![allow(missing_docs)]

use std::fmt::Show;
use std::io::IoResult;

pub use self::table::{ByteDecoder, ByteEncoder};

pub mod apm;
pub mod bin;
pub mod table;
#[cfg(test)]
mod test;

pub type Symbol = u8;
const SYMBOL_BITS: uint = 8;
const SYMBOL_TOTAL: uint = 1<<SYMBOL_BITS;

pub type Border = u32;
const BORDER_BYTES: uint = 4;
const BORDER_BITS: uint = BORDER_BYTES * 8;
const BORDER_EXCESS: uint = BORDER_BITS-SYMBOL_BITS;
const BORDER_SYMBOL_MASK: u32 = ((SYMBOL_TOTAL-1) << BORDER_EXCESS) as u32;

pub const RANGE_DEFAULT_THRESHOLD: Border = 1<<14;


/// Range Encoder basic primitive
/// Gets probability ranges on the input, produces whole bytes of code on the output,
/// where the code is an arbitrary fixed-ppoint value inside the resulting probability range.
pub struct RangeEncoder {
    low: Border,
    hai: Border,
    /// The minimum distance between low and hai to keep at all times,
    /// has to be at least the largest incoming 'total',
    /// and optimally many times larger
    pub threshold: Border,
    /// Tuning parameters
    bits_lost_on_threshold_cut: f32,
    bits_lost_on_division: f32,
}

impl RangeEncoder {
    /// Create a new instance
    /// will keep the active range below 'max_range'
    pub fn new(max_range: Border) -> RangeEncoder {
        debug_assert!(max_range > (SYMBOL_TOTAL as Border));
        RangeEncoder {
            low: 0,
            hai: -1,
            threshold: max_range,
            bits_lost_on_threshold_cut: 0.0,
            bits_lost_on_division: 0.0,
        }
    }

    /// Reset the current range
    pub fn reset(&mut self) {
        self.low = 0;
        self.hai = -1;
    }

    #[cfg(tune)]
    fn count_bits(range: Border, total: Border) -> f32 {
        -((range as f32) / (total as f32)).log2()
    }

    #[cfg(not(tune))]
    fn count_bits(_range: Border, _total: Border) -> f32 {
        0.0
    }

    /// Return the number of bits lost due to threshold cuts and integer operations
    #[cfg(tune)]
    pub fn get_bits_lost(&self) -> (f32, f32) {
        (self.bits_lost_on_threshold_cut, self.bits_lost_on_division)
    }

    /// Process a given interval [from/total,to/total) into the current range
    /// write into the output slice, and return the number of symbols produced
    pub fn process(&mut self, total: Border, from: Border, to: Border, output: &mut [Symbol]) -> uint {
        debug_assert!(from<to && to<=total);
        let old_range = self.hai - self.low;
        let range = old_range / total;
        debug_assert!(range>0, "RangeCoder range is too narrow [{}-{}) for the total {}",
            self.low, self.hai, total);
        debug!("\t\tProcessing [{}-{})/{} with range {}", from, to, total, range);
        let mut lo = self.low + range*from;
        let mut hi = self.low + range*to;
        self.bits_lost_on_division += RangeEncoder::count_bits(range*total, old_range);
        let mut num_shift = 0u;
        loop {
            if (lo^hi) & BORDER_SYMBOL_MASK != 0 {
                if hi-lo > self.threshold {
                    break
                }
                let old_range = hi-lo;
                let lim = hi & BORDER_SYMBOL_MASK;
                if hi-lim >= lim-lo {lo=lim}
                else {hi=lim-1};
                debug_assert!(lo < hi);
                self.bits_lost_on_threshold_cut += RangeEncoder::count_bits(hi-lo, old_range);
            }

            debug!("\t\tShifting on [{}-{}) to symbol {}", lo, hi, lo>>BORDER_EXCESS);
            output[num_shift] = (lo>>BORDER_EXCESS) as Symbol;
            num_shift += 1;
            lo<<=SYMBOL_BITS; hi<<=SYMBOL_BITS;
            debug_assert!(lo < hi);
        }
        self.low = lo;
        self.hai = hi;
        num_shift
    }

    /// Query the value encoded by 'code' in range [0,total)
    pub fn query(&self, total: Border, code: Border) -> Border {
        debug!("\t\tQuerying code {} of total {} under range [{}-{})",
            code, total, self.low, self.hai);
        debug_assert!(self.low <= code && code < self.hai);
        let range = (self.hai - self.low) / total;
        (code - self.low) / range
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


/// An abstract model to produce probability ranges
/// Can be a table, a mix of tables, or just a smart function.
pub trait Model<V: Copy + Show> {
    /// Get the probability range of a value
    fn get_range(&self, value: V) -> (Border,Border);
    /// Find the value by a given probability offset, return with the range
    fn find_value(&self, offset: Border) -> (V,Border,Border);
    /// Get the sum of all probabilities
    fn get_denominator(&self) -> Border;

    /// Encode a value using a range encoder
    /// return the number of symbols written
    fn encode(&self, value: V, re: &mut RangeEncoder, out: &mut [Symbol]) -> uint {
        let (lo, hi) = self.get_range(value);
        let total = self.get_denominator();
        debug!("\tEncoding value {:?} of range [{}-{}) with total {}", value, lo, hi, total);
        re.process(total, lo, hi, out)
    }

    /// Decode a value using given 'code' on the range encoder
    /// return a (value, num_symbols_to_shift) pair
    fn decode(&self, code: Border, re: &mut RangeEncoder) -> (V, uint) {
        let total = self.get_denominator();
        let offset = re.query(total, code);
        let (value, lo, hi) = self.find_value(offset);
        debug!("\tDecoding value {:?} of offset {} with total {}", value, offset, total);
        let mut out = [0 as Symbol; BORDER_BYTES];
        let shift = re.process(total, lo, hi, out.as_mut_slice());
        debug_assert_eq!(if shift==0 {0} else {code>>(BORDER_BITS - shift*8)},
            out.slice_to(shift).iter().fold(0 as Border, |u,&b| (u<<8)+(b as Border)));
        (value, shift)
    }
}


/// An arithmetic encoder helper
pub struct Encoder<W> {
    stream: W,
    range: RangeEncoder,
}

impl<W: Writer> Encoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            stream: w,
            range: RangeEncoder::new(RANGE_DEFAULT_THRESHOLD),
        }
    }

    /// Encode an abstract value under the given Model
    pub fn encode<V: Copy + Show, M: Model<V>>(&mut self, value: V, model: &M) -> IoResult<()> {
        let mut buf = [0 as Symbol; BORDER_BYTES];
        let num = model.encode(value, &mut self.range, buf.as_mut_slice());
        self.stream.write(buf.slice_to(num))
    }

    /// Finish encoding by writing the code tail word
    pub fn finish(mut self) -> (W, IoResult<()>) {
        debug_assert!(BORDER_BITS == 32);
        let code = self.range.get_code_tail();
        let result = self.stream.write_be_u32(code);
        let result = result.and(self.stream.flush());
        (self.stream, result)
    }

    /// Flush the output stream
    pub fn flush(&mut self) -> IoResult<()> {
        self.stream.flush()
    }

    /// Return the number of bytes lost due to threshold cuts and integer operations
    #[cfg(tune)]
    pub fn get_bytes_lost(&self) -> (f32, f32) {
        let (a,b) = self.range.get_bits_lost();
        (a/8.0, b/8.0)
    }
}

/// An arithmetic decoder helper
pub struct Decoder<R> {
    stream: R,
    range: RangeEncoder,
    code: Border,
    bytes_pending: uint,
}

impl<R: Reader> Decoder<R> {
    /// Create a decoder on top of a given Reader
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            stream: r,
            range: RangeEncoder::new(RANGE_DEFAULT_THRESHOLD),
            code: 0,
            bytes_pending: BORDER_BYTES,
        }
    }

    fn feed(&mut self) -> IoResult<()> {
        while self.bytes_pending != 0 {
            let b = try!(self.stream.read_u8());
            self.code = (self.code<<8) + (b as Border);
            self.bytes_pending -= 1;
        }
        Ok(())
    }

    /// Decode an abstract value based on the given Model
    pub fn decode<V: Copy + Show, M: Model<V>>(&mut self, model: &M) -> IoResult<V> {
        self.feed().unwrap();
        let (value, shift) = model.decode(self.code, &mut self.range);
        self.bytes_pending = shift;
        Ok(value)
    }

    /// Finish decoding
    pub fn finish(mut self) -> (R, IoResult<()>)  {
        let err = self.feed();
        (self.stream, err)
    }
}
