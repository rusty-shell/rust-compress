/*!

DC (Distance Coding) forward and backward transformation.
Designed to be used on BWT block output for compression.

MTF (Move To Front) encoder/decoder:
Used internally for DC processing.
Can also be used separately on the BWT output as an alternative to DC.

# Links

http://www.data-compression.info/Algorithms/DC/
http://en.wikipedia.org/wiki/Move-to-front_transform

# Example

```rust
use compress::dc;

let bytes = bytes!("abracadabra");
let (alphabet,distances) = dc::encode_simple(bytes);
let decoded = dc::decode_simple(bytes.len(),
                                alphabet.as_slice(),
                                distances.as_slice());
```

# Credit

This is an original implementation.
Thanks to Edgar Binder for inventing DC!

*/

use std::{iter, util, vec};

pub type Symbol = u8;
pub type Rank = u8;
pub type Distance = uint;
pub static TotalSymbols: uint = 0x100;


/// MoveToFront encoder/decoder
pub struct MTF {
    /// rank-ordered list of unique Symbols
    symbols: [Symbol, ..TotalSymbols],
}

impl MTF {
    /// create a new zeroed MTF
    pub fn new() -> MTF {
        MTF { symbols: [0, ..TotalSymbols] }
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
        let mut rank: Rank = 1u8;
        loop {
            util::swap(&mut self.symbols[rank], &mut next);
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
        let sym = self.symbols[rank];
        debug!("\tDecoding rank {} with symbol {}", rank, sym);
        for i in iter::range_inclusive(1,rank).rev() {
            self.symbols[i] = self.symbols[i-1];
        }
        self.symbols[0] = sym;
        sym
    }
}


/// encode a block of bytes 'input'
/// write output distance stream into 'distances'
/// return: unique bytes encountered in the order they appear
/// with the corresponding initial distances
pub fn encode(input: &[Symbol], distances: &mut [Distance], mtf: &mut MTF) -> ~[(Symbol,Distance)] {
    let N = input.len();
    assert_eq!(distances.len(), N);
    let mut last = [N, ..TotalSymbols];
    let mut unique: ~[(Symbol,Distance)] = ~[];
    for (i,&sym) in input.iter().enumerate() {
        distances[i] = N;
        let base = last[sym];
        last[sym] = i;
        debug!("\tProcessing symbol {} at position {}, last known at {}", sym, i, base);
        if base == N {
            let rank = unique.len();
            mtf.symbols[rank] = sym;
            mtf.encode(sym);    //==rank
            debug!("\t\tUnique => assigning rank {}, encoding {}", rank, i-rank);
            unique.push((sym,i-rank))
        }else {
            let rank = mtf.encode(sym) as Distance;
            if rank > 0 {
                debug!("\t\tRegular at rank {}, encoding {}", rank, i-base-rank-1);
                assert!(i >= base+rank+1);
                distances[base] = i-base-rank-1;
            }
        }
    }
    for (rank,&sym) in mtf.symbols.slice_to(unique.len()).iter().enumerate() {
        let base = last[sym];
        debug!("\tSweep symbol {} of rank {}, last known at {}, encoding {}", sym, rank, base, N-base-rank-1);
        assert!(N >= base+rank+1);
        distances[base] = N-base-rank-1;
    }
    assert_eq!(input.iter().zip(input.iter().skip(1)).zip(distances.iter()).
        position(|((&a,&b),&d)| d==N && a!=b), None);
    unique
}

/// encode with "batteries included" for quick testing
pub fn encode_simple(input: &[Symbol]) -> (~[Symbol],~[Distance]) {
    let N = input.len();
    if N==0 {
        (~[],~[])
    }else   {
        let mut raw_dist = vec::from_elem(N, 0 as Distance);
        let pairs = encode(input, raw_dist.as_mut_slice(), &mut MTF::new());
        let symbols = pairs.map(|&(sym,_)| sym);
        // skip the distance for the first unique symbol as it's always 0
        let init_iter = pairs.iter().skip(1).map(|&(_,d)| d);
        // chain initial distances with intermediate ones
        let raw_iter = raw_dist.iter().filter_map(|&d| if d!=N {Some(d)} else {None});
        let mut combined = init_iter.chain(raw_iter);
        (symbols,combined.collect())
    }
}

/// decode a block of distances with a list of initial symbols
pub fn decode(alphabet: &[Symbol], distances: &[Distance], output: &mut [Symbol], mtf: &mut MTF) {
    let N = output.len();
    let E = alphabet.len();
    match alphabet  {
        [] => {
            assert_eq!(N,0);
            return
        },
        [sym] => {
            for out in output.mut_iter()    {
                *out = sym;
            }
            return
        }
        _ => ()
    }
    assert!(1+distances.len() <= N + E);
    let mut next = [N, ..TotalSymbols];
    for (rank,&sym) in alphabet.iter().enumerate()   {
        next[sym] = if rank==0 {
            0 // not encoded, always 0
        }else {
            distances[rank-1] + (rank as Distance)
        };
        debug!("\tRegistering symbol {} of rank {} at position {}", sym, rank, next[sym]);
        mtf.symbols[rank] = sym;
    }
    let mut di = E-1;
    for rank in range(E,TotalSymbols) {
        mtf.symbols[rank] = 0; //erazing unused symbols
    }
    let mut i = 0u;
    while i<N && di<distances.len() {
        let sym = mtf.symbols[0];
        let stop = next[mtf.symbols[1]];
        debug!("\tFilling region [{}-{}) with symbol {}", i, stop, sym);
        while i<stop    {
            output[i] = sym;
            i += 1;
        }
        let future = stop + distances[di];
        debug!("\t\tLooking for future position {}", future);
        di += 1;
        let mut rank = 1u;
        while rank < E && future+rank > next[mtf.symbols[rank]] {
            mtf.symbols[rank-1] = mtf.symbols[rank];
            rank += 1;
        }
        if rank<E {
            debug!("\t\tFound sym {} of rank {} at position {}", mtf.symbols[rank],
                rank, next[mtf.symbols[rank]]);
        }else {
            debug!("\t]tNot found");
        }
        mtf.symbols[rank-1] = sym;
        debug!("\t\tAssigning future pos {} for symbol {}", future+rank-1, sym);
        next[sym] = future+rank-1;
    }
    assert_eq!(next.iter().position(|&d| d<N || d>=N+E), None);
    assert_eq!((i,di), (N,distances.len()));
}

/// decode with "batteries included" for quick testing
pub fn decode_simple(N: uint, alphabet: &[Symbol], distances: &[Distance]) -> ~[Symbol] {
    let mut output = vec::from_elem(N, 0 as Symbol);
    decode(alphabet, distances, output.as_mut_slice(), &mut MTF::new());
    output
}


#[cfg(test)]
mod test {
    //use extra::test;
    use super::{MTF, encode_simple, decode_simple};

    fn roundtrip_dc(bytes: &[u8]) {
        info!("Roundtrip DC of size {}", bytes.len());
        let (alphabet,distances) = encode_simple(bytes);
        debug!("Roundtrip DC input: {:?}, alphabet: {:?}, distances: {:?}", bytes, alphabet, distances);
        let decoded = decode_simple(bytes.len(), alphabet.as_slice(), distances.as_slice());
        assert_eq!(decoded.as_slice(), bytes);
    }

    fn roundtrip_mtf(bytes: &[u8]) {
        info!("Roundtrip MTF of size {}", bytes.len());
        let mut mtf = MTF::new();
        mtf.reset_alphabetical();
        let ranks = bytes.map(|&sym| mtf.encode(sym));
        debug!("Roundtrip MTF input: {:?}, ranks: {:?}", bytes, ranks);
        mtf.reset_alphabetical();
        let decoded = ranks.map(|&r| mtf.decode(r));
        assert_eq!(decoded.as_slice(), bytes);
    }

    #[test]
    fn some_roundtrips_dc() {
        roundtrip_dc(bytes!("teeesst_dc"));
        roundtrip_dc(bytes!(""));
        roundtrip_dc(include_bin!("data/test.txt"));
    }

    #[test]
    fn some_roundtrips_mtf() {
        roundtrip_mtf(bytes!("teeesst_mtf"));
        roundtrip_mtf(bytes!(""));
        roundtrip_mtf(include_bin!("data/test.txt"));
    }
}
