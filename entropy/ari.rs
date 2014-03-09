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
#[cfg(tune)]
use std::num;

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
    // tune parameters
    priv bits_lost_on_threshold_cut: f32,
    priv bits_lost_on_division: f32,
}

impl RangeEncoder {
    /// Create a new instance
    /// will keep the active range below 'max_range'
    /// A typical value is 16k
    pub fn new(max_range: Border) -> RangeEncoder {
        assert!(max_range > (symbol_total as Border));
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
        -num::log2((range as f32) / (total as f32))
    }

    #[cfg(not(tune))]
    fn count_bits(_range: Border, _total: Border) -> f32 {
        0.0
    }

    /// return the number of bits lost due to threshold cuts and integer operations
    #[cfg(tune)]
    pub fn get_bits_lost(&self) -> (f32, f32) {
        (self.bits_lost_on_threshold_cut, self.bits_lost_on_division)
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
        self.bits_lost_on_division += RangeEncoder::count_bits(range*total, self.hai-self.low);
        loop {
            if (lo^hi) & border_symbol_mask != 0 {
                if hi-lo > self.threshold {
                    break
                }
                let old_range = hi-lo;
                let lim = hi & border_symbol_mask;
                if hi-lim >= lim-lo {lo=lim}
                else {hi=lim-1};
                assert!(lo < hi);
                self.bits_lost_on_threshold_cut += RangeEncoder::count_bits(hi-lo, old_range);
            }

            debug!("\t\tShifting on [{}-{}) to symbol {}", lo, hi, lo>>border_excess);
            fn_shift((lo>>border_excess) as Symbol);
            lo<<=symbol_bits; hi<<=symbol_bits;
            assert!(lo < hi);
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
pub fn encode<M: Model>(value: Value, model: &M, re: &mut RangeEncoder, accum: &mut ~[Symbol]) {
    let (lo, hi) = model.get_range(value);
    let total = model.get_denominator();
    debug!("\tEncoding value {} of range [{}-{}) with total {}", value, lo, hi, total);
    re.process(total, lo, hi, |s| accum.push(s));
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
    priv buffer: ~[Symbol],
    priv bytes_written: uint,
}

impl<W: Writer> Encoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            stream: w,
            range: RangeEncoder::new(range_default_threshold),
            buffer: vec::with_capacity(4),
            bytes_written: 0,
        }
    }

    /// Encode an abstract value under the given 'model'
    pub fn encode<M: Model>(&mut self, value: Value, model: &M) -> io::IoResult<()> {
        self.buffer.truncate(0);
        encode(value, model, &mut self.range, &mut self.buffer);
        self.bytes_written += self.buffer.len();
        self.stream.write(self.buffer)
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

    /// return the number of bytes lost due to threshold cuts and integer operations
    #[cfg(tune)]
    pub fn get_bytes_lost(&self) -> (f32, f32) {
        let (a,b) = self.range.get_bits_lost();
        (a/8.0, b/8.0)
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
        assert!(self.bytes_read > 0);
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
    /// total frequency (constant)
    priv total: Border,
}

impl BinaryModel {
    /// Create a new flat (50/50 probability) instance
    pub fn new_flat(threshold: Border) -> BinaryModel {
        assert!(threshold >= 2);
        BinaryModel {
            zero: threshold>>1,
            total: threshold,
        }
    }

    /// Create a new instance with a given percentage for zeroes
    pub fn new_custom(zero_percent: u8, threshold: Border) -> BinaryModel {
        assert!(threshold >= 100);
        BinaryModel {
            zero: (zero_percent as Border)*threshold/100,
            total: threshold,
        }
    }

    /// Reset the model to 50/50 distribution
    pub fn reset_flat(&mut self) {
        self.zero = self.total>>1;
    }

    /// Return the probability of 0
    pub fn get_probability_zero(&self) -> Border {
        self.zero
    }

    /// Return the probability of 1
    pub fn get_probability_one(&self) -> Border {
        self.total - self.zero
    }

    /// Update the frequency of zero
    pub fn update_zero(&mut self, factor: uint) {
        debug!("\tUpdating zero by a factor of {}", factor);
        self.zero += (self.total-self.zero) >> factor;
    }

    /// Update the frequency of one
    pub fn update_one(&mut self, factor: uint) {
        debug!("\tUpdating one by a factor of {}", factor);
        self.zero -= self.zero >> factor;
    }

    /// Update frequencies in favor of given 'value'
    /// Lower factors produce more aggressive updates
    pub fn update(&mut self, value: Value, factor: uint) {
        assert!(value < 2);
        if value==1 {
            self.update_one(factor)
        }else {
            self.update_zero(factor)
        }
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


/// A proxy model for the combination of two binary models
/// using equation: (wa * A + wb * B) >> ws
pub struct BinarySumProxy<'a> {
    priv first: &'a BinaryModel,
    priv second: &'a BinaryModel,
    priv w_first: Border,
    priv w_second: Border,
    priv w_shift: Border,
}

impl<'a> BinarySumProxy<'a> {
    /// Create a new instance of the binary sum proxy
    pub fn new(wa: Border, first: &'a BinaryModel, wb: Border, second: &'a BinaryModel, shift: Border) -> BinarySumProxy<'a> {
        BinarySumProxy {
            first: first,
            second: second,
            w_first: wa,
            w_second: wb,
            w_shift: shift,
        }
    }

    fn get_probability_zero(&self) -> Border {
        (self.w_first * self.first.get_probability_zero() +
            self.w_second * self.second.get_probability_zero()) >>
            self.w_shift
    }
}

impl<'a> Model for BinarySumProxy<'a> {
    fn get_range(&self, value: Value) -> (Border,Border) {
        let zero = self.get_probability_zero();
        if value==0 {
            (0, zero)
        }else {
            (zero, self.get_denominator())
        }
    }

    fn find_value(&self, offset: Border) -> (Value,Border,Border) {
        let zero = self.get_probability_zero();
        let total = self.get_denominator();
        assert!(offset < total,
            "Invalid frequency offset {} requested under total {}",
            offset, total);
        if offset < zero {
            (0, 0, zero)
        }else {
            (1, zero, total)
        }
    }

    fn get_denominator(&self) -> Border {
        (self.w_first * self.first.get_denominator() +
            self.w_second * self.second.get_denominator()) >>
            self.w_shift
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

    /// Return read-only frequencies slice
    pub fn get_frequencies<'a>(&'a self) -> &'a [Frequency] {
        self.table.as_slice()
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


/// A proxy model for the sum of two frequency tables
/// using equation: (wa * A + wb * B) >> ws
pub struct TableSumProxy<'a> {
    priv first: &'a FrequencyTable,
    priv second: &'a FrequencyTable,
    priv w_first: Border,
    priv w_second: Border,
    priv w_shift: Border,
}

impl<'a> TableSumProxy<'a> {
    /// Create a new instance of the table sum proxy
    pub fn new(wa: Border, fa: &'a FrequencyTable, wb: Border, fb: &'a FrequencyTable, shift: Border) -> TableSumProxy<'a> {
        assert_eq!(fa.get_frequencies().len(), fb.get_frequencies().len());
        TableSumProxy {
            first: fa,
            second: fb,
            w_first: wa,
            w_second: wb,
            w_shift: shift,
        }
    }
}

impl<'a> Model for TableSumProxy<'a> {
    fn get_range(&self, value: Value) -> (Border,Border) {
        let (lo0, hi0) = self.first.get_range(value);
        let (lo1, hi1) = self.second.get_range(value);
        let (wa, wb, ws) = (self.w_first, self.w_second, self.w_shift);
        ((wa*lo0 + wb*lo1)>>ws, (wa*hi0 + wb*hi1)>>ws)
    }

    fn find_value(&self, offset: Border) -> (Value,Border,Border) {
        assert!(offset < self.get_denominator(),
            "Invalid frequency offset {} requested under total {}",
            offset, self.get_denominator());
        let mut value = 0u;
        let mut lo = 0 as Border;
        let mut hi;
        while {  hi = lo +
                (self.w_first * (self.first.get_frequencies()[value] as Border) +
                self.w_second * (self.second.get_frequencies()[value] as Border)) >>
                self.w_shift;
                hi <= offset } {
            lo = hi;
            value += 1;
        }
        (value, lo, hi)
    }

    fn get_denominator(&self) -> Border {
        (self.w_first * self.first.get_denominator() +
            self.w_second * self.second.get_denominator()) >>
            self.w_shift
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
    use test;

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip Ari of size {}", bytes.len());
        let mut e = super::ByteEncoder::new(MemWriter::new());
        e.write(bytes).unwrap();
        let (e, r) = e.finish();
        r.unwrap();
        let encoded = e.unwrap();
        debug!("Roundtrip input {:?} encoded {:?}", bytes, encoded);
        let mut d = super::ByteDecoder::new(BufReader::new(encoded));
        let decoded = d.read_to_end().unwrap();
        assert_eq!(bytes.as_slice(), decoded.as_slice());
    }

    fn encode_binary(bytes: &[u8], model: &mut super::BinaryModel, factor: uint) -> ~[u8] {
        let mut encoder = super::Encoder::new(MemWriter::new());
        for &byte in bytes.iter() {
            for i in range(0,8) {
                let bit = ((byte as super::Value)>>i) & 1;
                encoder.encode(bit, model).unwrap();
                model.update(bit, factor);
            }
        }
        let (writer, err) = encoder.finish();
        err.unwrap();
        writer.unwrap()
    }

    fn roundtrip_binary(bytes: &[u8], factor: uint) {
        let mut bm = super::BinaryModel::new_flat(super::range_default_threshold >> 3);
        let output = encode_binary(bytes, &mut bm, factor);
        bm.reset_flat();
        let mut decoder = super::Decoder::new(BufReader::new(output));
        decoder.start().unwrap();
        for &byte in bytes.iter() {
            let mut value = 0u8;
            for i in range(0,8) {
                let bit = decoder.decode(&bm).unwrap();
                bm.update(bit, factor);
                value += (bit as u8)<<i;
            }
            assert_eq!(value, byte);
        }
    }

    fn roundtrip_proxy(bytes: &[u8]) {
        // prepare data
        let factor0 = 3;
        let factor1 = 5;
        let update0 = 10;
        let update1 = 5;
        let threshold = super::range_default_threshold >> 3;
        let mut t0 = super::FrequencyTable::new_flat(16, threshold);
        let mut t1 = super::FrequencyTable::new_flat(16, threshold);
        let mut b0 = super::BinaryModel::new_flat(threshold);
        let mut b1 = super::BinaryModel::new_flat(threshold);
        // encode (high 4 bits with the proxy table, low 4 bits with the proxy binary)
        let mut encoder = super::Encoder::new(MemWriter::new());
        for &byte in bytes.iter() {
            let high = (byte>>4) as super::Value;
            {
                let proxy = super::TableSumProxy::new(2, &t0, 1, &t1, 0);
                encoder.encode(high, &proxy).unwrap();
            }
            t0.update(high, update0, 1);
            t1.update(high, update1, 1);
            for i in range(0,4) {
                let bit = ((byte as super::Value)>>i) & 1;
                {
                    let proxy = super::BinarySumProxy::new(1, &b0, 1, &b1, 1);
                    encoder.encode(bit, &proxy).unwrap();
                }
                b0.update(bit, factor0);
                b1.update(bit, factor1);
            }
        }
        let (writer, err) = encoder.finish();
        err.unwrap();
        let buffer = writer.unwrap();
        // decode
        t0.reset_flat();
        t1.reset_flat();
        b0.reset_flat();
        b1.reset_flat();
        let mut decoder = super::Decoder::new(BufReader::new(buffer));
        decoder.start().unwrap();
        for &byte in bytes.iter() {
            let high = {
                let proxy = super::TableSumProxy::new(2, &t0, 1, &t1, 0);
                decoder.decode(&proxy).unwrap()
            };
            t0.update(high, update0, 1);
            t1.update(high, update1, 1);
            let mut value = (high<<4) as u8;
            for i in range(0,4) {
                let bit = {
                    let proxy = super::BinarySumProxy::new(1, &b0, 1, &b1, 1);
                    decoder.decode(&proxy).unwrap()
                };
                value += (bit as u8)<<i;
                b0.update(bit, factor0);
                b1.update(bit, factor1);
            }
            assert_eq!(value, byte);
        }
    }

    #[test]
    fn roundtrips() {
        roundtrip(bytes!("abracadabra"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("../data/test.txt"));
    }

    #[test]
    fn roundtrips_binary() {
        roundtrip_binary(bytes!("abracadabra"), 1);
        roundtrip_binary(include_bin!("../data/test.txt"), 5);
    }

    #[test]
    fn roundtrips_proxy() {
        roundtrip_proxy(bytes!("abracadabra"));
        roundtrip_proxy(include_bin!("../data/test.txt"));
    }

    #[bench]
    fn compress_speed(bh: &mut test::BenchHarness) {
        let input = include_bin!("../data/test.txt");
        let mut storage = vec::from_elem(input.len(), 0u8);
        bh.iter(|| {
            let mut w = BufWriter::new(storage);
            w.seek(0, SeekSet).unwrap();
            let mut e = super::ByteEncoder::new(w);
            e.write(input).unwrap();
        });
        bh.bytes = input.len() as u64;
    }
}
