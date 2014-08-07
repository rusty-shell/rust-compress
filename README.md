# Rust Compresion

[![Build Status](https://travis-ci.org/alexcrichton/rust-compress.png?branch=master)](https://travis-ci.org/alexcrichton/rust-compress)

[Documentation](http://alexcrichton.com/rust-compress/compress/index.html)

**NOTE: This is not a production-quality library, it is a proof of concept. This
library mainly contains *decoders*, not *encoders*.**

This repository aims to house various implementations of compression algorithms,
all written in rust. This is still very much a work in progress.

```
git clone https://github.com/alexcrichton/rust-compress
cd rust-compress
cargo build
```

### Implemented Algorithms

The following algorithms are alredy implemented in the main branch:

* DEFLATE: standard decoder based on RFC 1951
* LZ4 (Ziv-Lempel modification): dummy encoder, semi-complete decoder
* BWT (Burrows-Wheeler Transform): straightforward encoder, standard decoder
* DC (Distance Coding): basic encoder, standard decoder
* Ari (Arithmetic coding): standard range encoder/decoder

### Desired Algorithms

The following algorithms are either planned or in development at this point:

* RLE (Run-Length Encoding)
* WFC (Weight-Frequency Coding)
* SA/BWT in linear time
