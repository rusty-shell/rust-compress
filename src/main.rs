#![crate_type = "bin"]

#![feature(box_syntax)]
//! A rust-compress application that allows testing of implemented
//! algorithms and their combinations using a simple command line.
//! Example invocations:
//! echo -n "abracadabra" | ./app bwt | xxd
//! echo "banana" | ./app bwt | ./app -d

#[macro_use] extern crate log;
extern crate compress;
extern crate byteorder;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::{env, str};
use compress::{bwt, lz4, ReadExact};
use compress::entropy::ari;
use byteorder::{LittleEndian, WriteBytesExt, ReadBytesExt};

static MAGIC    : u32   = 0x73632172;   //=r!cs

struct Config {
    exe_name: String,
    methods: Vec<String>,
    block_size: usize,
    decompress: bool,
}

impl Config {
    fn query<I>(mut args: I) -> Config where I: Iterator<Item = String> + Sized {
        let mut cfg = Config {
            exe_name: args.next().unwrap().clone(),
            methods: Vec::new(),
            block_size: 1<<16,
            decompress: false,
        };
        let mut handlers: HashMap<&str, Box<FnMut(&str, &mut Config)>> =
            HashMap::new();
        handlers.insert("d", box |_, cfg| { cfg.decompress = true; });
        handlers.insert("block", box |b, cfg| {
            cfg.block_size = b.parse().unwrap();
        });

        for arg in args {
			let slice = &arg[..];
            if slice.starts_with("-") {
                match handlers.iter_mut().find(|&(&k,_)| slice[1..].starts_with(k)) {
                    Some((k,h)) => (*h)(&slice[1+k.len()..], &mut cfg),
                    None => println!("Warning: unrecognized option: {}", &arg[..]),
                }
            }else {
                cfg.methods.push(arg.to_string());
            }
        }
        cfg
    }
}

struct Pass {
    encode: Box<FnMut(Box<Write + 'static>, &Config)
                      -> Box<Write + 'static> + 'static>,
    decode: Box<FnMut(Box<Read + 'static>, &Config)
                      -> Box<Read + 'static> + 'static>,
    info: String,
}


/// main entry point
pub fn main() {
    let mut passes: HashMap<String,Pass> = HashMap::new();
    passes.insert("dummy".to_string(), Pass {
        encode: box |w,_| w,
        decode: box |r,_| r,
        info: "pass-through".to_string(),
    });
    passes.insert("ari".to_string(), Pass {
        encode: box |w,_c| {
            box ari::ByteEncoder::new(w) as Box<Write + 'static>
        },
        decode: box |r,_c| {
            box ari::ByteDecoder::new(r) as Box<Read + 'static>
        },
        info: "Adaptive arithmetic byte coder".to_string(),
    });
    passes.insert("bwt".to_string(), Pass {
        encode: box |w,c| {
            box bwt::Encoder::new(w, c.block_size) as Box<Write + 'static>
        },
        decode: box |r,_c| {
            box bwt::Decoder::new(r, true) as Box<Read + 'static>
        },
        info: "Burrows-Wheeler Transformation".to_string(),
    });
    passes.insert("mtf".to_string(), Pass {
        encode: box |w,_c| {
            box bwt::mtf::Encoder::new(w) as Box<Write + 'static>
        },
        decode: box |r,_c| {
            box bwt::mtf::Decoder::new(r) as Box<Read + 'static>
        },
        info: "Move-To-Front Transformation".to_string(),
    });
    /* // looks like we are missing the encoder implementation
    passes.insert(~"flate", Pass {
        encode: |w,_c| {
            ~flate::Encoder::new(w, true) as ~Write
        },
        decode: |r,_c| {
            ~flate::Decoder::new(r, true) as ~Read
        },
        info: ~"Standardized Ziv-Lempel + Huffman variant",
    });*/
    passes.insert("lz4".to_string(), Pass {
        encode: box |w,_c| {
            box lz4::Encoder::new(w) as Box<Write + 'static>
        },
        decode: box |r,_c| { // LZ4 decoder seem to work
            box lz4::Decoder::new(r) as Box<Read + 'static>
        },
        info: "Ziv-Lempel derivative, focused at speed".to_string(),
    });

    let config = Config::query(env::args());
    let mut input = io::stdin();
    let mut output = io::stdout();
    if config.decompress {
        assert!(config.methods.is_empty(), "Decompression methods are set in stone");
        match input.read_u32::<LittleEndian>() {
            Ok(magic) if magic != MAGIC => {
                error!("Input is not a rust-compress archive");
                return
            },
            Err(e) => {
                error!("Unable to read input: {:?}", e);
                return
            },
            _ => () //OK
        }
        let methods: Vec<_> = (0..(input.read_u8().unwrap() as usize)).map(|_| {
            let len = input.read_u8().unwrap() as u64;
            let mut bytes = Vec::new();
            input.push_exactly(len, &mut bytes).unwrap();
            str::from_utf8(&bytes[..]).unwrap().to_string()
        }).collect();
        let mut rsum: Box<Read> = box input;
        for met in methods.iter() {
            info!("Found pass {}", *met);
            match passes.get_mut(met) {
                Some(pa) => rsum = (pa.decode)(rsum, &config),
                None => panic!("Pass is not implemented"),
            }
        }
        io::copy(&mut rsum, &mut output).unwrap();
    }else if config.methods.is_empty() {
        println!("rust-compress test application");
        println!("Usage:");
        println!("\t{} <options> <method1> .. <methodN> <input >output", config.exe_name);
        println!("Options:");
        println!("\t-d (to decompress)");
        println!("\t-block<N> (BWT block size)");
        println!("Passes:");
        for (name,pa) in passes.iter() {
            println!("\t{} = {}", *name, pa.info);
        }
    }else {
        output.write_u32::<LittleEndian>(MAGIC).unwrap();
        output.write_u8(config.methods.len() as u8).unwrap();
        for met in config.methods.iter() {
            output.write_u8(met.len() as u8).unwrap();
            output.write_all(met.as_bytes()).unwrap();
        }
        let mut wsum: Box<Write> = box output;
        for met in config.methods.iter() {
            match passes.get_mut(met) {
                Some(pa) => wsum = (pa.encode)(wsum, &config),
                None => panic!("Pass {} is not implemented", *met)
            }
        }
        io::copy(&mut input, &mut wsum).unwrap();
        wsum.flush().unwrap();
    }
}
