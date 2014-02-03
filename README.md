# Rust Compresion

[![Build Status](https://travis-ci.org/alexcrichton/rust-compress.png?branch=master)](https://travis-ci.org/alexcrichton/rust-compress)

This repository aims to house various implementations of compression algorithms,
all written in rust. This is still very much a work in progress.

```
rustpkg install github.com/alexcrichton/rust-compress
```

### Implemented Algorithms

The following algorithms are alredy implemented in the main branch:

* LZ4 (Ziv-Lempel modification): dummy encoder, semi-complete decoder
* BWT (Burrows-Wheeler Transform): straightforward encoder, standard decoder
* DC (Distance Coding): basic encoder, standard decoder
* Ari (Arithmetic coding): standard range encoder/decoder

### Desired Algorithms

The following algorithms are either planned or in development at this point:

* flate (LZ77 + Huffman)
* RLE (Run-Length Encoding)
* WFC (Weight-Frequency Coding)
* SA/BWT in linear time
