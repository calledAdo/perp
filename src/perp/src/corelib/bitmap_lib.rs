use super::bit_lib::{_least_sigbit_position, _most_sigbit_position};

use super::constants::{_ONE_BASIS_POINT, _ONE_PERCENT};

pub fn _flip_bit(bitmap: u128, bit_position: u64) -> u128 {
    if bit_position == 0 {
        return bitmap;
    }
    let mask = 1 << (99 - bit_position);
    return bitmap ^ mask;
}

pub fn _next_initialised_tick(bitmap: u128, multiplier: u64, bit_position: u64, in1: bool) -> u64 {
    let refrence = 99 - bit_position;
    if in1 {
        let mask = ((1 as u128) << refrence) - 1;
        let masked = bitmap & mask;
        if masked == 0 {
            return (multiplier + 1) * _ONE_PERCENT;
        } else {
            return multiplier * _ONE_PERCENT + _most_sigbit_position(masked) * _ONE_BASIS_POINT;
        }
    } else {
        let mask = !(((1 as u128) << (refrence + 1)) - 1);
        let masked = mask & bitmap;
        if masked == 0 {
            return (multiplier - 1) * _ONE_PERCENT;
        } else {
            return (multiplier * _ONE_PERCENT)
                + (_least_sigbit_position(masked) * _ONE_BASIS_POINT);
        }
    }
}
