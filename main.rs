#[crate_id = "app"];
#[crate_type = "bin"];
#[deny(warnings, missing_doc)];
#[feature(macro_rules)];

//! A rust-compress application that allows testing of implemented
//! algorithms and their combinations using a simple command line.
//! Example invocations:
//! echo -n "abracadabra" | ./app bwt | xxd
//! echo "banana" | ./app bwt | ./app -d

extern mod compress;


use std::hashmap::HashMap;
use std::{io, os, str, vec};
use compress::{bwt, lz4};
//use compress::entropy::ari;


static MAGIC    : u32   = 0x73632172;   //=r!cs

struct Config {
    exe_name: ~str,
    methods: ~[~str],
    block_size: uint,
    decompress: bool,
}

impl Config {
    fn query(args: &[~str]) -> Config {
        let mut cfg = Config {
            exe_name: args[0].clone(),
            methods: ~[],
            block_size: 1<<16,
            decompress: false,
        };
        let mut handlers: HashMap<&str,|&str|> = HashMap::new();
        handlers.insert(&"d",|_| { cfg.decompress = true; });
        handlers.insert(&"block",|b| { cfg.block_size = from_str(b).unwrap(); });
        
        for arg in args.iter().skip(1) {
            if arg.starts_with(&"-") {
                match handlers.iter().find(|&(&k,_)| arg.slice_from(1).starts_with(k)) {
                    Some((k,h)) => (*h)(arg.slice_from(1+k.len())),
                    None => println!("Warning: unrecognized option: {}", *arg),
                }
            }else {
                cfg.methods.push(arg.to_owned());
            }
        }
        cfg
    }
}

struct Pass {
    encode: 'static |~Writer,&Config| -> ~io::Writer,
    decode: 'static |~Reader,&Config| -> ~io::Reader,
    info: ~str,
}


/// main entry point
pub fn main() {
    let mut passes: HashMap<~str,Pass> = HashMap::new();
    passes.insert(~"dummy", Pass {
        encode: |w,_| w,
        decode: |r,_| r,
        info: ~"pass-through",
    });
    /* // unclear what to do with Ari since it requires the size to be known
    passes.insert(~"ari", Pass {
        encode: |w,_c| {
            ~ari::ByteEncoder::new(w) as ~Writer
        },
        decode: |r,_c| {
            ~ari::ByteDecoder::new(r) as ~Reader
        },
        info: ~"Adaptive arithmetic byte coder",
    });*/
    passes.insert(~"bwt", Pass {
        encode: |w,c| {
            ~bwt::Encoder::new(w, c.block_size) as ~Writer
        },
        decode: |r,_c| {
            ~bwt::Decoder::new(r, true) as ~Reader
        },
        info: ~"Burrows-Wheeler Transformation",
    });
    /* // looks like we are missing the encoder implementation
    passes.insert(~"flate", Pass {
        encode: |w,_c| {
            ~flate::Encoder::new(w, true) as ~Writer
        },
        decode: |r,_c| {
            ~flate::Decoder::new(r, true) as ~Reader
        },
        info: ~"Standardized Ziv-Lempel + Huffman variant",
    });*/
    passes.insert(~"lz4", Pass {
        encode: |w,_c| {
            ~lz4::Encoder::new(w) as ~Writer
        },
        decode: |r,_c| { // LZ4 decoder seem to work
            ~lz4::Decoder::new(r) as ~Reader
        },
        info: ~"Ziv-Lempel derivative, focused at speed",
    });

    let config = Config::query(os::args());
    let mut input = io::stdin();
    let mut output = io::stdout();
    if config.decompress {
        assert!(config.methods.is_empty(), "Decompression methods are set in stone");
        match input.read_le_u32() {
            Ok(magic) if magic != MAGIC => {
                error!("Input is not a rust-compress archive");
                return
            },
            Err(e) => {
                error!("Unable to read input: {}", e.to_str());
                return
            },
            _ => () //OK
        }
        let methods = vec::from_fn( input.read_u8().unwrap() as uint, |_| {
            let len = input.read_u8().unwrap() as uint;
            let bytes = input.read_bytes(len).unwrap();
            str::from_utf8(bytes).unwrap().to_owned()
        });
        let mut rsum: ~Reader = ~input;
        for met in methods.iter() {
            info!("Found pass {}", *met);
            match passes.find(met) {
                Some(pa) => rsum = (pa.decode)(rsum, &config),
                None => fail!("Pass is not implemented"),
            }
        }
        io::util::copy(&mut rsum, &mut output).unwrap();
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
        output.write_le_u32(MAGIC).unwrap();
        output.write_u8(config.methods.len() as u8).unwrap();
        for met in config.methods.iter() {
            output.write_u8(met.len() as u8).unwrap();
            output.write_str(*met).unwrap();
        }
        let mut wsum: ~Writer = ~output;
        for met in config.methods.iter() {
            match passes.find(met) {
                Some(pa) => wsum = (pa.encode)(wsum, &config),
                None => fail!("Pass {} is not implemented", *met)
            }
        }
        io::util::copy(&mut input, &mut wsum).unwrap();
        wsum.flush().unwrap();
    }
}
