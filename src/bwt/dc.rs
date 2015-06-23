/*!

DC (Distance Coding) forward and backward transformation.
Designed to be used on BWT block output for compression.

# Links

http://www.data-compression.info/Algorithms/DC/

# Example

```rust
use compress::bwt::dc;

let bytes = b"abracadabra";
let distances = dc::encode_simple::<usize>(bytes);
let decoded = dc::decode_simple(bytes.len(), &distances[..]);
```

# Credit

This is an original implementation.
Thanks to Edgar Binder for inventing DC!

*/

use std::io;
use std::iter::{self, repeat};
use std::slice as vec;
use super::num::traits::{NumCast, ToPrimitive};
use super::mtf::MTF;

pub type Symbol = u8;
pub type Rank = u8;
pub const TOTAL_SYMBOLS: usize = 0x100;

/// Distance coding context
/// Has all the information potentially needed by the underlying coding model
#[derive(PartialEq, Eq, Debug)]
pub struct Context {
    /// current symbol
    pub symbol: Symbol,
    /// last known MTF rank
    pub last_rank: Rank,
    /// maximum possible distance
    pub distance_limit: usize,
}

impl Context {
    /// create a new distance context
    pub fn new(s: Symbol, r: Rank, dmax: usize) -> Context {
        Context {
            symbol: s,
            last_rank: r,
            distance_limit: dmax,
        }
    }
}


/// DC body iterator, can be used to encode distances
pub struct EncodeIterator<'a,'b, D: 'b> {
    data: iter::Enumerate<iter::Zip<vec::Iter<'a,Symbol>,vec::Iter<'b, D>>>,
    pos: [usize; TOTAL_SYMBOLS],
    last_active: usize,
    size: usize,
}

impl<'a, 'b, D: NumCast> EncodeIterator<'a,'b, D> {
    /// create a new encode iterator
    pub fn new(input: &'a [Symbol], dist: &'b [D], init: [usize; TOTAL_SYMBOLS]) -> EncodeIterator<'a,'b,D> {
        assert_eq!(input.len(), dist.len());
        EncodeIterator {
            data: input.iter().zip(dist.iter()).enumerate(),
            pos: init,
            last_active: 0,
            size: input.len()
        }
    }

    /// get the initial symbol positions, to be called before iteration
    pub fn get_init<'c>(&'c self) -> &'c [usize; TOTAL_SYMBOLS] {
        assert_eq!(self.last_active, 0);
        &self.pos
    }
}

impl<'a, 'b, D> Iterator for EncodeIterator<'a,'b,D>
    where D: Clone + Eq + NumCast + 'b
{
    type Item = (D, Context);
    fn next(&mut self) -> Option<(D,Context)> {
        let filler: D = NumCast::from(self.size).unwrap();
        self.data.find(|&(_,(_,d))| *d != filler).map(|(i,(sym,d))| {
            let rank = self.last_active - self.pos[*sym as usize];
            assert!(rank < TOTAL_SYMBOLS);
            self.last_active = i+1;
            self.pos[*sym as usize] = i + 1 + d.to_usize().unwrap();
            debug!("Encoding distance {} at pos {} for symbol {}, computed rank {}, predicting next at {}",
                d.to_usize().unwrap(), i, *sym, rank, self.pos[*sym as usize]);
            (d.clone(), Context::new(*sym, rank as Rank, self.size-i))
        })
    }
}

/// Encode a block of bytes 'input'
/// write output distance stream into 'distances'
/// return: unique bytes encountered in the order they appear
/// with the corresponding initial distances
pub fn encode<'a, 'b, D: Clone + Copy + Eq + NumCast>(input: &'a [Symbol], distances: &'b mut [D], mtf: &mut MTF) -> EncodeIterator<'a,'b,D> {
    let n = input.len();
    assert_eq!(distances.len(), n);
    let mut num_unique = 0;
    let mut last = [n; TOTAL_SYMBOLS];
    let mut init = [n; TOTAL_SYMBOLS];
    let filler: D = NumCast::from(n).unwrap();
    for (i,&sym) in input.iter().enumerate() {
        distances[i] = filler.clone();
        let base = last[sym as usize];
        last[sym as usize] = i;
        debug!("\tProcessing symbol {} at position {}, last known at {}", sym, i, base);
        if base == n {
            let rank = num_unique;
            mtf.symbols[rank] = sym;
            mtf.encode(sym);    //==rank
            // initial distances are not ordered to support re-shuffle
            debug!("\t\tUnique => assigning rank {}, encoding {}", rank, i);
            init[sym as usize] = i;
            num_unique += 1;
        }else {
            let rank = mtf.encode(sym) as usize;
            if rank > 0 {
                debug!("\t\tRegular at rank {}, encoding {}", rank, i-base-rank-1);
                assert!(i >= base+rank+1);
                distances[base] = NumCast::from(i-base-rank-1).unwrap();
            }
        }
    }
    for (rank,&sym) in mtf.symbols[..num_unique].iter().enumerate() {
        let base = last[sym as usize];
        debug!("\tSweep symbol {} of rank {}, last known at {}, encoding {}", sym, rank, base, n-base-rank-1);
        assert!(n >= base+rank+1);
        distances[base] = NumCast::from(n-base-rank-1).unwrap();
    }
    // a basic but expensive check, to be improved
    //assert_eq!(input.iter().zip(input.iter().skip(1)).zip(distances.iter()).
    //    position(|((&a,&b),d)| *d==filler && a!=b), None);
    EncodeIterator::new(input, distances, init)
}


/// Encode version with "batteries included" for quick testing
pub fn encode_simple<D: Clone + Copy + Eq + NumCast>(input: &[Symbol]) -> Vec<D> {
    let n = input.len();
    let mut raw_dist: Vec<D> = repeat(NumCast::from(0).unwrap()).take(n).collect();
    let mut eniter = encode(input, &mut raw_dist, &mut MTF::new());
    let init: Vec<D> = (0..TOTAL_SYMBOLS).map(|i| NumCast::from(eniter.get_init()[i]).unwrap()).collect();
    init.iter().map(|d| d.clone()).chain(eniter.by_ref().map(|(d,_)| d)).collect()
}

/// Decode a block of distances given the initial symbol positions
pub fn decode<F>(mut next: [usize; TOTAL_SYMBOLS], output: &mut [Symbol], mtf: &mut MTF,
                 mut fn_dist: F) -> io::Result<()>
    where F: FnMut(Context) -> io::Result<usize>
{

    let n = output.len();
    let mut i = 0;
    for (sym,d) in next.iter().enumerate() {
        if *d < n {
            let mut j = i;
            while j>0 && next[mtf.symbols[j-1] as usize] > *d {
                mtf.symbols[j] = mtf.symbols[j-1];
                j -= 1;
            }
            mtf.symbols[j] = sym as Symbol;
            i += 1;
        }
    }
    if i<=1 {
        // redundant alphabet case
        let sym = mtf.symbols[0];
        for out in output.iter_mut()    {
            *out = sym;
        }
        return Ok(())
    }

    let alphabet_size = i;
    let mut ranks = [0 as Rank; TOTAL_SYMBOLS];
    for rank in 0..i {
        let sym = mtf.symbols[rank];
        debug!("\tRegistering symbol {} of rank {} at position {}",
            sym, rank, next[sym as usize]);
        ranks[sym as usize] = 0; //could use 'rank' but don't know how to derive it during encoding
    }

    i = 0;
    while i<n {
        let sym = mtf.symbols[0];
        let stop = next[mtf.symbols[1] as usize];
        debug!("\tFilling region [{}-{}) with symbol {}", i, stop, sym);
        while i<stop    {
            output[i] = sym;
            i += 1;
        }
        let ctx = Context::new(sym, ranks[sym as usize], n+1-i);
        let future = match fn_dist(ctx) {
            Ok(d) => stop + d,
            Err(e) => return Err(e)
        };
        debug!("\t\tLooking for future position {}", future);
        assert!(future <= n);
        let mut rank = 1;
        while rank < alphabet_size && future+rank > next[mtf.symbols[rank] as usize] {
            mtf.symbols[rank-1] = mtf.symbols[rank];
            rank += 1;
        }
        if rank < alphabet_size {
            debug!("\t\tFound sym {} of rank {} at position {}", mtf.symbols[rank],
                rank, next[mtf.symbols[rank] as usize]);
        }else {
            debug!("\t\tNot found");
        }
        mtf.symbols[rank-1] = sym;
        debug!("\t\tAssigning future pos {} for symbol {}", future+rank-1, sym);
        next[sym as usize] = future+rank-1;
        ranks[sym as usize] = (rank-1) as Rank;
    }
    assert_eq!(next.iter().position(|&d| d<n || d>=n+alphabet_size), None);
    assert_eq!(i, n);
    Ok(())
}

/// Decode version with "batteries included" for quick testing
pub fn decode_simple<D: ToPrimitive>(n: usize, distances: &[D]) -> Vec<Symbol> {
    let mut output: Vec<Symbol> = repeat(0 as Symbol).take(n).collect();
    let mut init = [0; TOTAL_SYMBOLS];
    for i in 0..TOTAL_SYMBOLS {
        init[i] = distances[i].to_usize().unwrap();
    }
    let mut di = TOTAL_SYMBOLS;
    decode(init, &mut output[..], &mut MTF::new(), |_ctx| {
        di += 1;
        if di > distances.len() {
            Err(io::Error::new(io::ErrorKind::Other, "Unexpected end of file"))
        } else {
            Ok(distances[di-1].to_usize().unwrap())
        }
    }).unwrap();
    output.into_iter().collect()
}


#[cfg(test)]
mod test {
    use std::iter::repeat;

    fn roundtrip(bytes: &[u8]) {
        info!("Roundtrip DC of size {}", bytes.len());
        let distances = super::encode_simple::<usize>(bytes);
        debug!("Roundtrip DC input: {:?}, distances: {:?}", bytes, distances);
        let decoded = super::decode_simple(bytes.len(), &distances[..]);
        assert_eq!(&decoded[..], bytes);
    }

    /// rountrip version that compares the coding contexts on the way
    fn roundtrip_ctx(bytes: &[u8]) {
        let n = bytes.len();
        info!("Roundtrip DC context of size {}", n);
        let mut mtf = super::super::mtf::MTF::new();
        let mut raw_dist: Vec<u16> = repeat(0).take(n).collect();
        let eniter = super::encode(bytes, &mut raw_dist[..], &mut mtf);
        let mut init = [0; super::TOTAL_SYMBOLS];
        for i in 0..super::TOTAL_SYMBOLS {
            init[i] = eniter.get_init()[i];
        }
        // implicit iterator copies, or we can gather in one pass and then split
        let (distances, contexts): (Vec<_>, Vec<_>) = eniter.unzip();
        let mut output: Vec<u8> = repeat(0).take(n).collect();
        let mut di = 0;
        super::decode(init, &mut output[..], &mut mtf, |ctx| {
            assert_eq!(contexts[di], ctx);
            di += 1;
            Ok(distances[di-1] as usize)
        }).unwrap();
        assert_eq!(di, distances.len());
        assert_eq!(&output[..], bytes);
    }

    #[test]
    fn roundtrips() {
        roundtrip(b"teeesst_dc");
        roundtrip(b"");
        roundtrip(include_bytes!("../data/test.txt"));
    }

    #[test]
    fn roundtrips_context() {
        roundtrip_ctx(b"teeesst_dc");
        roundtrip_ctx(b"../data/test.txt");
    }
}
