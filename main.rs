#[crate_id = "compress"];
#[crate_type = "bin"];
#[deny(warnings, missing_doc)];
#[feature(macro_rules)];

//! A rust-compress utility that allows to test implemented algorithms
//! and their combinations using a simple command line.
//! Example invocations:
//! echo -n "abracadabra" | ./compress bwt | xxd

use std::hashmap::HashMap;
use std::io;
use std::os;
use std::vec;

pub mod bwt;
pub mod dc;


struct Config {
    exe_name: ~str,
    method: ~str,
    block_size: uint,
}

impl Config {
    fn query(args: &[~str]) -> Config {
        let mut cfg = Config {
            exe_name: args[0].clone(),
            method: ~"",
            block_size: 0,
        };
        let mut handlers: HashMap<&str,|&str|> = HashMap::new();
        handlers.insert(&"block",|b| { cfg.method = from_str(b).unwrap(); });
        
        for arg in args.iter().skip(1) {
            if arg.starts_with(&"-") {
                match handlers.iter().find(|&(&k,_)| arg.slice_from(1).starts_with(k)) {
                    Some((k,h)) => (*h)(arg.slice_from(1+k.len())),
                    None => println!("Warning: unrecognized option: {}", *arg),
                }
            }else {
                assert!(cfg.method.is_empty());
                cfg.method = arg.to_owned();
            }
        }
        cfg
    }
}

/// main entry point
pub fn main() {
    let config = Config::query(os::args());
    match config.method.as_slice() {
        &"bwt" => {
            // block parameter is not used at the moment
            let input = io::stdin().read_to_end().unwrap();
            let mut suf = vec::from_elem(input.len(), 0u as bwt::Suffix);
            let mut output = io::stdout();
            let origin = bwt::encode_brute(input, suf,
                |ch| output.write_u8(ch).unwrap());
            output.write_le_u32(origin as u32).unwrap();
        },
        &"unbwt" => {
            let input = io::stdin().read_to_end().unwrap();
            assert!(input.len() >= 4);
            let n = input.len() - 4;
            let origin = io::MemReader::new(input.slice_from(n).to_owned()).
                read_le_u32().unwrap() as bwt::Suffix;
            let mut suf = vec::from_elem(n, 0u as bwt::Suffix);
            let mut output = io::stdout();
            bwt::decode_std(input.slice_to(n), origin, suf,
                |ch| output.write_u8(ch).unwrap());
        },
        &"" => {
            println!("rust-compress test utility");
            println!("Usage:");
            println!("\t{} <options> <method> <input.bin >output.bin", config.exe_name);
            println!("Options:");
            println!("\t-block<X>[k|m]");
            println!("Methods:");
            println!("\t[un]bwt");
        }
        _ => {
            println!("Requested method '{}' is not implemented", config.method);
        }
    }
}
