/*!

Binary models for the arithmetic coder.
The simplicity of the domain allows for normalized updates in place using bit shifts.

# Links

# Example

# Credit

*/

use super::Border;

/// A binary value frequency model
pub struct Model {
    /// frequency of bit 0
    zero: Border,
    /// total frequency (constant)
    total: Border,
    /// learning rate
    pub rate: Border,
}

impl Model {
    /// Create a new flat (50/50 probability) instance
    pub fn new_flat(threshold: Border, rate: Border) -> Model {
        Model {
            zero: threshold>>1,
            total: threshold,
            rate: rate,
        }
    }

    /// Create a new instance with a given percentage for zeroes
    pub fn new_custom(zero_percent: u8, threshold: Border, rate: Border) -> Model {
        assert!(threshold >= 100);
        Model {
            zero: (zero_percent as Border)*threshold/100,
            total: threshold,
            rate: rate,
        }
    }

    /// Reset the model to 50/50 distribution
    pub fn reset_flat(&mut self) {
        self.zero = self.total>>1;
    }

    /// Return the probability of 0
    pub fn get_probability_zero(&self) -> Border {
        self.zero
    }

    /// Return the probability of 1
    pub fn get_probability_one(&self) -> Border {
        self.total - self.zero
    }

    /// Update the frequency of zero
    pub fn update_zero(&mut self) {
        debug!("\tUpdating zero");
        self.zero += (self.total-self.zero) >> (self.rate as usize);
    }

    /// Update the frequency of one
    pub fn update_one(&mut self) {
        debug!("\tUpdating one");
        self.zero -= self.zero >> (self.rate as usize);
    }

    /// Update frequencies in favor of given 'value'
    /// Lower factors produce more aggressive updates
    pub fn update(&mut self, value: bool) {
        if value {
            self.update_one()
        }else {
            self.update_zero()
        }
    }
}

impl super::Model<bool> for Model {
    fn get_range(&self, value: bool) -> (Border,Border) {
        if value {
            (self.zero, self.total)
        }else {
            (0, self.zero)
        }
    }

    fn find_value(&self, offset: Border) -> (bool,Border,Border) {
        assert!(offset < self.total,
            "Invalid frequency offset {} requested under total {}",
            offset, self.total);
        if offset < self.zero {
            (false, 0, self.zero)
        }else {
            (true, self.zero, self.total)
        }
    }

    fn get_denominator(&self) -> Border {
        self.total
    }
}


/// A proxy model for the combination of two binary models
/// using equation: (wa * A + wb * B) >> ws
pub struct SumProxy<'a> {
    first: &'a Model,
    second: &'a Model,
    w_first: Border,
    w_second: Border,
    w_shift: Border,
}

impl<'a> SumProxy<'a> {
    /// Create a new instance of the binary sum proxy
    pub fn new(wa: Border, first: &'a Model, wb: Border, second: &'a Model, shift: Border) -> SumProxy<'a> {
        SumProxy {
            first: first,
            second: second,
            w_first: wa,
            w_second: wb,
            w_shift: shift,
        }
    }

    fn get_probability_zero(&self) -> Border {
        (self.w_first * self.first.get_probability_zero() +
            self.w_second * self.second.get_probability_zero()) >>
            (self.w_shift as usize)
    }
}

impl<'a> super::Model<bool> for SumProxy<'a> {
    fn get_range(&self, value: bool) -> (Border,Border) {
        let zero = self.get_probability_zero();
        if value {
            (zero, self.get_denominator())
        }else {
            (0, zero)
        }
    }

    fn find_value(&self, offset: Border) -> (bool,Border,Border) {
        let zero = self.get_probability_zero();
        let total = self.get_denominator();
        assert!(offset < total,
            "Invalid frequency offset {} requested under total {}",
            offset, total);
        if offset < zero {
            (false, 0, zero)
        }else {
            (true, zero, total)
        }
    }

    fn get_denominator(&self) -> Border {
        (self.w_first * self.first.get_denominator() +
            self.w_second * self.second.get_denominator()) >>
            (self.w_shift as usize)
    }
}
