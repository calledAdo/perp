use candid::{CandidType, Decode, Encode, Principal};
use ic_cdk::export_candid;

use corelib::calc_lib::_calc_interest;
use corelib::order_lib::{CloseOrderParams, OpenOrderParams, Order, TradeOrder};
use corelib::price_lib::_equivalent;
use corelib::swap_lib::SwapParams;
use corelib::tick_lib::{_def_max_tick, _tick_to_price};

use types::{FundingRateTracker, MarketDetails, StateDetails, TickDetails, ID};

use serde::Deserialize;
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;

use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, Storable};
use ic_stable_structures::{DefaultMemoryImpl, StableBTreeMap, StableCell};

const _MARKET_DETAILS_MEMORY: MemoryId = MemoryId::new(1);

const _STATE_DETAILS_MEMORY: MemoryId = MemoryId::new(2);

const _USER_POSITION_MEMEORY: MemoryId = MemoryId::new(3);

const _FUNDING_RATE_TRACKER_MEMORY: MemoryId = MemoryId::new(4);

type Memory = VirtualMemory<DefaultMemoryImpl>;

thread_local! {

    static MEMORY_MANAGER:RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(MemoryManager::init(DefaultMemoryImpl::default())) ;

    static MARKET_DETAILS:StableCell<MarketDetails,Memory> = StableCell::new(MEMORY_MANAGER.with(
        |s|{s.borrow().get(_MARKET_DETAILS_MEMORY)}),MarketDetails::default()).unwrap();

        /// State details
    static STATE_DETAILS:RefCell<StableCell<StateDetails,Memory>> = RefCell::new(StableCell::new(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_STATE_DETAILS_MEMORY)
    }),StateDetails::default()).unwrap());

    static FUNDING_RATE_TRACKER:RefCell<StableCell<FundingRateTracker,Memory>> = RefCell::new(StableCell::new(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_FUNDING_RATE_TRACKER_MEMORY)
    }),FundingRateTracker::default()).unwrap());

    static USERS_POSITION:RefCell<StableBTreeMap<ID,PositionDetails,Memory>> = RefCell::new(
        StableBTreeMap::init(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_USER_POSITION_MEMEORY)
    })));

    static MULTIPLIERS_BITMAPS:RefCell<HashMap<u64,u128>> = RefCell::new(HashMap::new());

    static TICKS_DETAILS :RefCell<HashMap<Tick,TickDetails>> = RefCell::new(HashMap::new());



}

/// Open PositionDetails function
///
/// opens a new position for user (given that user has no existing position)
///
/// Params
/// - Collateral Value :: The amount in collatreal token to utilise as collateral
/// - Max Tick :: max executing tick ,also seen as max price fro the _swap ,if set to none or set outside the required range ,default max tick is used
/// - Leverage :: The leverage for the required position multiplies by 10 i.e a 1.5 levarage is 1.5 * 10 = 15
/// - Long :: Indicating if its a long position or not ,true if long and false otherwise
/// - Limit :: true if position type is a limit order
///
/// Note
///  - If Order type is a limit order ,max tick coinsides with the reference tick for the tradeorder

#[ic_cdk::update]
async fn open_position(
    collateral_value: Amount,
    max_tick: Option<Tick>,
    leveragex10: u8,
    long: bool,
    limit: bool,
) {
    let user = ID(ic_cdk::caller());

    //aseerts that user has no position already
    USERS_POSITION.with(|ref_users_position| {
        assert_eq!(ref_users_position.borrow().contains_key(&user), false);
    });

    let market_details = MARKET_DETAILS.with(|ref_market_details| ref_market_details.get().clone());

    let mut state_details =
        STATE_DETAILS.with(|ref_state_details| ref_state_details.borrow_mut().get().clone());

    // if leverage is greater than max leverage or collateral value is less than min collateral
    //returns
    if leveragex10 >= state_details.max_leveragex10
        || collateral_value < state_details.min_collateral
    {
        return;
    }

    let vault = Vault::init(market_details.vault_id.0, market_details.collateral_asset.0);

    // if deposit fails exit function
    if vault.send_asset_in(collateral_value).await == false {
        return;
    }

    // levarage is asways given as a multiple of ten
    let debt_value = (u128::from(leveragex10 - 10) * collateral_value) / 10;

    // if not enough liquidity to give out debt ,refund user and exit
    if vault.borrow_liquidty(debt_value).await == false {
        vault.send_asset_out(collateral_value);
        return;
    };

    let stopping_tick = max_or_default_max(max_tick, state_details.current_tick, long);

    match _open_position(
        long,
        limit,
        collateral_value,
        debt_value,
        state_details.interest_rate,
        state_details.current_tick,
        stopping_tick,
    ) {
        Some((unutilised_debt, resulting_tick, _crossed_ticks)) => {
            // update current tick
            state_details.current_tick = resulting_tick;

            STATE_DETAILS.with(|ref_state_details| {
                ref_state_details.borrow_mut().set(state_details).unwrap()
            });

            vault.update_asset_details(None, unutilised_debt, 0);

            if limit {
                let watcher = Watcher::init(market_details.watcher_id.0);

                watcher.store_tick_order(stopping_tick, user);
            }
        }
        None => {
            // if debt was not utilised ,refund user and send back debt
            vault.send_asset_out(collateral_value);

            // send back
            vault.update_asset_details(None, debt_value, 0);

            // panic to revert swap
            assert_eq!(true, false)
        }
    }
}

///Close PositionDetails Function
///
/// closes user position and sends back collateral
///
/// Returns
///  - Profit :The amount to send to position owner
///
/// Note
///  - if position_type is order ,the collateral is sent back and debt is sent back without interest
///
#[ic_cdk::update]
async fn close_position() -> Amount {
    let user = ID(ic_cdk::caller());

    let mut state_details =
        STATE_DETAILS.with(|ref_state_details| ref_state_details.borrow_mut().get().clone());

    let market_details = MARKET_DETAILS.with(|ref_market_details| ref_market_details.get().clone());

    // vault canister
    let vault = Vault::init(market_details.vault_id.0, market_details.collateral_asset.0);

    let current_tick = state_details.current_tick;

    let position =
        USERS_POSITION.with(|ref_user_position| ref_user_position.borrow().get(&user).unwrap());

    match position.position_type {
        PositionType::Market => {
            // if position type is market ,means the position is already active

            let (profit, resulting_tick, crossed_ticks) = if position.long {
                _close_long_position(user, position, current_tick, 10000, vault)
            } else {
                _close_short_position(user, position, current_tick, 100000, vault)
            };
            // update current_tick
            state_details.current_tick = resulting_tick;

            STATE_DETAILS.with(|ref_state_details| {
                ref_state_details.borrow_mut().set(state_details).unwrap()
            });

            vault.send_asset_out(profit);

            let watcher = Watcher::init(market_details.watcher_id.0);

            // send out ticks
            watcher.execute_ticks_orders(crossed_ticks);

            // return profits
            return profit;
        }
        PositionType::Order(order) => {
            // close trade _order
            _close_order(&order, order.order_size, order.buy, order.ref_tick);

            let debt = if position.long {
                position.debt
            } else {
                let tick_price = _tick_to_price(order.ref_tick);
                _equivalent(position.debt, tick_price, false)
            };

            // delete user position
            USERS_POSITION.with(|ref_user_position| ref_user_position.borrow_mut().remove(&user));

            // send asset out
            vault.send_asset_out(position.collateral_value);
            // uodate position
            vault.update_asset_details(None, debt, 0);

            return position.collateral_value;
        }
    };
}

/// Convert Position Function
///
/// converts user position of order type to market type after order has been filled
///
/// Params ;
///  - User :The principal of the position's owner
///
/// Note:
///  - This function can only be called by watcher

#[ic_cdk::update]
async fn convert_position(user_id: Principal) {
    let user = ID(user_id);
    let mut position =
        USERS_POSITION.with(|ref_user_position| ref_user_position.borrow().get(&user).unwrap());

    let equivalent = |amount: Amount, buy: bool| -> Amount {
        let current_price = _tick_to_price(position.entry_tick);
        _equivalent(amount, current_price, buy)
    };

    if let PositionType::Order(_) = position.position_type {
        let volume_share = FUNDING_RATE_TRACKER.with(|tr| {
            let mut funding_rate_tracker = { tr.borrow().get().clone() };

            let debt_value = if position.long {
                position.debt
            } else {
                equivalent(position.debt, false)
            };

            let share = funding_rate_tracker
                .add_volume(position.collateral_value + debt_value, position.long);
            //
            //
            tr.borrow_mut().set(funding_rate_tracker).unwrap();
            share
        });

        position.timestamp = 0; // now
        position.volume_share = volume_share;
        position.position_type = PositionType::Market;

        USERS_POSITION.with(|ref_users_position| {
            ref_users_position
                .borrow_mut()
                .insert(ID(ic_cdk::caller()), position)
        });
    }
}
///
///
/// Open PositionDetails (Private)
///
/// opens a position for user if possible
/// Params
///  - Long : Position direction ,true if long or false otherwise
///  - Collateral Value : amount of collateral asset being put in as collateral
///  - Debt Value : The amount of collateral_asset used as debt for opening position
///  - Interest Rate : The current interest rate for opening a position
///  - Entry Tick : The entry tick or the current state tick for this market
///  - Max Tick : The maximum tick to execute swap ,also seen as maximum price
///
/// Returns
///  - Option containing
///  - - Amount Remaining Value :The value of  amount remaining or amount of unutilised debt
///  - - Resulting Tick :The resuting tick from swapping
///  - - Crossed Ticks :A vector of all crossed ticks during swap
/// Note
///  - If position can not be opened it returns none and both collateral and debt gets refunded back and swap is reverted afterwards
///
fn _open_position(
    long: bool,
    limit: bool,
    collateral_value: Amount,
    debt_value: Amount,
    interest_rate: u32,
    entry_tick: Tick,
    max_tick: Tick,
) -> Option<(Amount, Tick, Vec<Tick>)> {
    let current_price = _tick_to_price(entry_tick);

    //
    let equivalent =
        |amount: Amount, buy: bool| -> Amount { _equivalent(amount, current_price, buy) };

    let (collateral, debt) = if long {
        (collateral_value, debt_value)
    } else {
        (
            equivalent(collateral_value, true),
            equivalent(debt_value, true),
        )
    };
    let position: PositionDetails;

    let result; //(unutilised_debt,resulting_tick,crossed_ticks);

    match limit {
        true => {
            let entry_tick = max_tick;
            //
            let mut order = TradeOrder::new(collateral + debt, entry_tick, long);

            _open_order(&mut order, entry_tick);

            position = PositionDetails {
                long,
                entry_tick,
                collateral_value,
                debt,
                interest_rate,
                volume_share: 0, // not initialised yet
                position_type: PositionType::Order(order),
                timestamp: 0, //not initialised
            };

            result = (0, entry_tick, Vec::new());
        }

        //
        false => {
            // swap is executed

            let (_, amount_remaining, resulting_tick, crossed_ticks) =
                _swap(collateral + debt, long, entry_tick, max_tick);

            if amount_remaining >= debt {
                return None;
            }

            // the amount remaining value to be refunded to
            let amount_remaining_value = if long {
                amount_remaining
            } else {
                equivalent(amount_remaining, false)
            };

            let volume_share = FUNDING_RATE_TRACKER.with(|tr| {
                let mut funding_rate_tracker = { tr.borrow().get().clone() };

                let share = funding_rate_tracker
                    .add_volume(collateral_value + debt_value - amount_remaining_value, long);
                //
                //
                tr.borrow_mut().set(funding_rate_tracker).unwrap();
                share
            });

            position = PositionDetails {
                long,
                entry_tick: resulting_tick,
                collateral_value,
                debt: debt - amount_remaining, //actual debt
                interest_rate,

                volume_share,
                position_type: PositionType::Market,
                timestamp: 1000, //change to time()
            };
            result = (amount_remaining_value, resulting_tick, crossed_ticks);
        }
    }

    USERS_POSITION.with(|ref_users_position| {
        ref_users_position
            .borrow_mut()
            .insert(ID(ic_cdk::caller()), position)
    });

    return Some(result);
}

/// Close Long PositionDetails
///
///closes a user's  long position if position can be fully closed and  repays debt
///
/// Params
/// - User :The user
/// - PositionDetails :The PositionDetails
/// - Current Tick :The current tick of market's state
/// - Stopping Tick : The max tick,corresponds to max price
/// - Vault : Vault canister
///
/// Returns
///  - Profit :The amount to send to position owner after paying debt ,this amount is zero if debt is not fully paid
///  - Resulting Tick :The resulting tick from swapping
///  - Crosssed Ticks :An array of ticks that have been crossed during swapping
///   
/// Note
///  - If position can not be closed fully ,the position is partially closed (updated) and debt is paid back either fully or partially
fn _close_long_position(
    user: ID,
    position: PositionDetails,
    current_tick: Tick,
    stopping_tick: Tick,
    vault: Vault,
) -> (Amount, Tick, Vec<Tick>) {
    //
    //
    let equivalent = |amount: Amount, tick: Tick, buy: bool| -> Amount {
        let current_price = _tick_to_price(tick);
        _equivalent(amount, current_price, buy)
    };
    //
    let position_realised_value = FUNDING_RATE_TRACKER.with(|tr| {
        let mut funding_rate_tracker = { tr.borrow().get().clone() };
        let value = funding_rate_tracker.remove_volume(position.volume_share, true);
        tr.borrow_mut().set(funding_rate_tracker).unwrap();
        value
    });

    // perp  asset

    let realised_position_size = equivalent(position_realised_value, position.entry_tick, true);

    //sell

    let (amount_out, amount_remaining, resulting_tick, crossed_ticks) =
        _swap(realised_position_size, false, current_tick, stopping_tick);

    let interest_fee = _calc_interest(position.debt, position.interest_rate, position.timestamp);

    let total_fee = position.debt + interest_fee;

    let profit;

    if amount_remaining > 0 {
        // update position ,paying debt either partially or fully
        let new_volume_share = FUNDING_RATE_TRACKER.with(|tr| {
            let mut funding_rate_tracker = { tr.borrow().get().clone() };

            let share = funding_rate_tracker
                .add_volume(equivalent(amount_remaining, resulting_tick, false), true);

            tr.borrow_mut().set(funding_rate_tracker).unwrap();
            share
        });

        let mut new_position = PositionDetails {
            entry_tick: resulting_tick,
            collateral_value: position.collateral_value,
            long: position.long,
            debt: 0, // initialise debt as 0
            volume_share: new_volume_share,

            interest_rate: position.interest_rate,
            position_type: PositionType::Market,
            timestamp: position.timestamp,
        };

        //
        if amount_out < total_fee {
            //update new position details
            new_position.debt = total_fee - amount_out;
            new_position.timestamp = 1000; //change

            // if any debt can be given at all
            let interest_received = if amount_out > position.debt {
                amount_out - position.debt
            } else {
                0
            };

            vault.update_asset_details(Some(new_position.debt), amount_out, interest_received);

            // profit is zero
            profit = 0;
        } else {
            // debt is zero
            new_position.collateral_value = new_volume_share;

            vault.update_asset_details(None, position.debt, interest_fee);

            profit = amount_out - total_fee;
        }

        //
        USERS_POSITION
            .with(|ref_user_position| ref_user_position.borrow_mut().insert(user, new_position));
        //()
    } else {
        profit = amount_out - total_fee;

        USERS_POSITION.with(|ref_user_position| ref_user_position.borrow_mut().remove(&user));
    }

    return (profit, resulting_tick, crossed_ticks);
}

///
/// Close Short Position
///
/// similar to Close Long Function,but for short positions

fn _close_short_position(
    user: ID,
    position: PositionDetails,
    current_tick: Tick,
    stopping_tick: Tick,
    vault: Vault,
) -> (Amount, Tick, Vec<Tick>) {
    //
    //
    let equivalent = |amount: Amount, tick: Tick, buy: bool| -> Amount {
        let current_price = _tick_to_price(tick);
        _equivalent(amount, current_price, buy)
    };

    let position_realised_value = FUNDING_RATE_TRACKER.with(|tr| {
        let mut funding_rate_tracker = tr.borrow().get().clone();
        //
        let value = funding_rate_tracker.remove_volume(position.volume_share, false);
        //
        tr.borrow_mut().set(funding_rate_tracker).unwrap();
        value
    });

    let realised_position_size = position_realised_value;

    let (amount_out, amount_remaining, resulting_tick, crossed_ticks) =
        _swap(realised_position_size, true, current_tick, stopping_tick);

    let interest_fee = _calc_interest(position.debt, position.interest_rate, position.timestamp);

    let total_fee = position.debt + interest_fee;

    let profit;

    if amount_remaining != 0 {
        let new_volume_share = FUNDING_RATE_TRACKER.with(|tr| {
            let mut funding_rate_tracker = { tr.borrow().get().clone() };

            let share = funding_rate_tracker.add_volume(amount_remaining, position.long);

            tr.borrow_mut().set(funding_rate_tracker).unwrap();
            share
        });

        //
        let mut new_position = PositionDetails {
            entry_tick: resulting_tick,
            collateral_value: position.collateral_value,
            long: position.long,
            debt: 0,
            volume_share: new_volume_share,
            interest_rate: position.interest_rate,
            position_type: PositionType::Market,
            timestamp: 1000,
        };

        if amount_out < total_fee {
            new_position.debt = total_fee - amount_out;

            let interest_received = if amount_out > position.debt {
                amount_out - position.debt
            } else {
                0
            };
            //
            vault.update_asset_details(
                Some(equivalent(new_position.debt, resulting_tick, false)),
                equivalent(amount_out, resulting_tick, false),
                equivalent(interest_received, resulting_tick, false),
            );

            profit = 0;
        } else {
            new_position.collateral_value = new_volume_share;
            vault.update_asset_details(
                None,
                equivalent(position.debt, resulting_tick, false),
                equivalent(interest_fee, resulting_tick, false),
            );

            profit = equivalent(amount_out - total_fee, resulting_tick, false);
        }

        USERS_POSITION
            .with(|ref_user_position| ref_user_position.borrow_mut().insert(user, new_position));
    } else {
        profit = equivalent(amount_out - total_fee, resulting_tick, false);

        // deletes user position
        USERS_POSITION.with(|ref_user_position| ref_user_position.borrow_mut().remove(&user));
    }

    return (profit, resulting_tick, crossed_ticks);
}

///
///
///
/// Opens Order Functions
///
/// opens an order at a particular tick
///
/// Params
/// - Order :: a generic type that implements the trait Order for the type of order to close
/// - Reference Tick :: The  tick to place order

fn _open_order<V: Order>(order: &mut V, reference_tick: Tick) {
    TICKS_DETAILS.with(|ref_ticks_details| {
        let ticks_details = &mut *ref_ticks_details.borrow_mut();
        MULTIPLIERS_BITMAPS.with(|ref_multiplier_bitmaps| {
            let multipliers_bitmaps = &mut *ref_multiplier_bitmaps.borrow_mut();

            let mut open_order_params = OpenOrderParams {
                order,
                reference_tick,
                multipliers_bitmaps,
                ticks_details,
            };

            open_order_params.open_order();
        })
    });
}

///
/// Close Order Function
///
/// closes an order at a particular tick
///
/// Params :
///  - Order :: a generic that implements the trait Order for the type of order to close
///  - Order Size :: Tha amount of asset in order
///  - Order Direction :: Either a buy or a sell
///  - Order Reference Tick :: The tick where order was placed  
///
/// Returns
///  - Amont Out :: This corresponds to the asset to be bought i.e perp(base) asset for a buy order or quote asset for a sell order
///  - Amount Remaining :: This amount remaining corrseponds to the amount of asset at that tick that is still unfilled
///
/// Note
///  - When closing Liqudiity order ,amount out  and amount remaining corresponds to amount of perp asset and collateral asset respectively
///

fn _close_order<V: Order>(
    order: &V,
    order_size: Amount,
    order_direction: bool,
    order_reference_tick: Tick,
) -> (Amount, Amount) {
    TICKS_DETAILS.with(|ref_ticks_details| {
        let ticks_details = &mut *ref_ticks_details.borrow_mut();
        MULTIPLIERS_BITMAPS.with(|ref_multiplier_bitmaps| {
            let multipliers_bitmaps = &mut *ref_multiplier_bitmaps.borrow_mut();

            let mut close_order_params = CloseOrderParams {
                order,
                order_size,
                order_direction,
                order_reference_tick,
                multipliers_bitmaps,
                ticks_details,
            };
            close_order_params.close_order()
        })
    })
}

/// Swap Function
///
/// Params
///  - Order Size :: Tha amount of asset in order
///  - Buy :: the order direction ,true for buy and false otherwise
///  - Init Tick :: The current state tick
///  - Stopping Tick :: The maximum tick ,corresponds to maximum price
///
/// Returns
///  - Amount Out :: The amount out froom swapping
///  - Amount Remaining :: The amount remaining from swapping
///  - Crossed Ticks :: An vector of all ticks crossed during swap

fn _swap(
    order_size: Amount,
    buy: bool,
    init_tick: Tick,
    stopping_tick: Tick,
) -> (Amount, Amount, Tick, Vec<Tick>) {
    TICKS_DETAILS.with(|ref_ticks_details| {
        let ticks_details = &mut ref_ticks_details.borrow_mut();
        MULTIPLIERS_BITMAPS.with(|ref_multiplier_bitmaps| {
            let multipliers_bitmaps = &mut ref_multiplier_bitmaps.borrow_mut();

            let mut swap_params = SwapParams {
                buy,
                init_tick,
                stopping_tick,
                order_size,
                multipliers_bitmaps,
                ticks_details,
            };
            swap_params._swap()
        })
    })
}

/// Max or Default Max Tick
///
/// retrieves the max tick if valid else returns the default max tick

fn max_or_default_max(max_tick: Option<Tick>, current_tick: Tick, buy: bool) -> Tick {
    match max_tick {
        Some(tick) => {
            if buy && tick < _def_max_tick(current_tick, true) {
                return tick;
            };
            if !buy && tick > _def_max_tick(current_tick, false) {
                return tick;
            }
        }
        None => {}
    }
    _def_max_tick(current_tick, buy)
}

struct Watcher {
    canister_id: Principal,
}

impl Watcher {
    pub fn init(canister_id: Principal) -> Self {
        return Watcher { canister_id };
    }

    pub fn store_tick_order(&self, tick: Tick, user: ID) {
        let _ = ic_cdk::notify(self.canister_id, "storeTickOrder", (tick, user));
    }

    pub fn execute_ticks_orders(&self, ticks: Vec<Tick>) {
        let _ = ic_cdk::notify(self.canister_id, "executeTicksOrders", (ticks,));
    }
}

/// The Vault type representing vault canister that stores asset for the entire protocol
/// it facilitates all movement of assets including collection and repayment of debt utilised for leverage

#[derive(Clone, Copy)]
struct Vault {
    canister_id: Principal,
    asset_id: Principal,
}

impl Vault {
    // initialises the vault canister
    pub fn init(canister_id: Principal, asset_id: Principal) -> Self {
        Vault {
            canister_id,
            asset_id,
        }
    }

    /// Update Assets Details function
    ///
    /// Updates the details of the particular asset
    /// utlised when paying debt ,updating debt on a particular asset
    pub fn update_asset_details(
        &self,
        new_debt: Option<Amount>,
        amount_received: Amount,
        interest: Amount,
    ) {
        if new_debt == None && amount_received == 0 && interest == 0 {
            return; // returns without sending any message
        }
        let _ = ic_cdk::notify(
            self.canister_id,
            "updatedPosition",
            (self.asset_id, new_debt, amount_received, interest),
        );
    }

    /// Borrow Liquidity functiion
    ///
    /// Borrows liquidty from vault to utilise as leverage for opening position
    /// returns true is vault contains that amount of free liquidity or false otherwise
    pub async fn borrow_liquidty(&self, amount: Amount) -> bool {
        if amount == 0 {
            return true;
        };
        let (val,) = ic_cdk::call(self.canister_id, "lock_liquidity", (amount,))
            .await
            .unwrap();

        return val;
    }

    /// Send Asset in Function
    ///
    /// removes asset from user's account
    ///
    /// returns true if user has sufficient amount and false otherwise
    pub async fn send_asset_in(&self, amount: Amount) -> bool {
        let (val,) = ic_cdk::call(self.canister_id, "send_asset_in", (amount,))
            .await
            .unwrap();
        return val;
    }

    /// Send Asset Out Function
    ///
    /// send asset to user's account
    pub fn send_asset_out(&self, amount: Amount) {
        if amount == 0 {
            return;
        };
        let _ = ic_cdk::notify(self.canister_id, "send_asset_out", (amount,));
    }
}

type Time = u64;
type Amount = u128;
type Tick = u64;

#[derive(CandidType, Deserialize, Clone)]
enum PositionType {
    Market,
    Order(TradeOrder),
}

#[derive(CandidType, Deserialize, Clone)]
struct PositionDetails {
    /// Entry Tick
    ///
    /// The tick at which position is opened
    entry_tick: Tick,
    /// true if long
    long: bool,
    /// Collatreal Value
    ///
    /// collatreal within position
    collateral_value: Amount,
    /// Debt
    ///
    /// the amount borrowed as leveragex10
    ///
    /// Note:debt is in perp Asset when shorting and in collateral_value asset when longing
    debt: Amount,
    // /// PositionDetails Size
    // ///
    // /// The amount of asset in position
    // ///
    // /// This can either be
    // ///
    // ///  - The amount resulting from the _swap when opening a position or
    // ///  - The amount used to gotten from opening placing order at a tick in the case of an order type
    // position_size: Amount,
    /// Volume Share
    ///
    ///Measure of liqudiity share in position with respect to the net amount in all open position of same direction i.e
    /// LONG or SHORT
    volume_share: Amount,
    /// Intrerest Rate
    ///
    /// Current interest rate for opening a position with margin
    ///
    interest_rate: u32,
    ///PositionDetails Type
    ///
    ///Posittion type can either be a
    ///
    /// Market
    ///  - This is when position is opened instantly at the current price
    ///
    /// Order
    ///   - This comprises of an order set at a particular tick and position is only opened when
    ///   that  order has been executed
    position_type: PositionType,

    /// TimeStamp
    ///
    /// timestamp when psotion was executed opened
    /// Tnis corresponds to the start time for  calculating interest rate on a leveraged position
    ///
    /// Note: For order type, position this  is time  order was excuted
    timestamp: Time,
}

impl Storable for PositionDetails {
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
}

impl BoundedStorable for PositionDetails {
    const IS_FIXED_SIZE: bool = true;

    const MAX_SIZE: u32 = 90;
}

export_candid!();

pub mod corelib;
pub mod types;
