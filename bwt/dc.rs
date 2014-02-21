/*!

DC (Distance Coding) forward and backward transformation.
Designed to be used on BWT block output for compression.

# Links

http://www.data-compression.info/Algorithms/DC/

# Example

```rust
use compress::bwt::dc;

let bytes = bytes!("abracadabra");
let (alphabet,distances) = dc::encode_simple::<uint>(bytes);
let decoded = dc::decode_simple(bytes.len(),
                                alphabet.as_slice(),
                                distances.as_slice());
```

# Credit

This is an original implementation.
Thanks to Edgar Binder for inventing DC!

*/

use std::{io, vec};
use super::mtf::MTF;

pub type Symbol = u8;
pub type Rank = u8;
pub static TotalSymbols: uint = 0x100;


/// encode a block of bytes 'input'
/// write output distance stream into 'distances'
/// return: unique bytes encountered in the order they appear
/// with the corresponding initial distances
pub fn encode<D: Clone + Eq + NumCast>(input: &[Symbol], distances: &mut [D], mtf: &mut MTF) -> ~[(Symbol,D)] {
    let N = input.len();
    assert_eq!(distances.len(), N);
    let mut last = [N, ..TotalSymbols];
    let mut unique: ~[(Symbol,D)] = ~[];
    let filler: D = NumCast::from(N).unwrap();
    for (i,&sym) in input.iter().enumerate() {
        distances[i] = filler.clone();
        let base = last[sym];
        last[sym] = i;
        debug!("\tProcessing symbol {} at position {}, last known at {}", sym, i, base);
        if base == N {
            let rank = unique.len();
            mtf.symbols[rank] = sym;
            mtf.encode(sym);    //==rank
            // initial distances are not ordered to support re-shuffle
            debug!("\t\tUnique => assigning rank {}, encoding {}", rank, i);
            let d = NumCast::from(i).unwrap();
            unique.push((sym,d));
        }else {
            let rank = mtf.encode(sym) as uint;
            if rank > 0 {
                debug!("\t\tRegular at rank {}, encoding {}", rank, i-base-rank-1);
                assert!(i >= base+rank+1);
                distances[base] = NumCast::from(i-base-rank-1).unwrap();
            }
        }
    }
    for (rank,&sym) in mtf.symbols.slice_to(unique.len()).iter().enumerate() {
        let base = last[sym];
        debug!("\tSweep symbol {} of rank {}, last known at {}, encoding {}", sym, rank, base, N-base-rank-1);
        assert!(N >= base+rank+1);
        distances[base] = NumCast::from(N-base-rank-1).unwrap();
    }
    assert_eq!(input.iter().zip(input.iter().skip(1)).zip(distances.iter()).
        position(|((&a,&b),d)| *d==filler && a!=b), None);
    unique
}

/// encode with "batteries included" for quick testing
pub fn encode_simple<D: Clone + Eq + NumCast>(input: &[Symbol]) -> (~[Symbol],~[D]) {
    let N = input.len();
    if N==0 {
        (~[],~[])
    }else   {
        let mut raw_dist: ~[D] = vec::from_elem(N, NumCast::from(0).unwrap());
        let pairs = encode(input, raw_dist, &mut MTF::new());
        let symbols = pairs.map(|&(sym,_)| sym);
        let init_iter = pairs.iter().map(|pair| { let (_, ref d) = *pair; d.clone() });
        let filler: D = NumCast::from(N).unwrap();
        // chain initial distances with intermediate ones
        let raw_iter = raw_dist.iter().filter_map(|d| if *d!=filler {Some(d.clone())} else {None});
        let mut combined = init_iter.chain(raw_iter);
        (symbols,combined.collect())
    }
}

/// Decode a block of distances with a list of initial symbols
pub fn decode(alphabet: Option<&[Symbol]>, output: &mut [Symbol], mtf: &mut MTF,
        fn_dist: |Symbol|->io::IoResult<uint>) -> io::IoResult<()> {
    let N = output.len();
    let mut next = [N, ..TotalSymbols];
    let E = match alphabet  {
        Some([]) => {
            // alphabet is empty
            assert_eq!(N,0);
            return Ok(())
        },
        Some([sym]) => {
            // there is only one known symbol
            for out in output.mut_iter()    {
                *out = sym;
            }
            return Ok(())
        }
        Some(list) => {
            // given fixed alphabet
            for (rank,&sym) in list.iter().enumerate()   {
                // initial distances are not ordered
                next[sym] = match fn_dist(sym) {
                    Ok(d) => d, // + (rank as Distance)
                    Err(e) => return Err(e)
                };
                mtf.symbols[rank] = sym;
                debug!("\tRegistering symbol {} of rank {} at position {}", sym, rank, next[sym]);
            }
            for rank in range(list.len(),TotalSymbols) {
                mtf.symbols[rank] = 0; //erazing unused symbols
            }
            list.len()
        },
        None => {
            // alphabet is large, total range of symbols is assumed
            for i in range(0,TotalSymbols) {
                next[i] = match fn_dist(i as Symbol) {
                    Ok(d) => d,
                    Err(e) => return Err(e)
                };
                mtf.symbols[i] = i as Symbol;
                debug!("\tRegistering symbol {} at position {}", i, next[i]);
            }
            TotalSymbols
        },
    };
    let mut i = 0u;
    while i<N {
        let sym = mtf.symbols[0];
        let stop = next[mtf.symbols[1]];
        debug!("\tFilling region [{}-{}) with symbol {}", i, stop, sym);
        while i<stop    {
            output[i] = sym;
            i += 1;
        }
        let future = match fn_dist(sym) {
            Ok(d) => stop + d,
            Err(e) => return Err(e)
        };
        debug!("\t\tLooking for future position {}", future);
        let mut rank = 1u;
        while rank < E && future+rank > next[mtf.symbols[rank]] {
            mtf.symbols[rank-1] = mtf.symbols[rank];
            rank += 1;
        }
        if rank<E {
            debug!("\t\tFound sym {} of rank {} at position {}", mtf.symbols[rank],
                rank, next[mtf.symbols[rank]]);
        }else {
            debug!("\t\tNot found");
        }
        mtf.symbols[rank-1] = sym;
        debug!("\t\tAssigning future pos {} for symbol {}", future+rank-1, sym);
        next[sym] = future+rank-1;
    }
    assert_eq!(next.iter().position(|&d| d<N || d>=N+E), None);
    assert_eq!(i, N);
    Ok(())
}

/// decode with "batteries included" for quick testing
pub fn decode_simple<D: ToPrimitive>(N: uint, alphabet: &[Symbol], distances: &[D]) -> ~[Symbol] {
    let mut output = vec::from_elem(N, 0 as Symbol);
    let mut di = 0u;
    decode(Some(alphabet), output.as_mut_slice(), &mut MTF::new(), |_sym| {
        di += 1;
        if di > distances.len() {
            Err(io::standard_error(io::EndOfFile))
        }else {
            Ok(distances[di-1].to_uint().unwrap())
        }
    }).unwrap();
    output
}


#[cfg(test)]
mod test {
    //use extra::test;
    use super::{encode_simple, decode_simple};

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip DC of size {}", bytes.len());
        let (alphabet,distances) = encode_simple::<uint>(bytes);
        debug!("Roundtrip DC input: {:?}, alphabet: {:?}, distances: {:?}", bytes, alphabet, distances);
        let decoded = decode_simple(bytes.len(), alphabet.as_slice(), distances.as_slice());
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips() {
        roundtrip(bytes!("teeesst_dc"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("../data/test.txt"));
    }
}
