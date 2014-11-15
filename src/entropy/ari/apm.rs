/*!

Adaptive Probability Models

# Links
* http://mattmahoney.net/dc/bbb.cpp
* https://github.com/IlyaGrebnov/libbsc

# Example

# Credit
Matt Mahoney for the wonderful 'bbb' commented source

*/

use std::num::Float;
use super::Border;
pub type FlatProbability = u16;
pub type WideProbability = i16;

const BIN_WEIGHT_BITS: uint = 8;
const BIN_WEIGHT_TOTAL: uint = 1<<BIN_WEIGHT_BITS;
const FLAT_BITS: FlatProbability = 12;
const FLAT_TOTAL: int = 1<<(FLAT_BITS as uint);
const WIDE_BITS: uint = 12;
const WIDE_OFFSET: WideProbability = 1<<(WIDE_BITS-1);
//const WIDE_TOTAL: int = (1<<WIDE_BITS)+1;
const PORTAL_OFFSET: uint = 1<<(WIDE_BITS-BIN_WEIGHT_BITS-1);
const PORTAL_BINS: uint = 2*PORTAL_OFFSET + 1;


/// Bit probability model
pub struct Bit(FlatProbability);

impl Bit {
    /// Return an equal 0-1 probability
    #[inline]
    pub fn new_equal() -> Bit {
        Bit(FLAT_TOTAL as FlatProbability >> 1)
    }

    /// Return flat probability
    #[inline]
    pub fn to_flat(&self) -> FlatProbability {
        let Bit(fp) = *self;
        fp
    }

    /// Return wide probability
    #[inline]
    pub fn to_wide(&self) -> WideProbability {
        //table_stretch[self.to_flat() as uint]
        let p = (self.to_flat() as f32) / (FLAT_TOTAL as f32);
        let d = (p / (1.0-p)).ln();
        let wp = (d * WIDE_OFFSET as f32).to_i16().unwrap();
        wp
    }

    /// Construct from flat probability
    #[inline]
    pub fn from_flat(fp: FlatProbability) -> Bit {
        Bit(fp)
    }

    /// Construct from wide probability
    #[inline]
    pub fn from_wide(wp: WideProbability) -> Bit {
        //Bit(table_squash[(wp+WIDE_OFFSET) as uint])
        let d = (wp as f32) / (WIDE_OFFSET as f32);
        let p = 1.0 / (1.0 + (-d).exp());
        let fp = (p * FLAT_TOTAL as f32).to_u16().unwrap();
        Bit(fp)
    }

    /// Mutate for better zeroes
    pub fn update_zero(&mut self, rate: int, bias: int) {
        let &Bit(ref mut fp) = self;
        let one = FLAT_TOTAL - bias - (*fp as int);
        *fp += (one >> (rate as uint)) as FlatProbability;
    }

    /// Mutate for better ones
    pub fn update_one(&mut self, rate: int, bias: int) {
        let &Bit(ref mut fp) = self;
        let zero = (*fp as int) - bias;
        *fp -= (zero >> (rate as uint)) as FlatProbability;
    }

    /// Mutate for a given value
    #[inline]
    pub fn update(&mut self, value: bool, rate: int, bias: int) {
        if !value {
            self.update_zero(rate, bias)
        }else {
            self.update_one(rate, bias)
        }
    }
}

impl super::Model<bool> for Bit {
    fn get_range(&self, value: bool) -> (Border,Border) {
        let fp = self.to_flat() as Border;
        if !value {
            (0, fp)
        }else {
            (fp, FLAT_TOTAL as Border)
        }
    }

    fn find_value(&self, offset: Border) -> (bool,Border,Border) {
        assert!(offset < FLAT_TOTAL as Border,
            "Invalid bit offset {} requested", offset);
        let fp = self.to_flat() as Border;
        if offset < fp {
            (false, 0, fp)
        }else {
            (true, fp, FLAT_TOTAL as Border)
        }
    }

    fn get_denominator(&self) -> Border {
        FLAT_TOTAL as Border
    }
}


/// Binary context gate
/// maps an input binary probability into a new one
/// by interpolating between internal maps in non-linear space
pub struct Gate {
    map: [Bit, ..PORTAL_BINS],
}

pub type BinCoords = (uint, uint); // (index, weight)

impl Gate {
    /// Create a new gate instance
    pub fn new() -> Gate {
        let mut g = Gate {
            map: [Bit::new_equal(), ..PORTAL_BINS],
        };
        for (i,bit) in g.map.iter_mut().enumerate() {
            let rp = (i as f32)/(PORTAL_OFFSET as f32) - 1.0;
            let wp = (rp * (WIDE_OFFSET as f32)).to_i16().unwrap();
            *bit = Bit::from_wide(wp);
        }
        g
    }

    /// Pass a bit through the gate
    #[inline]
    pub fn pass(&self, bit: &Bit) -> (Bit, BinCoords) {
        let (fp, index) = self.pass_wide(bit.to_wide());
        (Bit::from_flat(fp), index)
    }

    /// Pass a wide probability on input, usable when
    /// you mix it linearly beforehand (libbsc does that)
    pub fn pass_wide(&self, wp: WideProbability) -> (FlatProbability, BinCoords) {
        let index = ((wp + WIDE_OFFSET) >> BIN_WEIGHT_BITS) as uint;
        let weight = wp as uint & (BIN_WEIGHT_TOTAL-1);
        let z = [
            self.map[index+0].to_flat() as uint,
            self.map[index+1].to_flat() as uint];
        let sum = z[0]*(BIN_WEIGHT_TOTAL-weight) + z[1]*weight;
        let fp = (sum >> BIN_WEIGHT_BITS) as FlatProbability;
        (fp, (index, weight))
    }

    //TODO: weight update ratio & bias as well

    /// Mutate for better zeroes
    pub fn update_zero(&mut self, bc: BinCoords, rate: int, bias: int) {
        let (index, _) = bc;
        self.map[index+0].update_zero(rate, bias);
        self.map[index+1].update_zero(rate, bias);
    }

    /// Mutate for better ones
    pub fn update_one(&mut self, bc: BinCoords, rate: int, bias: int) {
        let (index, _) = bc;
        self.map[index+0].update_one(rate, bias);
        self.map[index+1].update_one(rate, bias);
    }

    /// Mutate for a given value
    #[inline]
    pub fn update(&mut self, value: bool, bc: BinCoords, rate: int, bias: int) {
        if !value {
            self.update_zero(bc, rate, bias)
        }else {
            self.update_one(bc, rate, bias)
        }
    }
}
