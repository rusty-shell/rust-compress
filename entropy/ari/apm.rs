/*!

Adaptive Probability Models

# Links
* http://mattmahoney.net/dc/bbb.cpp
* https://github.com/IlyaGrebnov/libbsc

# Example

# Credit
Matt Mahoney for the wonderful 'bbb' commented source

*/

pub type FlatProbability  = u16;
pub type WideProbability    = i16;

static BIN_WEIGHT_BITS: uint = 8;
static BIN_WEIGHT_TOTAL: uint = 1<<BIN_WEIGHT_BITS;
static FLAT_BITS: FlatProbability = 12;
static FLAT_TOTAL: int = (1<<FLAT_BITS)+1;
static WIDE_BITS: uint = 12;
static WIDE_OFFSET: WideProbability = 1<<(WIDE_BITS-1);
//static WIDE_TOTAL: int = (1<<WIDE_BITS)+1;
static PORTAL_BINS: uint = (1<<(WIDE_BITS-BIN_WEIGHT_BITS))+1;


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
        let wp = (d * WIDE_OFFSET as f32).to_uint().unwrap();
        wp as WideProbability
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
        let fp = (p * (FLAT_TOTAL-1) as f32).to_uint().unwrap();
        Bit(fp as FlatProbability)
    }
    
    /// Mutate for better zeroes
    pub fn update_zero(&mut self, rate: int, bias: int) {
        let &Bit(ref mut fp) = self;
        let one = FLAT_TOTAL - 1 - bias - (*fp as int);
        let add = (one * rate) >> FLAT_BITS;
        *fp += add as FlatProbability;
    }
    
    /// Mutate for better ones
    pub fn update_one(&mut self, rate: int, bias: int) {
        let &Bit(ref mut fp) = self;
        let zero = (*fp as int) - bias;
        let sub = (zero * rate) >> FLAT_BITS;
        *fp -= sub as FlatProbability;
    }
}


/// Binary context gate
/// maps an input probability into a new one
/// by interpolating between internal maps in non-linear space
pub struct Gate {
    map: [Bit, ..PORTAL_BINS],
}

pub type BinCoords = (uint, uint); // (index, weight)

impl Gate {
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
}
