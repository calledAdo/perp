use std::u128;

pub fn _most_sigbit_position(num: u128) -> u64 {
    return (num.leading_zeros() as u64) - 28;
}

pub fn _least_sigbit_position(num: u128) -> u64 {
    return 99 - (num.trailing_zeros() as u64);
}
