use super::bitmap_lib::{_flip_bit, _next_initialised_tick};
use super::price_lib::_equivalent;
use super::tick_lib::*;
use crate::types::TickDetails;

use std::collections::HashMap;

//use ic_cdk::api::time;

type Tick = u64;
type Amount = u128;
type MB = HashMap<u64, u128>;
type TD = HashMap<u64, TickDetails>;

struct SwapTickConstants {
    current_tick: Tick,
    order_size: Amount,
}

/// SwapParams for initiating  a swap
/// utilsed for opening position at market price
pub struct SwapParams<'a> {
    /// Swap Direction
    ///
    /// true if buying or false if selling
    pub buy: bool,
    /// Init Tick
    ///
    /// the current state tick of the market ,also seen as current price
    ///
    /// this also translates to the current price
    pub init_tick: Tick,
    ///Stopping Tick
    ///
    /// stopping tick at which swapping should not not exceed
    ///
    /// This can be viewed as maximum excecution price for a market order
    /// if specified swap does not exceed this and returns the net previous amount from ticks below and the amount remaining
    pub stopping_tick: Tick,
    /// Order Size
    ///
    /// the amount of asset being swapped
    pub order_size: Amount,
    /// Multiplier BitMaps
    ///
    /// HashMap  of multipliers to their bitmaps
    pub multipliers_bitmaps: &'a mut MB,
    /// Ticks Details
    ///
    /// HashMasp  of ticks to their  respective tick_details
    pub ticks_details: &'a mut TD,
}

impl<'a> SwapParams<'a> {
    /// Swap Function
    ///
    /// Swap is executed as a loop starting at the current tick till stopping tick is reached is exceeded
    ///
    /// Returns
    ///  - AmountOut :The amount of token gotten from the swap
    ///  - AmountRemaining :The amount of asset remaining dues to swap not being completely filled before stopping tick
    ///  - Current or Resulting Tick : This corresponds to the tick at which either asset was fully swapped
    /// or tick before stopping tick was exceeded
    pub fn _swap(&mut self) -> (Amount, Amount, Tick, Vec<Tick>) {
        let mut current_tick = self.init_tick;

        let mut crossed_ticks: Vec<Tick> = Vec::new();

        let mut amount_out = 0;

        let mut amount_remaining = self.order_size;

        loop {
            let (multiplier, bit_position) = _mul_and_bit(current_tick);

            let bitmap = match self.multipliers_bitmaps.get(&multiplier) {
                Some(&res) => res,
                None => {
                    // if multiplier has no bitmap means that means  no tick within the multiplier  is
                    //initialised

                    // calculates the  next_default tick (See bitmap_lib)
                    // if next default tick exceeds stopping tick
                    //breaks else
                    // updates current tick to the next default tick

                    let next_default_tick = _next_default_tick(multiplier, self.buy);
                    if _exceeded_stopping_tick(next_default_tick, self.stopping_tick, self.buy) {
                        break;
                    };
                    current_tick = next_default_tick;
                    //stops currrent iteration,starts the next at the next default tick
                    continue;
                }
            };

            let tick_params = SwapTickConstants {
                order_size: amount_remaining,
                current_tick,
            };

            let (_value_out, _cleared, boundary_closed);

            if self.buy {
                (_value_out, amount_remaining, _cleared, boundary_closed) =
                    self._buy_at_tick(tick_params);
            } else {
                (_value_out, amount_remaining, _cleared, boundary_closed) =
                    self._sell_at_tick(tick_params);
            }

            amount_out += _value_out;

            // if static liquidity was exhausted at that tick
            if boundary_closed {
                //add ticks to list of crossed ticks
                crossed_ticks.push(current_tick);

                // if all liquidity is cleared in tick ,delete tick details and flip bit in bitmap
                if _cleared {
                    self.ticks_details.remove(&current_tick);

                    let flipped_bitmap = _flip_bit(bitmap, bit_position);

                    let tick_zero = _tick_zero(multiplier);
                    // if flipping bitmap results in zero and tick zero(see bitmap_lib) is not contained in ticks_details hashmap
                    //delete btimap
                    if flipped_bitmap == 0 && !self.ticks_details.contains_key(&tick_zero) {
                        self.multipliers_bitmaps.remove(&multiplier);
                    } else {
                        // insert flipped bitmap
                        self.multipliers_bitmaps.insert(multiplier, flipped_bitmap);
                    };
                }
            }

            if amount_remaining == 0 {
                break;
            }

            let next_initialised_tick =
                _next_initialised_tick(bitmap, multiplier, bit_position, self.buy);

            if _exceeded_stopping_tick(next_initialised_tick, self.stopping_tick, self.buy) {
                break;
            };

            current_tick = next_initialised_tick;
        }
        return (amount_out, amount_remaining, current_tick, crossed_ticks);
    }

    /// buy at tick function
    ///
    /// Performs a swap at a particular tick
    ///
    /// Returns
    /// - AmountOut :The amount resulting from the swap for a buy order at that tick
    /// - AmountRemaining :The amount remaining from swapping at that tick ,
    ///  this is  zero if the swap is completedly fully at tick
    /// - Cleared : true if all liquidity at tick  was cleared
    /// - Boundary Closed : true if all static liquidity at tick (see TickDetails and LiquidityBoundary) is cleared
    fn _buy_at_tick(&mut self, tick_params: SwapTickConstants) -> (Amount, Amount, bool, bool) {
        let mut amount_out = 0;

        let mut amount_remaining = tick_params.order_size;

        let mut cleared = false;

        let mut boundary_closed = false;

        let tick_price = _tick_to_price(tick_params.current_tick);

        let equivalent =
            |amount: Amount, buy: bool| -> Amount { _equivalent(amount, tick_price, buy) };

        let tick_details = match self.ticks_details.get_mut(&tick_params.current_tick) {
            Some(res) => res,
            None => return (amount_out, amount_remaining, cleared, false),
        };

        // when buying all liquidity is in token0
        let init_tick_liq = tick_details.liq_token0;

        let static_liq = tick_details.liq_bounds_token0._liquidity_within();

        //value of all_liquidity in token1
        let init_liq_equivalent = equivalent(init_tick_liq, false);

        // value of static_liquidity in token1
        let static_liq_equivalent = equivalent(static_liq, false);

        if init_liq_equivalent <= self.order_size {
            // all liquidity has been exhausted
            amount_out = init_tick_liq;

            amount_remaining -= init_liq_equivalent;

            //update tick
            tick_details.liq_token0 = 0;

            tick_details.liq_token1 += init_liq_equivalent - static_liq_equivalent;
        } else {
            //liquidity remains
            amount_out = equivalent(self.order_size, true);

            amount_remaining = 0;

            tick_details.liq_token0 = init_tick_liq - amount_out; // tick_details -= amount_out

            if self.order_size >= static_liq_equivalent {
                tick_details.liq_token1 += self.order_size - static_liq_equivalent
            }
        }

        // reduce static liquidity by amount out
        tick_details.liq_bounds_token0._reduce_boundary(amount_out);

        // if all static liquidity in tick is zero

        if tick_details.liq_bounds_token0._liquidity_within() == 0
            && tick_details.liq_bounds_token1._liquidity_within() == 0
        {
            tick_details.crossed_time += 5000; //change to time

            //boundary has been closed
            boundary_closed = true;
            // tick is cleared when all liquidity is zero
            cleared = tick_details.liq_token0 == 0 && tick_details.liq_token1 == 0;
        };

        return (amount_out, amount_remaining, cleared, boundary_closed);
    }

    /// Sell at tick function
    ///
    /// Performs a swap at a particular tick
    ///
    /// Returns
    /// - AmountOut :The amount resulting from the swap for a sell order at that tick
    /// - AmountRemaining :The amount remaining from swapping at that tick ,
    ///  this is  zero if the swap is completedly fully at tick
    /// - Cleared : true if all liquidity at tick  was cleared
    /// - Boundary Closed : true if all static liquidity at tick (see TickDetails and LiquidityBoundary) is cleared

    fn _sell_at_tick(&mut self, tick_params: SwapTickConstants) -> (Amount, Amount, bool, bool) {
        let mut amount_out = 0;

        let mut amount_remaining = tick_params.order_size;

        let mut cleared = false;

        let mut boundary_closed = false;

        let tick_price = _tick_to_price(tick_params.current_tick);

        let equivalent =
            |amount: Amount, buy: bool| -> Amount { _equivalent(amount, tick_price, buy) };

        // tick details
        let tick_details = match self.ticks_details.get_mut(&tick_params.current_tick) {
            Some(res) => res,
            None => return (amount_out, amount_remaining, cleared, boundary_closed),
        };

        let init_tick_liq = tick_details.liq_token1;

        let static_liq = tick_details.liq_bounds_token1._liquidity_within();

        let init_liq_equivalent = equivalent(init_tick_liq, true);

        let static_liq_equivalent = equivalent(static_liq, true);

        if init_liq_equivalent <= self.order_size {
            amount_out = init_tick_liq;

            amount_remaining -= init_liq_equivalent;

            // updates ticks details
            tick_details.liq_token1 = 0;
            tick_details.liq_token0 += init_liq_equivalent - static_liq_equivalent;
        } else {
            //liquidity remains
            amount_out = equivalent(self.order_size, false);

            amount_remaining = 0;

            tick_details.liq_token1 = init_tick_liq - amount_out;

            if self.order_size >= static_liq_equivalent {
                tick_details.liq_token0 += self.order_size - static_liq_equivalent
            }
        }

        tick_details.liq_bounds_token1._reduce_boundary(amount_out);

        if tick_details.liq_bounds_token1._liquidity_within() == 0
            && tick_details.liq_bounds_token0._liquidity_within() == 0
        {
            tick_details.crossed_time += 2000; //change to time

            boundary_closed = true;

            cleared = tick_details.liq_token0 == 0 && tick_details.liq_token1 == 0;
        }

        return (amount_out, amount_remaining, cleared, boundary_closed);
    }
}

#[cfg(test)]
mod test {

    use crate::types::LiquidityBoundary;

    use super::*;
    use super::{_mul_and_bit, _tick_zero};
    use std::cell::RefCell;
    use std::collections::HashMap;

    thread_local! {
        static MBS:RefCell<MB> = RefCell::new(HashMap::new());
        static TDS:RefCell<TD> = RefCell::new(HashMap::new());
    }

    #[test]
    fn testing_excceded() {
        MBS.with(|mbs| {
            TDS.with(|tds| {
                let order_size = 10000;
                let init_tick = _tick_zero(3000);
                let mut params = SwapParams {
                    buy: true,
                    init_tick,
                    stopping_tick: _tick_zero(200),
                    order_size,
                    multipliers_bitmaps: &mut mbs.borrow_mut(),
                    ticks_details: &mut tds.borrow_mut(),
                };

                //test exceeded
                // buy order
                let result = params._swap();
                // all amount should be sent
                assert_eq!(order_size, result.1);

                //sell order
                params.buy = false;
                params.stopping_tick = _tick_zero(20000);

                let result2 = params._swap();

                assert_eq!(order_size, result2.1);
            })
        })
    }

    #[test]
    fn test_bitmap_not_found() {
        MBS.with(|mbs| {
            TDS.with(|tds| {
                let order_size = 200000;
                let init_tick = _tick_zero(300);

                let stopping_tick = _def_max_tick(init_tick, true);

                let mut swap_params = SwapParams {
                    order_size,
                    buy: true,
                    init_tick,
                    stopping_tick,
                    multipliers_bitmaps: &mut mbs.borrow_mut(),
                    ticks_details: &mut tds.borrow_mut(),
                };

                let result = swap_params._swap();

                println!(
                    "The current tick is {} and max tick is {} ",
                    result.2, stopping_tick
                );

                assert_eq!(stopping_tick >= result.2, true);

                // if no btmap was found swap is not executed
                assert_eq!(order_size, result.1);
            })
        })
    }

    fn _fill_tick(tick: Tick, details: TickDetails) {
        MBS.with(|mbs| {
            TDS.with(|tds| {
                let (mul, bit) = _mul_and_bit(tick);
                mbs.borrow_mut().insert(mul, _flip_bit(0, bit));

                tds.borrow_mut().insert(tick, details);
            })
        })
    }
    #[test]
    fn test_swap_at_tick_liquidity_order() {
        MBS.with(|mbs| {
            TDS.with(|tds| {
                let liquidity = 100_000_000_000;
                let current_tick = 199 * 100_000;
                let stopping_tick = _def_max_tick(current_tick, true);
                let tick = 200 * 100_000;

                let tick_details;

                tick_details = TickDetails {
                    liq_bounds_token0: LiquidityBoundary::default(),
                    liq_token0: liquidity,
                    liq_token1: 0,
                    total_shares: liquidity,
                    liq_bounds_token1: LiquidityBoundary::default(),
                    crossed_time: 500000000000,
                };

                _fill_tick(tick, tick_details);
                // Test Swap when all liquidity is dynamic
                {
                    let multipliers_bitmaps = &mut *mbs.borrow_mut();

                    let ticks_details = &mut *tds.borrow_mut();

                    //lesser value
                    let order_size = 1_000_000;

                    let mut swap_params = SwapParams {
                        buy: true,
                        init_tick: current_tick,
                        stopping_tick,
                        order_size,
                        multipliers_bitmaps,
                        ticks_details,
                    };

                    let result = swap_params._swap();

                    assert_eq!(result.0, order_size / 2);
                    // amount remaining is zero
                    assert_eq!(result.1, 0);
                    // tick is the current tick since its liquidty was not exhausted
                    assert_eq!(result.2, tick);

                    let new_ticks_details = ticks_details.get(&tick).unwrap();

                    assert_eq!(new_ticks_details.liq_token1, order_size);
                }

                // Test Exhausting all liquidity
                {
                    let multipliers_bitmaps = &mut *mbs.borrow_mut();

                    let ticks_details = &mut *tds.borrow_mut();

                    let previous_ticks_details = ticks_details.get(&tick).unwrap();

                    let prev_liqiudity = previous_ticks_details.liq_token0;

                    // much bigger amount
                    let order_size = 1_000_000_000_000_000_000_000;

                    let mut swap_params = SwapParams {
                        buy: true,
                        init_tick: current_tick,
                        stopping_tick,
                        order_size,
                        multipliers_bitmaps,
                        ticks_details,
                    };

                    let result = swap_params._swap();

                    assert_eq!(result.0, prev_liqiudity);

                    let new_tick_details = ticks_details.get(&tick).unwrap();

                    //all liquidity was cleared
                    assert_eq!(new_tick_details.liq_token0, 0);

                    println!(
                        "the value is {} while amount remaining is",
                        new_tick_details.liq_token1
                    );
                    assert_eq!(
                        new_tick_details.liq_token1,
                        1_000_000 + order_size - result.1
                    );
                }
            })
        })
    }

    #[test]
    fn test_swap_at_tick_trade_order() {
        MBS.with(|mbs| {
            TDS.with(|tds| {
                let liquidity = 100_000_000_000;
                let current_tick = 199 * 100_000;
                let stopping_tick = _def_max_tick(current_tick, true);
                let tick = 200 * 100_000;
                // try liquidity_swap
                let tick_details = TickDetails {
                    liq_bounds_token0: LiquidityBoundary {
                        removed_liquidity: 0,
                        upper_bound: liquidity,
                        lower_bound: 0,
                    },
                    liq_token0: liquidity,
                    liq_token1: 0,
                    total_shares: 0,
                    liq_bounds_token1: LiquidityBoundary::default(),
                    crossed_time: 500000000000,
                };

                _fill_tick(tick, tick_details);

                let order_size = 1_000_000;

                let multipliers_bitmaps = &mut mbs.borrow_mut();

                let ticks_details = &mut tds.borrow_mut();

                let mut swap_params = SwapParams {
                    buy: true,
                    init_tick: current_tick,
                    stopping_tick,
                    order_size,
                    multipliers_bitmaps,
                    ticks_details,
                };
                //perform swap
                let result = swap_params._swap();

                let new_tick_details = ticks_details.get(&tick).unwrap();

                // all amount in is removed by static liqqudity at tick
                assert_eq!(new_tick_details.liq_token1, 0);

                // liquidity within static boundary is reduced by increasing by factor amount out
                assert_eq!(result.0, new_tick_details.liq_bounds_token0.lower_bound);
            })
        })
    }
}
