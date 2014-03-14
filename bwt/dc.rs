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

use std::io;
use vec = std::slice;
use super::mtf::MTF;

pub type Symbol = u8;
pub type Rank = u8;
pub static TotalSymbols: uint = 0x100;

/// Distance coding context
pub struct Context<D> {
    /// current symbol
    symbol: Symbol,
    /// last known MTF rank
    last_rank: Option<Rank>,
    /// maximum possible distance
    distance_limit: D,
}

impl<D> Context<D> {
    /// create a new distance context
    pub fn new(s: Symbol, r: Option<Rank>, d: D) -> Context<D> {
        Context {
            symbol: s,
            last_rank: r,
            distance_limit: d,
        }
    }
}


/// encode a block of bytes 'input'
/// write output distance stream into 'distances'
/// return: unique bytes encountered in the order they appear
/// with the corresponding initial distances
pub fn encode<D: Clone + Eq + NumCast>(input: &[Symbol], distances: &mut [D], mtf: &mut MTF) -> ~[(Symbol,D)] {
    let n = input.len();
    assert_eq!(distances.len(), n);
    let mut last = [n, ..TotalSymbols];
    let mut unique: ~[(Symbol,D)] = ~[];
    let filler: D = NumCast::from(n).unwrap();
    for (i,&sym) in input.iter().enumerate() {
        distances[i] = filler.clone();
        let base = last[sym as uint];
        last[sym as uint] = i;
        debug!("\tProcessing symbol {} at position {}, last known at {}", sym, i, base);
        if base == n {
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
        let base = last[sym as uint];
        debug!("\tSweep symbol {} of rank {}, last known at {}, encoding {}", sym, rank, base, n-base-rank-1);
        assert!(n >= base+rank+1);
        distances[base] = NumCast::from(n-base-rank-1).unwrap();
    }
    assert_eq!(input.iter().zip(input.iter().skip(1)).zip(distances.iter()).
        position(|((&a,&b),d)| *d==filler && a!=b), None);
    unique
}

/// encode with "batteries included" for quick testing
pub fn encode_simple<D: Clone + Eq + NumCast>(input: &[Symbol]) -> (~[Symbol],~[D]) {
    let n = input.len();
    if n==0 {
        (~[],~[])
    }else   {
        let mut raw_dist: ~[D] = vec::from_elem(n, NumCast::from(0).unwrap());
        let pairs = encode(input, raw_dist, &mut MTF::new());
        let symbols: ~[Symbol] = pairs.iter().map(|&(sym,_)| sym).collect();
        let init_iter = pairs.iter().map(|pair| { let (_, ref d) = *pair; d.clone() });
        let filler: D = NumCast::from(n).unwrap();
        // chain initial distances with intermediate ones
        let raw_iter = raw_dist.iter().filter_map(|d| if *d!=filler {Some(d.clone())} else {None});
        let mut combined = init_iter.chain(raw_iter);
        (symbols, combined.collect())
    }
}

/// Decode a block of distances with a list of initial symbols
pub fn decode(alphabet: Option<&[Symbol]>, output: &mut [Symbol], mtf: &mut MTF,
        fn_dist: |&Context<uint>|->io::IoResult<uint>) -> io::IoResult<()> {
    let n = output.len();
    let mut next = [n, ..TotalSymbols];
    let mut ranks = [0 as Rank, ..TotalSymbols];
    let alphabet_size = match alphabet  {
        Some([]) => {
            // alphabet is empty
            assert_eq!(n, 0);
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
                let ctx = Context::new(sym, None, n);
                // initial distances are not ordered
                next[sym] = match fn_dist(&ctx) {
                    Ok(d) => d, // + (rank as Distance)
                    Err(e) => return Err(e)
                };
                mtf.symbols[rank] = sym;
                debug!("\tRegistering symbol {} of rank {} at position {}", sym, rank, next[sym as uint]);
            }
            for rank in range(list.len(),TotalSymbols) {
                mtf.symbols[rank] = 0; //erazing unused symbols
            }
            list.len()
        },
        None => {
            // alphabet is large, total range of symbols is assumed
            for i in range(0,TotalSymbols) {
                let sym = i as Symbol;
                let ctx = Context::new(sym, None, n);
                next[i] = match fn_dist(&ctx) {
                    Ok(d) => d,
                    Err(e) => return Err(e)
                };
                mtf.symbols[i] = sym;
                ranks[sym] = i as Rank;
                debug!("\tRegistering symbol {} at position {}", i, next[i]);
            }
            // sort ranks by first occurrence
            mtf.symbols.mut_slice_to(TotalSymbols).sort_by(|&a,&b| next[a as uint].cmp(&next[b as uint]));
            TotalSymbols
        },
    };
    let mut i = 0u;
    while i<n {
        let sym = mtf.symbols[0];
        let stop = next[mtf.symbols[1] as uint];
        debug!("\tFilling region [{}-{}) with symbol {}", i, stop, sym);
        while i<stop    {
            output[i] = sym;
            i += 1;
        }
        let ctx = Context::new(sym, Some(ranks[sym]), n-i);
        let future = match fn_dist(&ctx) {
            Ok(d) => stop + d,
            Err(e) => return Err(e)
        };
        debug!("\t\tLooking for future position {}", future);
        assert!(future <= n);
        let mut rank = 1u;
        while rank < alphabet_size && future+rank > next[mtf.symbols[rank] as uint] {
            mtf.symbols[rank-1] = mtf.symbols[rank];
            rank += 1;
        }
        if rank < alphabet_size {
            debug!("\t\tFound sym {} of rank {} at position {}", mtf.symbols[rank],
                rank, next[mtf.symbols[rank] as uint]);
        }else {
            debug!("\t\tNot found");
        }
        mtf.symbols[rank-1] = sym;
        debug!("\t\tAssigning future pos {} for symbol {}", future+rank-1, sym);
        next[sym] = future+rank-1;
        ranks[sym] = rank as Rank;
    }
    assert_eq!(next.iter().position(|&d| d<n || d>=n+alphabet_size), None);
    assert_eq!(i, n);
    Ok(())
}

/// decode with "batteries included" for quick testing
pub fn decode_simple<D: ToPrimitive>(n: uint, alphabet: &[Symbol], distances: &[D]) -> ~[Symbol] {
    let mut output = vec::from_elem(n, 0 as Symbol);
    let mut di = 0u;
    decode(Some(alphabet), output.as_mut_slice(), &mut MTF::new(), |_ctx| {
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
    use std;

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip DC of size {}", bytes.len());
        let (alphabet,distances) = super::encode_simple::<uint>(bytes);
        debug!("Roundtrip DC input: {:?}, alphabet: {:?}, distances: {:?}", bytes, alphabet, distances);
        let decoded = super::decode_simple(bytes.len(), alphabet.as_slice(), distances.as_slice());
        assert_eq!(decoded.as_slice(), bytes);
    }

    fn roundtrip_full_alphabet(bytes: &[u8]) {
        let n = bytes.len();
        info!("Roundtrip DC (full alphabet) of size {}", n);
        let mut mtf = super::MTF::new();
        // encoding with full alphabet
        let mut raw_dist = std::slice::from_elem(n, 0u);
        let pairs = super::encode(bytes, raw_dist, &mut mtf);
        let mut alphabet = std::slice::from_elem(0x100, n);
        for &(sym,dist) in pairs.iter() {
            alphabet[sym as uint] = dist;
        }
        let raw_iter = raw_dist.iter().filter(|&d| *d!=n);
        let distances: ~[&uint] = alphabet.iter().chain(raw_iter).collect();
        // decoding with full alphabet
        let mut decoded = std::slice::from_elem(n, 0 as super::Symbol);
        let mut di = 0u;
        super::decode(None, decoded.as_mut_slice(), &mut mtf, |_sym| {
            di += 1;
            Ok(distances[di-1].to_uint().unwrap())
        }).unwrap();
        // comparing with input
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn roundtrips_short() {
        roundtrip(bytes!("teeesst_dc"));
        roundtrip(bytes!(""));
        roundtrip(include_bin!("../data/test.txt"));
    }

    #[test]
    fn roundtrips_long() {
        let input: ~[u8] = std::iter::range_inclusive(0u8, 0xFFu8).collect();
        roundtrip_full_alphabet(input);
    }
}
