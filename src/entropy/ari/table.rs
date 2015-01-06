/*!

Frequency table models for the arithmetic coder.
The module also implements Reader/Writer using simple byte coding.

# Links

# Example

# Credit

*/

use std::io;
use super::Border;


pub type Frequency = u16;

/// A simple table of frequencies.
pub struct Model {
    /// sum of frequencies
    total: Border,
    /// main table: value -> Frequency
    table: Vec<Frequency>,
    /// maximum allowed sum of frequency,
    /// should be smaller than RangeEncoder::threshold
    cut_threshold: Border,
    /// number of bits to shift on cut
    cut_shift: uint,
}

impl Model {
    /// Create a new table with frequencies initialized by a function
    pub fn new_custom<F>(num_values: uint, threshold: Border,
                         mut fn_init: F) -> Model
        where F: FnMut(uint) -> Frequency
    {
        let freq: Vec<Frequency> = range(0, num_values).map(|i| fn_init(i)).collect();
        let total = freq.iter().fold(0 as Border, |u,&f| u+(f as Border));
        let mut ft = Model {
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
    pub fn new_flat(num_values: uint, threshold: Border) -> Model {
        Model::new_custom(num_values, threshold, |_| 1)
    }

    /// Reset the table to the flat state
    pub fn reset_flat(&mut self) {
        for freq in self.table.iter_mut() {
            *freq = 1;
        }
        self.total = self.table.len() as Border;
    }

    /// Adapt the table in favor of given 'value'
    /// using 'add_log' and 'add_const' to produce the additive factor
    /// the higher 'add_log' is, the more concervative is the adaptation
    pub fn update(&mut self, value: uint, add_log: uint, add_const: Border) {
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
        for freq in self.table.iter_mut() {
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

impl super::Model<uint> for Model {
    fn get_range(&self, value: uint) -> (Border,Border) {
        let lo = self.table.slice_to(value).iter().fold(0, |u,&f| u+(f as Border));
        (lo, lo + (self.table[value] as Border))
    }

    fn find_value(&self, offset: Border) -> (uint,Border,Border) {
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
pub struct SumProxy<'a> {
    first: &'a Model,
    second: &'a Model,
    w_first: Border,
    w_second: Border,
    w_shift: Border,
}

impl<'a> SumProxy<'a> {
    /// Create a new instance of the table sum proxy
    pub fn new(wa: Border, fa: &'a Model, wb: Border, fb: &'a Model, shift: Border) -> SumProxy<'a> {
        assert_eq!(fa.get_frequencies().len(), fb.get_frequencies().len());
        SumProxy {
            first: fa,
            second: fb,
            w_first: wa,
            w_second: wb,
            w_shift: shift,
        }
    }
}

impl<'a> super::Model<uint> for SumProxy<'a> {
    fn get_range(&self, value: uint) -> (Border,Border) {
        let (lo0, hi0) = self.first.get_range(value);
        let (lo1, hi1) = self.second.get_range(value);
        let (wa, wb, ws) = (self.w_first, self.w_second, self.w_shift as uint);
        ((wa*lo0 + wb*lo1)>>ws, (wa*hi0 + wb*hi1)>>ws)
    }

    fn find_value(&self, offset: Border) -> (uint,Border,Border) {
        assert!(offset < self.get_denominator(),
            "Invalid frequency offset {} requested under total {}",
            offset, self.get_denominator());
        let mut value = 0u;
        let mut lo = 0 as Border;
        let mut hi;
        while {  hi = lo +
                (self.w_first * (self.first.get_frequencies()[value] as Border) +
                self.w_second * (self.second.get_frequencies()[value] as Border)) >>
                (self.w_shift as uint);
                hi <= offset } {
            lo = hi;
            value += 1;
        }
        (value, lo, hi)
    }

    fn get_denominator(&self) -> Border {
        (self.w_first * self.first.get_denominator() +
            self.w_second * self.second.get_denominator()) >>
            (self.w_shift as uint)
    }
}


/// A basic byte-encoding arithmetic
/// uses a special terminator code to end the stream
pub struct ByteEncoder<W> {
    /// A lower level encoder
    pub encoder: super::Encoder<W>,
    /// A basic frequency table
    pub freq: Model,
}

impl<W: Writer> ByteEncoder<W> {
    /// Create a new encoder on top of a given Writer
    pub fn new(w: W) -> ByteEncoder<W> {
        let freq_max = super::RANGE_DEFAULT_THRESHOLD >> 2;
        ByteEncoder {
            encoder: super::Encoder::new(w),
            freq: Model::new_flat(super::SYMBOL_TOTAL+1, freq_max),
        }
    }

    /// Finish encoding & write the terminator symbol
    pub fn finish(mut self) -> (W, io::IoResult<()>) {
        let ret = self.encoder.encode(super::SYMBOL_TOTAL, &self.freq);
        let (w,r2) = self.encoder.finish();
        (w, ret.and(r2))
    }
}

impl<W: Writer> Writer for ByteEncoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::IoResult<()> {
        buf.iter().fold(Ok(()), |result,byte| {
            let value = *byte as uint;
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
    pub decoder: super::Decoder<R>,
    /// A basic frequency table
    pub freq: Model,
    /// Remember if we found the terminator code
    is_eof: bool,
}

impl<R: Reader> ByteDecoder<R> {
    /// Create a decoder on top of a given Reader
    pub fn new(r: R) -> ByteDecoder<R> {
        let freq_max = super::RANGE_DEFAULT_THRESHOLD >> 2;
        ByteDecoder {
            decoder: super::Decoder::new(r),
            freq: Model::new_flat(super::SYMBOL_TOTAL+1, freq_max),
            is_eof: false,
        }
    }

    /// Finish decoding
    pub fn finish(self) -> (R, io::IoResult<()>) {
        self.decoder.finish()
    }
}

impl<R: Reader> Reader for ByteDecoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        if self.is_eof {
            return Err(io::standard_error(io::EndOfFile))
        }
        let mut amount = 0u;
        for out_byte in dst.iter_mut() {
            let value = try!(self.decoder.decode(&self.freq));
            if value == super::SYMBOL_TOTAL {
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
