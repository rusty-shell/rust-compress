/*!

MTF (Move To Front) encoder/decoder
Produces a rank for each input character based on when it was seen last time.
Useful for BWT output encoding, which produces a lot of zeroes and low ranks.

# Links

http://en.wikipedia.org/wiki/Move-to-front_transform

# Example

```rust
use std::io;
use compress::bwt::mtf;

// Encode a stream of bytes
let bytes = bytes!("abracadabra");
let mut e = mtf::Encoder::new(io::MemWriter::new());
e.write(bytes).unwrap();
let encoded = e.finish().unwrap();

// Decode a stream of ranks
let mut d = mtf::Decoder::new(io::BufReader::new(encoded.as_slice()));
let decoded = d.read_to_end().unwrap();
```

# Credit

*/

use std::{io, iter, mem};

pub type Symbol = u8;
pub type Rank = u8;
pub static TOTAL_SYMBOLS: uint = 0x100;


/// MoveToFront encoder/decoder
pub struct MTF {
    /// rank-ordered list of unique Symbols
    pub symbols: [Symbol, ..TOTAL_SYMBOLS],
}

impl MTF {
    /// create a new zeroed MTF
    pub fn new() -> MTF {
        MTF { symbols: [0, ..TOTAL_SYMBOLS] }
    }

    /// set the order of symbols to be alphabetical
    pub fn reset_alphabetical(&mut self) {
        for (i,sym) in self.symbols.mut_iter().enumerate() {
            *sym = i as Symbol;
        }
    }

    /// encode a symbol into its rank
    pub fn encode(&mut self, sym: Symbol) -> Rank {
        let mut next = self.symbols[0];
        if next == sym {
            return 0
        }
        let mut rank: Rank = 1;
        loop {
            mem::swap(&mut self.symbols[rank as uint], &mut next);
            if next == sym {
                break;
            }
            rank += 1;
            assert!((rank as uint) < self.symbols.len());
        }
        self.symbols[0] = sym;
        rank
    }

    /// decode a rank into its symbol
    pub fn decode(&mut self, rank: Rank) -> Symbol {
        let sym = self.symbols[rank as uint];
        debug!("\tDecoding rank {} with symbol {}", rank, sym);
        for i in iter::range_inclusive(1,rank as uint).rev() {
            self.symbols[i] = self.symbols[i-1];
        }
        self.symbols[0] = sym;
        sym
    }
}


/// A simple MTF stream encoder
pub struct Encoder<W> {
    w: W,
    mtf: MTF,
}

impl<W> Encoder<W> {
    /// start encoding into the given writer
    pub fn new(w: W) -> Encoder<W> {
        let mut mtf = MTF::new();
        mtf.reset_alphabetical();
        Encoder {
            w: w,
            mtf: mtf,
        }
    }

    /// finish encoding and return the wrapped writer
    pub fn finish(self) -> W {
        self.w
    }
}

impl<W: Writer> Writer for Encoder<W> {
    fn write(&mut self, buf: &[u8]) -> io::IoResult<()> {
        for sym in buf.iter() {
            let rank = self.mtf.encode(*sym);
            try!(self.w.write_u8(rank));
        }
        Ok(())
    }

    fn flush(&mut self) -> io::IoResult<()> {
        self.w.flush()
    }
}


/// A simple MTF stream decoder
pub struct Decoder<R> {
    r: R,
    mtf: MTF,
    eof: bool,
}

impl<R> Decoder<R> {
    /// start decoding the given reader
    pub fn new(r: R) -> Decoder<R> {
        let mut mtf = MTF::new();
        mtf.reset_alphabetical();
        Decoder {
            r: r,
            mtf: mtf,
            eof: false,
        }
    }

    /// finish decoder and return the wrapped reader
    pub fn finish(self) -> R {
        self.r
    }
}

impl<R: Reader> Reader for Decoder<R> {
    fn read(&mut self, dst: &mut [u8]) -> io::IoResult<uint> {
        let mut bytes_read = 0u;
        for sym in dst.mut_iter() {
            let rank = match self.r.read_u8() {
                Ok(r) => r,
                Err(io::IoError{kind: io::EndOfFile, ..}) if bytes_read!=0 => break,
                Err(e) => return Err(e)
            };
            bytes_read += 1;
            *sym = self.mtf.decode(rank);
        }
        Ok((bytes_read))
    }
}


#[cfg(test)]
mod test {
    use std::io;
    use test::Bencher;
    use super::{Encoder, Decoder};

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip MTF of size {}", bytes.len());
        let mut e = Encoder::new(io::MemWriter::new());
        e.write(bytes).unwrap();
        let encoded = e.finish().unwrap();
        debug!("Roundtrip MTF input: {:?}, ranks: {:?}", bytes, encoded);
        let mut d = Decoder::new(io::BufReader::new(encoded.as_slice()));
        let decoded = d.read_to_end().unwrap();
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(bytes!("teeesst_mtf"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("../data/test.txt"));
    }

    #[bench]
    fn encode_speed(bh: &mut Bencher) {
        let input = include_bin!("../data/test.txt");
        let mem = io::MemWriter::with_capacity(input.len());
        let mut e = Encoder::new(mem);
        bh.iter(|| {
            e.write(input).unwrap();
        });
        bh.bytes = input.len() as u64;
    }

    #[bench]
    fn decode_speed(bh: &mut Bencher) {
        let input = include_bin!("../data/test.txt");
        let mut e = Encoder::new(io::MemWriter::new());
        e.write(input).unwrap();
        let encoded = e.finish().unwrap();
        bh.iter(|| {
            let mut d = Decoder::new(io::BufReader::new(encoded.as_slice()));
            d.read_to_end().unwrap();
        });
        bh.bytes = input.len() as u64;
    }
}
