use serde::Deserialize;

use super::bitmap_lib::_flip_bit;
use super::calc_lib::{_calc_shares, _calc_shares_value};
use super::price_lib::_equivalent;
use super::tick_lib::{_mul_and_bit, _tick_to_price};

use candid::CandidType;

use std::collections::HashMap;

use crate::types::TickDetails;

type Time = u64;
type Amount = u128;
type Tick = u64;
type MB = HashMap<u64, u128>;
type TD = HashMap<Tick, TickDetails>;

/// Order Trait for different OrderTypes
pub trait Order {
    fn _opening_update(&mut self, ref_tick_details: &mut TickDetails);
    fn _closing_update(&self, ref_tick_details: &mut TickDetails) -> (Amount, Amount);
}

///OpenOrderParams for creating orders
#[derive(CandidType)]
pub struct OpenOrderParams<'a, V>
where
    V: Order,
{
    ///Reference tick
    ///
    /// reference tick for opening order
    pub reference_tick: Tick,
    /// Multiplier BitMaps
    ///
    ///A HashMap of  multipliers (percentiles) to their respective bitmap
    pub multipliers_bitmaps: &'a mut MB,
    ///Ticks Details
    ///
    ///A HashMap of tick to their tick_details
    pub ticks_details: &'a mut TD,
    /// Order
    ///
    /// A mutable refrence to any generic type that implements the Order trait  determing which order type is being opened
    pub order: &'a mut V,
}

impl<'a, V> OpenOrderParams<'a, V>
where
    V: Order,
{
    /// Open Order function
    ///
    /// creates an order at a particular tick
    pub fn open_order(&mut self) {
        let tick_details = self
            .ticks_details
            .entry(self.reference_tick)
            .or_insert_with(|| {
                //  flip bitmap

                let (multiplier, bit_position) = _mul_and_bit(self.reference_tick);

                let map = self.multipliers_bitmaps.entry(multiplier).or_insert(0);
                *map = _flip_bit(*map, bit_position);

                // initialises it with a default value

                TickDetails::default()
            });

        self.order._opening_update(tick_details);
    }
}

///CloseOrderParams for closing order

#[derive(CandidType)]
pub struct CloseOrderParams<'a, V>
where
    V: Order,
{
    ///Order
    ///
    /// An immutable reference  to a  generic type order that implements the Order trait,
    /// determining which type of order is being closed
    pub order: &'a V,
    ///Order Size
    ///
    /// the order size
    pub order_size: Amount,

    /// Order Direction
    ///
    /// true if order was a buy or false if it was a sell
    pub order_direction: bool,
    /// Reference Tick   
    ///
    /// reference tick at which order was placed
    pub order_reference_tick: Tick,
    /// Multipliers Bitmaps
    ///
    ///A HashMap of  multipliers (percentiles) to their respective bitmap
    pub multipliers_bitmaps: &'a mut MB,
    ///Ticks Details
    ///
    ///A HashMap of tick to their tick_details
    pub ticks_details: &'a mut TD,
}

impl<'a, V> CloseOrderParams<'a, V>
where
    V: Order,
{
    /// Close_order function
    ///
    /// Returns a tuple
    ///
    /// Note
    /// - If closing a trade order ,tuple represents amount out vs amount remaining
    /// - If closing a liquidity order ,tuple represents token0 amount and token1 amount corresponding order shares(see LiquidityOrder)
    pub fn close_order(&mut self) -> (Amount, Amount) {
        match self.ticks_details.get_mut(&self.order_reference_tick) {
            Some(tick_details) => {
                // if closing a trade  order ,this returns
                // amount_out and amount remaining
                // if closing a Liquidity order
                // it returns token0 amount and token1 amount
                let (amount0, amount1) = self.order._closing_update(tick_details);

                //  if all liquidity is zero delete tick_details
                if tick_details.liq_token0 == 0 && tick_details.liq_token1 == 0 {
                    self.ticks_details.remove(&self.order_reference_tick);

                    let (multiplier, bit_position) = _mul_and_bit(self.order_reference_tick);
                    // flip bitmap

                    self.multipliers_bitmaps
                        .entry(multiplier)
                        .and_modify(|res| *res = _flip_bit(*res, bit_position));
                };
                return (amount0, amount1);
            }
            None => {
                // if tick details does not exist means ,all trade order  that currently references that tick
                //   has been filled  and
                //   all liquidity order has  been removed
                let tick_price = _tick_to_price(self.order_reference_tick);
                return (
                    _equivalent(self.order_size, tick_price, self.order_direction),
                    0,
                );
            }
        };
    }
}

/// Trade Order for placing Limit Orders
#[derive(Default, CandidType, Copy, Clone, Deserialize)]
pub struct TradeOrder {
    /// true if order is a buy order
    pub buy: bool,
    /// order_size
    pub order_size: Amount,
    /// Initial Upper Bound
    ///
    /// The initial upper bound
    /// Note
    ///  - The init upper bound is that of token1 for a buy order and token0 for a sell order
    pub init_upper_bound: Amount,
    /// Reference Tick
    ///
    /// the reference tick of the particular order
    pub ref_tick: Tick,
    /// Tick Cross Time
    ///
    /// the ticks last cross_time
    pub init_cross_time: Time,
}

impl TradeOrder {
    pub fn new(order_size: Amount, ref_tick: Tick, buy: bool) -> Self {
        return TradeOrder {
            order_size,
            ref_tick,
            init_upper_bound: 0,
            buy,
            init_cross_time: 0,
        };
    }
}

impl Order for TradeOrder {
    /// Opening Update function
    ///
    /// opens a trade order by
    ///  - Updating the reference tick details
    ///  - Updating the init_upper_bound and init_cross_time of the order
    fn _opening_update(&mut self, tick_details: &mut TickDetails) {
        let init_upper_bound = if self.buy {
            tick_details.liq_bounds_token1.upper_bound
        } else {
            tick_details.liq_bounds_token0.upper_bound
        };

        self.init_upper_bound = init_upper_bound;
        self.init_cross_time = tick_details.crossed_time;

        tick_details._add_liquidity(self.buy, self.order_size, 0, self.order_size);
    }

    /// Closing Update function
    ///
    /// closing a trade order
    ///
    /// Returns
    /// - Amount Out :This returns the amount of the particular asset expected from the order
    /// i.e base asset(perp asset) for a buy order and quote asset (collateral asset) for a sell order
    /// - Amount Remaining :This  returns the amount  not filled in the order  

    fn _closing_update(&self, tick_details: &mut TickDetails) -> (Amount, Amount) {
        let tick_price = _tick_to_price(self.ref_tick);
        let equivalent = |amount: Amount| -> Amount { _equivalent(amount, tick_price, self.buy) };

        // if ticks crossed_time is greatar than  order init_cross_time
        // it  means all trade orders placed at that tick before tick's current cross time has been filled
        if self.init_cross_time < tick_details.crossed_time {
            // the equivalent amount is sent out
            return (equivalent(self.order_size), 0);
        };

        let tick_lower_bound = if self.buy {
            tick_details.liq_bounds_token1.lower_bound
        } else {
            tick_details.liq_bounds_token0.lower_bound
        };
        //
        //
        let (amount_out, amount_remaining) = if tick_lower_bound <= self.init_upper_bound {
            //order not filled
            (0, self.order_size)
        } else if tick_lower_bound < self.init_upper_bound + self.order_size {
            // order partially filled
            (
                equivalent(tick_lower_bound - self.init_upper_bound),
                (self.init_upper_bound + self.order_size) - tick_lower_bound,
            )
        } else {
            // order fully filled
            (equivalent(self.order_size), 0)
        };

        tick_details._remove_liquidity(self.buy, amount_remaining);

        return (amount_out, amount_remaining);
    }
}

/// Liquidity order type
///
/// Utilised for opening order by liquidity providers
#[derive(Default, CandidType, Copy, Clone)]
pub struct LiquidityOrder {
    /// Order Size
    ///
    /// Amount being put in
    pub order_size: Amount,
    /// Liq Shares
    ///
    /// the measure of liquidity put into tick with respect to the amount alreaady within it
    pub liq_shares: Amount,
    /// Reference Tick
    ///
    /// the reference_tick to place the order
    pub reference_tick: Tick,
    /// Buy
    ///
    // ordr direction (true for buy or false for sell)
    pub buy: bool,
}

impl LiquidityOrder {
    pub fn new(order_size: Amount, reference_tick: Tick, buy: bool) -> LiquidityOrder {
        return LiquidityOrder {
            order_size,
            reference_tick,
            buy,
            liq_shares: 0,
        };
    }
}

impl Order for LiquidityOrder {
    /// Opening Update function
    ///
    /// opens a liquidity order by
    ///  - Updating the reference tick details
    ///  - calculating the liquidity share with respect to the dynamic liquidity (see LiquidityBoundary) at that tick
    fn _opening_update(&mut self, tick_details: &mut TickDetails) {
        let dynamic_liquidity = if self.buy {
            tick_details.liq_token1 - tick_details.liq_bounds_token1._liquidity_within()
        } else {
            tick_details.liq_token0 - tick_details.liq_bounds_token0._liquidity_within()
        };

        self.liq_shares = _calc_shares(
            self.order_size,
            tick_details.total_shares,
            dynamic_liquidity,
        );

        tick_details._add_liquidity(self.buy, self.order_size, self.liq_shares, 0);
    }

    /// Closing Update function
    ///
    /// closing a liquidity order
    ///
    /// Returns
    /// - Size token0   :This returns the amount of token0 within that tick corresponding to a order liq shares
    /// - Size token1  : This returns the amount of token0 within that tick corresponding to a order liq shares
    ///
    /// Note
    /// - Only one of these two can be non zero as liquidity order can not be closed if the current tick  is the  order's reference tick   

    fn _closing_update(&self, tick_details: &mut TickDetails) -> (Amount, Amount) {
        //
        let liq1_upper_bound = tick_details.liq_bounds_token1.upper_bound;
        let liq0_upper_bound = tick_details.liq_bounds_token0.upper_bound;

        let calc_shares_value = |all_liq: Amount, liq_upper_bound: Amount| {
            let dynamic_liquidity = if all_liq > liq_upper_bound {
                all_liq - liq_upper_bound
            } else {
                return 0;
            };
            _calc_shares_value(
                self.liq_shares,
                tick_details.total_shares,
                dynamic_liquidity,
            )
        };
        let token0_out = calc_shares_value(tick_details.liq_token0, liq0_upper_bound);

        let token1_out = calc_shares_value(tick_details.liq_token1, liq1_upper_bound);

        tick_details.liq_token0 -= token0_out;
        tick_details.liq_token1 -= token1_out;
        tick_details.total_shares -= self.liq_shares;

        return (token0_out, token1_out);
    }
}
