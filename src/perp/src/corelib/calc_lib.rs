use ic_cdk::api::time;

use super::constants::*;

type Amount = u128;

pub fn _calc_interest(debt: Amount, interest_rate: u32, start_time: u64) -> Amount {
    let mut fee: Amount = 0;

    let one_hour: u64 = 3600 * ((10 as u64).pow(9));

    let mut _starting_time = start_time;

    let current_time = time();

    while start_time < current_time {
        fee += ((interest_rate as u128) * debt) / u128::from(100 * _ONE_PERCENT);

        _starting_time += one_hour;
    }

    return fee;
}

pub fn _calc_shares(
    amount_in: Amount,
    init_total_shares: Amount,
    init_liquidity: Amount,
) -> Amount {
    if init_liquidity == 0 {
        return amount_in;
    }
    return (amount_in * init_total_shares) / init_liquidity;
}

pub fn _calc_shares_value(
    shares: Amount,
    init_total_shares: Amount,
    init_liquidity: Amount,
) -> Amount {
    return (shares * init_liquidity) / init_total_shares;
}

pub fn _percentage(x: u64, amount: Amount) -> Amount {
    return ((x as u128) * amount) / u128::from(100 * _ONE_PERCENT);
}
