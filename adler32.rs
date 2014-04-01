//! Adler-32 checksum implementation
//!
//! This implementation is based off the example found at
//! http://en.wikipedia.org/wiki/Adler-32.

static MOD_ADLER: u32 = 65521;

pub struct State {
    a: u32,
    b: u32,
}

impl State {
    pub fn new() -> State {
        State { a: 1, b: 0 }
    }

    pub fn feed(&mut self, buf: &[u8]) {
        for byte in buf.iter() {
            self.a = (self.a + *byte as u32) % MOD_ADLER;
            self.b = (self.a + self.b) % MOD_ADLER;
        }
    }

    pub fn result(&self) -> u32 {
        (self.b << 16) | self.a
    }

    pub fn reset(&mut self) {
        self.a = 1;
        self.b = 0;
    }
}
