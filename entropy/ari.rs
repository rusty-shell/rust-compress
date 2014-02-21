/*!

Arithmetic encoder/decoder using the Range encoder underneath.
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

use std::{io, vec};

pub type Symbol = u8;
static symbol_bits: uint = 8;
static symbol_total: uint = 1<<symbol_bits;

pub type Border = u32;
static border_bits: uint = 32;
static border_excess: uint = border_bits-symbol_bits;
static border_symbol_mask: u32 = ((symbol_total-1) << border_excess) as u32;

/// Range Encoder basic primitive
/// Gets probability ranges on the input, produces whole bytes of code on the output,
/// where the code is an arbitrary fixed-ppoint value inside the resulting probability range.
pub struct RangeEncoder {
    priv low: Border,
    priv hai: Border,
    /// The minimum distance between low and hai to keep at all times,
    /// has to be at least the largest incoming 'total',
    /// and optimally many times larger
    threshold: Border,
}

impl RangeEncoder {
    /// Create a new instance
    /// will keep the active range below 'max_range'
    /// A typical value is 16k
    pub fn new(max_range: Border) -> RangeEncoder {
        assert!(max_range > (symbol_total as Border));
        RangeEncoder { low:0, hai:-1, threshold: max_range }
    }

    /// Reset the current range
    pub fn reset(&mut self) {
        self.low = 0;
        self.hai = -1;
    }

    /// Process a given interval [from/total,to/total) into the current range
    /// Yields stabilized code symbols (bytes) into the 'fn_shift' function
    pub fn process(&mut self, total: Border, from: Border, to: Border, fn_shift: |Symbol|) {
        let range = (self.hai - self.low) / total;
        assert!(range>0, "RangeCoder range is too narrow [{}-{}) for the total {}",
            self.low, self.hai, total);
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
        //TODO: use better mul-div logic, when LLVM allows
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


pub type Value = uint;

/// An abstract model to produce probability ranges
/// Can be a table, a mix of tables, or just a smart function.
pub trait Model {
    /// get the probability range of a value
    fn get_range(&self, value: Value) -> (Border,Border);
    /// find the value by a given probability offset, return with the range
    fn find_value(&self, offset: Border) -> (Value,Border,Border);
    /// sum of all probabilities
    fn get_denominator(&self) -> Border;
}


/// Arithmetic coding functions
pub static range_default_threshold: Border = 1<<14;

/// Encode 'value', using a model and a range encoder
/// returns a list of output bytes
pub fn encode<M: Model>(value: Value, model: &M, re: &mut RangeEncoder) -> ~[Symbol] {
    let (lo, hi) = model.get_range(value);
    let mut accum: ~[Symbol] = ~[];
    let total = model.get_denominator();
    debug!("\tEncoding value {} of range [{}-{}) with total {}", value, lo, hi, total);
    re.process(total, lo, hi, |s| accum.push(s));
    accum
}

/// Decode a value using given 'code' on the range encoder
/// Returns a (value, num_symbols_to_shift) pair
pub fn decode<M: Model>(code: Border, model: &M, re: &mut RangeEncoder) -> (Value,uint) {
    let total = model.get_denominator();
    let offset = re.query(total, code);
    let (value, lo, hi) = model.find_value(offset);
    debug!("\tDecoding value {} of offset {} with total {}", value, offset, total);
    let mut shift_bytes = 0u;
    re.process(total, lo, hi, |_| shift_bytes+=1);
    (value,shift_bytes)
}


/// An arithmetic encoder helper
pub struct Encoder<W> {
    priv stream: W,
    priv range: RangeEncoder,
    priv bytes_written: uint,
}

impl<W: Writer> Encoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            stream: w,
            range: RangeEncoder::new(range_default_threshold),
            bytes_written: 0,
        }
    }

    /// Encode an abstract value under the given 'model'
    pub fn encode<M: Model>(&mut self, value: Value, model: &M) -> io::IoResult<()> {
        let bytes = encode(value, model, &mut self.range);
        self.bytes_written += bytes.len();
        self.stream.write(bytes)
    }

    /// Finish decoding by writing the code tail word
    pub fn finish(mut self) -> (W, io::IoResult<()>) {
        assert!(border_bits == 32);
        self.bytes_written += 4;
        let code = self.range.get_code_tail();
        let result = self.stream.write_be_u32(code);
        let result = result.and(self.stream.flush());
        (self.stream, result)
    }

    /// Flush the output stream
    pub fn flush(&mut self) -> io::IoResult<()> {
        self.stream.flush()
    }

    /// Tell the number of bytes written so far
    pub fn tell(&self) -> uint {
        self.bytes_written
    }
}

/// An arithmetic decoder helper
pub struct Decoder<R> {
    priv stream: R,
    priv range: RangeEncoder,
    priv code: Border,
    priv bytes_read: uint,
}

impl<R: Reader> Decoder<R> {
    /// Create a decoder on top of a given Reader
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            stream: r,
            range: RangeEncoder::new(range_default_threshold),
            code: 0,
            bytes_read: 0,
        }
    }

    /// Start decoding by reading a full code word
    pub fn start(&mut self) -> io::IoResult<()> {
        assert!(border_bits == 32);
        self.bytes_read += 4;
        self.stream.read_be_u32().map(|code| {self.code=code})
    }

    /// Decode an abstract value based on the given model
    pub fn decode<M: Model>(&mut self, model: &M) -> io::IoResult<Value> {
        let (value,shift) = decode(self.code, model, &mut self.range);
        self.bytes_read += shift;
        for b in self.stream.bytes().take(shift) {
            self.code = (self.code<<8) + (b as Border);
        }
        Ok(value)
    }

    /// Release the original reader
    pub fn finish(self) -> R {
        self.stream
    }

    /// Tell the number of bytes read so far
    pub fn tell(&self) -> uint {
        self.bytes_read
    }
}


/// A binary value frequency model
pub struct BinaryModel {
    /// frequency of bit 0
    priv zero: Border,
    /// total frequency
    priv total: Border,
    /// maximum allowed sum of frequency,
    /// should be smaller than RangeEncoder::threshold
    priv cut_threshold: Border,
    /// number of bits to shift on cut
    priv cut_shift: uint,
}

impl BinaryModel {
    /// Create a new flat (50/50 probability) instance
    pub fn new_flat(threshold: Border) -> BinaryModel {
        assert!(threshold > 2);
        BinaryModel {
            zero: 1,
            total: 2,
            cut_threshold: threshold,
            cut_shift: 2
        }
    }
    /// Create a new instance with a given percentage for zeroes
    pub fn new_custom(zero_percent: u8, threshold: Border) -> BinaryModel {
        assert!(threshold > 100);
        BinaryModel {
            zero: zero_percent as Border,
            total: 100,
            cut_threshold: threshold,
            cut_shift: 2,
        }
    }
    /// Update frequencies in favor of given 'value'
    pub fn update(&mut self, value: Value, add_log: uint, add_const: Border) {
        assert!(value < 2);
        let add = (self.total>>add_log) + add_const;
        debug!("\tUpdating by adding {} to value {}", add, value);
        self.total += add;
        if value==0 {
            self.zero += add;
        }
        if self.total >= self.cut_threshold {
            self.downscale();
        }
    }
    /// Reduce frequencies by 'cut_iter' bits
    pub fn downscale(&mut self) {
        let roundup = (1<<self.cut_shift) - 1;
        self.zero = (self.zero + roundup) >> self.cut_shift;
        self.total = (self.total + roundup) >> self.cut_shift;
    }
}

impl Model for BinaryModel {
    fn get_range(&self, value: Value) -> (Border,Border) {
        if value==0 {
            (0, self.zero)
        }else {
            (self.zero, self.total)
        }
    }

    fn find_value(&self, offset: Border) -> (Value,Border,Border) {
        assert!(offset < self.total,
            "Invalid frequency offset {} requested under total {}",
            offset, self.total);
        if offset < self.zero {
            (0, 0, self.zero)
        }else {
            (1, self.zero, self.total)
        }
    }

    fn get_denominator(&self) -> Border {
        self.total
    }
}


pub type Frequency = u16;

/// A simple table of frequencies.
pub struct FrequencyTable {
    /// sum of frequencies
    priv total: Border,
    /// main table: value -> Frequency
    priv table: ~[Frequency],
    /// maximum allowed sum of frequency,
    /// should be smaller than RangeEncoder::threshold
    priv cut_threshold: Border,
    /// number of bits to shift on cut
    priv cut_shift: uint,
}

impl FrequencyTable {
    /// Create a new table with frequencies initialized by a function
    pub fn new_custom(num_values: uint, threshold: Border, fn_init: |Value|-> Frequency) -> FrequencyTable {
        let freq = vec::from_fn(num_values, fn_init);
        let total = freq.iter().fold(0 as Border, |u,&f| u+(f as Border));
        let mut ft = FrequencyTable {
            total: total,
            table: freq,
            cut_threshold: threshold,
            cut_shift: 1,
        };
        // downscale if needed
        while ft.total >= threshold {
            ft.downscale();
        }
        ft
    }

    /// Create a new tanle with all frequencies being equal
    pub fn new_flat(num_values: uint, threshold: Border) -> FrequencyTable {
        FrequencyTable::new_custom(num_values, threshold, |_| 1)
    }

    /// Reset the table to the flat state
    pub fn reset_flat(&mut self) {
        for freq in self.table.mut_iter() {
            *freq = 1;
        }
        self.total = self.table.len() as Border;
    }

    /// Adapt the table in favor of given 'value'
    /// using 'add_log' and 'add_const' to produce the additive factor
    /// the higher 'add_log' is, the more concervative is the adaptation
    pub fn update(&mut self, value: Value, add_log: uint, add_const: Border) {
        let add = (self.total>>add_log) + add_const;
        assert!(add < 2*self.cut_threshold);
        debug!("\tUpdating by adding {} to value {}", add, value);
        self.table[value] += add as Frequency;
        self.total += add;
        if self.total >= self.cut_threshold {
            self.downscale();
            assert!(self.total < self.cut_threshold);
        }
    }

    /// Reduce frequencies by 'cut_iter' bits
    pub fn downscale(&mut self) {
        debug!("\tDownscaling frequencies");
        let roundup = (1<<self.cut_shift) - 1;
        self.total = 0;
        for freq in self.table.mut_iter() {
            // preserve non-zero frequencies to remain positive
            *freq = (*freq+roundup) >> self.cut_shift;
            self.total += *freq as Border;
        }
    }
}

impl Model for FrequencyTable {
    fn get_range(&self, value: Value) -> (Border,Border) {
        let lo = self.table.slice_to(value).iter().fold(0, |u,&f| u+(f as Border));
        (lo, lo + (self.table[value] as Border))
    }

    fn find_value(&self, offset: Border) -> (Value,Border,Border) {
        assert!(offset < self.total,
            "Invalid frequency offset {} requested under total {}",
            offset, self.total);
        let mut value = 0u;
        let mut lo = 0 as Border;
        let mut hi;
        while {hi=lo+(self.table[value] as Border); hi} <= offset {
            lo = hi;
            value += 1;
        }
        (value, lo, hi)
    }

    fn get_denominator(&self) -> Border {
        self.total
    }
}


/// A basic byte-encoding arithmetic
/// uses a special terminator code to end the stream
pub struct ByteEncoder<W> {
    /// A lower level encoder
    encoder: Encoder<W>,
    /// A basic frequency table
    freq: FrequencyTable,
}

impl<W: Writer> ByteEncoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> ByteEncoder<W> {
        let freq_max = range_default_threshold >> 2;
        ByteEncoder {
            encoder: Encoder::new(w),
            freq: FrequencyTable::new_flat(symbol_total+1, freq_max),
        }
    }

    /// Finish encoding & write the terminator symbol
    pub fn finish(mut self) -> (W, io::IoResult<()>) {
        let ret = self.encoder.encode(symbol_total, &self.freq);
        let (w,r2) = self.encoder.finish();
        (w, ret.and(r2))
    }
}

impl<W: Writer> Writer for ByteEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::IoResult<()> {
        buf.iter().fold(Ok(()), |result,byte| {
            let value = *byte as Value;
            let ret = self.encoder.encode(value, &self.freq);
            self.freq.update(value, 10, 1);
            result.and(ret)
        })
    }

    fn flush(&mut self) -> io::IoResult<()> {
        self.encoder.flush()
    }
}


/// A basic byte-decoding arithmetic
/// expects a special terminator code for the end of the stream
pub struct ByteDecoder<R> {
    /// A lower level decoder
    decoder: Decoder<R>,
    /// A basic frequency table
    freq: FrequencyTable,
    /// Remember if we found the terminator code
    priv is_eof: bool,
}

impl<R: Reader> ByteDecoder<R> {
    /// Create a decoder on top of a given Reader
    pub fn new(r: R) -> ByteDecoder<R> {
        let freq_max = range_default_threshold >> 2;
        ByteDecoder {
            decoder: Decoder::new(r),
            freq: FrequencyTable::new_flat(symbol_total+1, freq_max),
            is_eof: false,
        }
    }
}

impl<R: Reader> Reader for ByteDecoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        if self.decoder.tell() == 0 {
            try!(self.decoder.start());
        }
        if self.is_eof {
            return Err(io::standard_error(io::EndOfFile))
        }
        let mut amount = 0u;
        for out_byte in dst.mut_iter() {
            let value = try!(self.decoder.decode(&self.freq));
            if value == symbol_total {
                self.is_eof = true;
                break
            }
            self.freq.update(value, 10, 1);
            *out_byte = value as u8;
            amount += 1;
        }
        Ok(amount)
    }
}


#[cfg(test)]
mod test {
    use std::io::{BufReader, BufWriter, MemWriter, SeekSet};
    use std::vec;
    use extra::test;
    use super::{ByteEncoder, ByteDecoder};

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip Ari of size {}", bytes.len());
        let mut e = ByteEncoder::new(MemWriter::new());
        e.write(bytes).unwrap();
        let (e, r) = e.finish();
        r.unwrap();
        let encoded = e.unwrap();
        debug!("Roundtrip input {:?} encoded {:?}", bytes, encoded);
        let mut d = ByteDecoder::new(BufReader::new(encoded));
        let decoded = d.read_to_end().unwrap();
        assert_eq!(bytes.as_slice(), decoded.as_slice());
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(bytes!("abracadabra"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("../data/test.txt"));
    }

    #[bench]
    fn compress_speed(bh: &mut test::BenchHarness) {
        let input = include_bin!("../data/test.txt");
        let mut storage = vec::from_elem(input.len(), 0u8);
        bh.iter(|| {
            let mut w = BufWriter::new(storage);
            w.seek(0, SeekSet).unwrap();
            let mut e = ByteEncoder::new(w);
            e.write(input).unwrap();
        });
        bh.bytes = input.len() as u64;
    }
}
