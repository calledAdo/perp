use super::calc_lib::_percentage;
use super::constants::*;

pub fn _def_max_tick(current_tick: u64, buy: bool) -> u64 {
    if buy {
        current_tick + (_percentage(5 * _ONE_PERCENT, u128::from(current_tick)) as u64)
    } else {
        current_tick - (_percentage(5 * _ONE_PERCENT, u128::from(current_tick)) as u64)
    }
}

pub fn _next_default_tick(multiplier: u64, buy: bool) -> u64 {
    if buy {
        _tick_zero(multiplier + 1)
    } else {
        _tick_zero(multiplier - 1)
    }
}

pub fn _tick_zero(multiplier: u64) -> u64 {
    multiplier * _ONE_PERCENT
}

pub fn _mul_and_bit(tick: u64) -> (u64, u64) {
    let base = _ONE_PERCENT;
    let multiplier = tick / base;
    let bit_position = (tick % base) / (_ONE_BASIS_POINT);
    return (multiplier, bit_position);
}

pub fn _exceeded_stopping_tick(current_tick: u64, stopping_tick: u64, buy: bool) -> bool {
    match buy {
        true => return current_tick > stopping_tick,
        false => return current_tick < stopping_tick,
    }
}

pub fn _tick_to_price(tick: u64) -> u128 {
    return (u128::from(tick) * u128::from(_BASE_PRICE)) / u128::from(100 * _ONE_PERCENT);
}
