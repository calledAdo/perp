use super::constants::_PRICE_DECIMAL;

type Amount = u128;

pub fn _equivalent(amount: Amount, price: Amount, buy: bool) -> Amount {
    if buy {
        let num = amount * u128::from(_PRICE_DECIMAL);
        let den = price;
        return num / den;
    } else {
        let num = amount * price;
        let den = u128::from(_PRICE_DECIMAL);

        return num / den;
    }
}
