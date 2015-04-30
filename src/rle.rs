//! Run time length encoding and decoding based on byte streams.
//!
//! A run is defined as a sequence of identical bytes of length two or greater. 
//! A run of byte a and length n is encoded by a two repitions of a, followed 
//! by a length specification which describes how much often these bytes are 
//! repeated. Such a specification is a string of bytes
//! with dynamic length. The most significat bit of each byte in this string 
//! indicates if the byte is the last byte in the string. The rest of the bits
//! are concatenated using the Little Endian convention.
//!
//! Every byte string is a prefix of a valid RLE encoding!

use std::io::{self, Write, Read};
use std::iter::repeat;
use std::collections::VecDeque;

/// This structure is used to compress a stream of bytes using a RLE
/// compression algorithm. This is a wrapper around an internal writer which
/// bytes will be written to.
pub struct Encoder<W> {
    w: W,
    reps: u64,
    byte: u8,
    in_run: bool
}

impl<W: Write> Encoder<W> {
    /// Creates a new encoder which will have its output written to the given
    /// output stream.
    pub fn new(w: W) -> Encoder<W> {
        Encoder {
            w: w,
            reps: 0,
            byte: 0,
            in_run: false
        }
    }

    /// This function is used to flag that this session of compression is done
    /// with. The stream is finished up (final bytes are written), and then the
    /// wrapped writer is returned.
    pub fn finish(mut self) -> (W, io::Result<()>) {
        (self.w, self.flush())
    }

    fn process_byte(&mut self, byte: u8) -> io::Result<()> {
        // TODO: move this check to write, so it won't be called as often?
        if ! self.in_run {
            self.byte = byte;
            self.reps = 1;
            self.in_run = true;
            return Ok(());
        }

        if self.byte == byte {
            self.reps += 1;
            return Ok(())
        }

        if self.byte != byte {
            self.flush();
            self.reps = 1;
            self.byte = byte;
        }

        Ok(())
    }
}

impl<W: Write> Write for Encoder<W> {
    fn write(&mut self, mut buf: &[u8]) -> io::Result<usize> {
        let mut bytes_written = 0;

        for byte in buf {
            try!(self.process_byte(*byte));
            bytes_written += 1;
        }

        Ok(bytes_written)
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.reps == 1 {
            try!(self.w.write(&[self.byte]));
        } else if self.reps > 1 {
            let mut buf = [0; 11];
            let mut reps_encode = self.reps - 2;
            buf[0] = self.byte;
            buf[1] = self.byte;

            let mut index = 2;

            loop {
                buf[index] = (reps_encode & 0b0111_1111) as u8;
                reps_encode = reps_encode >> 7;

                if reps_encode == 0 {
                    buf[index] = buf[index] | 0b1000_0000;
                    break;
                }

                index += 1;
            }

            try!(self.w.write(&buf[..(index + 1)]));
        }

        Ok(())
    }
}

struct RunBuilder {
    byte: u8,
    slice: [u8; 9],
    byte_count: u8
}

impl RunBuilder {
    fn new(byte: u8) -> Run {
        Run {
            byte: u8,
            slice: [0; 9],
            byte_count: 0
        }
    }
}

struct Run {
    byte: u8,
    reps: u64
}

enum DecoderState {
    Clean,
    Single(u8),
    Run(RunBuilder)
}

pub struct Decoder<R> {
    r: R,
    buf: VecDeque<u8>,
    state: DecoderState,
    run: Option<Run>
}

impl<R: Read> Decoder<R> {
    pub fn new(r: R) -> Decoder<R> {
        Decoder {
            r: r,
            state: DecoderState::Clean
        }
    }

    fn read_byte(&mut self) -> io::Result<Option<u8>> {
        if let None = self.run {
            self.run = try!(self.read_run());
        }

        if let Some(Run { byte: b, reps: r }) = self.run {
            if r <= 1 {
                self.run = None;
            } else {
                self.run = Some(Run { byte: b, reps: r - 1 });
            }

            return Ok(Some(b))
        }

        Ok(None)
    }

    fn read_run(&mut self) -> io::Result<Run> {
        // TODO: diz b wher da magik happen
    }

    fn fill_buff(&mut self) -> io::Result<()> {
        let mut buf = [0u8; 256];
        try!(self.r.read(&mut buf));

        for &byte in buf {
            self.buf.push_back(byte);
        }

        Ok(())
    }

    fn fetch_byte(&mut self) -> io::Result<Option<u8>> {
        match self.buf.pop_front() {
            Some(byte) => Ok(Some(byte)),
            None => {
                try!(self.fill_buf());

                match self.buf.pop_front() {
                    None => Ok(None),
                    Ok(byte) => Ok(Some(byte))
                }
            }
        }
    }
}

impl<R: Read> Read for Decoder<R> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let mut bytes_read = 0;

        for _ in 0..buf.len() {
            match try!(self.read_byte()) {
                Some(b) => buf[bytes_read] = b,
                None => return Ok(bytes_read)
            }

            bytes_read += 1;
        }

        Ok(bytes_read)
    }
}

#[cfg(test)]
mod test {
    use super::Encoder;
    use std::io::Write;
    use std::iter::{Iterator, repeat};

    fn test_encode(input: &[u8], output: &[u8]) {
        let mut encoder = Encoder::new(Vec::new());
        encoder.write_all(input).unwrap();
        let (buf, _) = encoder.finish();

        assert_eq!(&buf[..], output);
    }

    #[test]
    fn simple_encoding() {
        test_encode(b"", b"");
        test_encode(b"a", b"a");
        test_encode(b"abca123", b"abca123");
        test_encode(&[20, 20, 20, 20, 20, 15], &[20, 20, 5 - 2 + 128, 15]);
        test_encode(&[0, 0], &[0, 0, 2 - 2 + 128]);
    }

    #[test]
    fn long_runs() {
        let mut data = repeat(5).take(129).collect::<Vec<_>>();
        test_encode(&data[..], &[5, 5, 255]);

        let mut data = [1, 3, 4, 4].iter().map(|&x| x).chain(repeat(100).take(2 + 52 + 128)).collect::<Vec<_>>();
        test_encode(&data[..], &[1, 3, 4, 4, 0 + 128, 100, 100, 52, 1 + 128]);
    }
}
