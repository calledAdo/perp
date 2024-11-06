use candid::{CandidType, Decode, Encode, Principal};
use ic_cdk::{export_candid, storage};

use sha2::{Digest, Sha256};

use corelib::calc_lib::{_calc_interest, _percentage64};
use corelib::constants::{_BASE_PRICE, _ONE_PERCENT};
use corelib::order_lib::{CloseOrderParams, LimitOrder, OpenOrderParams};
use corelib::price_lib::_equivalent;
use corelib::swap_lib::SwapParams;
use corelib::tick_lib::{_def_max_tick, _tick_to_price};
use types::{
    FundingRateTracker, GetExchangeRateRequest, GetExchangeRateResult, MarketDetails, StateDetails,
    TickDetails, ID,
};

use serde::{Deserialize, Serialize};
use std::borrow::Cow;
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt::Debug;
use std::time::Duration;

use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};
use ic_stable_structures::{BoundedStorable, Storable};
use ic_stable_structures::{DefaultMemoryImpl, StableBTreeMap, StableCell};

type Time = u64;
type Amount = u128;
type Tick = u64;
type Subaccount = [u8; 32];

type Memory = VirtualMemory<DefaultMemoryImpl>;

const _MARKET_DETAILS_MEMORY: MemoryId = MemoryId::new(1);

const _STATE_DETAILS_MEMORY: MemoryId = MemoryId::new(2);

const _USER_POSITION_MEMEORY: MemoryId = MemoryId::new(3);

const _FUNDING_RATE_TRACKER_MEMORY: MemoryId = MemoryId::new(4);

const _ADMIN_MEMORY: MemoryId = MemoryId::new(5);

const _ACCOUNT_ERROR_LOGS_MEMORY: MemoryId = MemoryId::new(6);

const ONE_HOUR: u64 = 3_600_000_000_000;

const DEFAULT_SWAP_SLIPPAGE: u64 = 50_000; //0.5%

thread_local! {

    static MEMORY_MANAGER:RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(MemoryManager::init(DefaultMemoryImpl::default())) ;

    static ADMIN:RefCell<StableCell<ID,Memory>> = RefCell::new(StableCell::new(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_ADMIN_MEMORY)
    }),ID::from(Principal::anonymous())).unwrap());


    static MARKET_DETAILS:RefCell<StableCell<MarketDetails,Memory>> = RefCell::new(StableCell::new(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_MARKET_DETAILS_MEMORY)
    }),MarketDetails::default()).unwrap());


        /// State details
    static STATE_DETAILS:RefCell<StableCell<StateDetails,Memory>> = RefCell::new(StableCell::new(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_STATE_DETAILS_MEMORY)
    }),StateDetails::default()).unwrap());

    static FUNDING_RATE_TRACKER:RefCell<StableCell<FundingRateTracker,Memory>> = RefCell::new(StableCell::new(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_FUNDING_RATE_TRACKER_MEMORY)
    }),FundingRateTracker::default()).unwrap());

    static ACCOUNTS_POSITION:RefCell<StableBTreeMap<Subaccount,PositionDetails,Memory>> = RefCell::new(
        StableBTreeMap::init(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_USER_POSITION_MEMEORY)
    })));


    static ACCOUNTS_ERROR_LOGS:RefCell<StableBTreeMap<Subaccount,PositionUpdateErrorLog,Memory>> = RefCell::new(
        StableBTreeMap::init(MEMORY_MANAGER.with(|s|{
        s.borrow().get(_ACCOUNT_ERROR_LOGS_MEMORY)
    })));

    static INTEGRAL_BITMAPS:RefCell<HashMap<u64,u128>> = RefCell::new(HashMap::new());

    static TICKS_DETAILS :RefCell<HashMap<Tick,TickDetails>> = RefCell::new(HashMap::new());

    static ERRORS:RefCell<Vec<ErrorType>> = RefCell::new(Vec::new());

}

#[ic_cdk::init]
fn init(market_details: MarketDetails) {
    let caller = ic_cdk::api::caller();

    ADMIN.with(|ref_admin| ref_admin.borrow_mut().set(ID::from(caller)).unwrap());
    MARKET_DETAILS.with(|ref_market_details| {
        ref_market_details.borrow_mut().set(market_details).unwrap();
    });
}

/// Get State Details
///
/// Returns the Current State Details

#[ic_cdk::query(name = "getStateDetails")]
fn get_state_details() -> StateDetails {
    _get_state_details()
}

/// Get Market Details
///
///  Returns the Market Details

#[ic_cdk::query(name = "getMarketDetails")]
fn get_market_details() -> MarketDetails {
    _get_market_details()
}

///  get Tick Details
///
/// Returns the tick details of a particular tick if it is intialised else returns false

#[ic_cdk::query(name = "getTickDetails")]
fn get_tick_details(tick: Tick) -> TickDetails {
    TICKS_DETAILS.with(|ref_tick_details| ref_tick_details.borrow().get(&tick).unwrap().clone())
}

/// Try Close Function
///
/// Checks if a particular account's position of limit order type has been fully filled

#[ic_cdk::query(name = "tryClose")]
fn try_close(account: [u8; 32]) -> bool {
    return convert_position(account);
}

/// Get Account Position
///
/// Gets an account position or panics if account has no position
#[ic_cdk::query(name = "getAccountPosition")]
fn get_account_position(account: [u8; 32]) -> PositionDetails {
    return _get_account_position(&account);
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
/// - Order Type :: the type of order to create
///  _
///
/// Returns
///  - Position:the details of the position
///
/// Note
///  - If Order type is a limit order ,max tick coinsides with the reference tick for the limit order
///  - ANON TICKS are for future purposes and have no effect for now

#[ic_cdk::update(name = "openPosition")]
async fn open_position(
    collateral_value: Amount,
    max_tick: Option<Tick>,
    leveragex10: u8,
    long: bool,
    order_type: OrderType,
    _anon_tick1: Tick,
    _anon_tick2: Tick,
) -> Result<PositionDetails, String> {
    let user = ic_cdk::caller();

    let account = user._to_subaccount();

    //aseerts that user has no position already
    let failed_initial_check = _has_position_or_pending_error_log(&account);

    if failed_initial_check {
        return Err("Account has pending error or unclosed position ".to_string());
    }

    let mut state_details = _get_state_details();

    assert!(state_details.not_paused);

    // if leverage is greater than max leverage or collateral value is less than min collateral
    //returns
    if leveragex10 >= state_details.max_leveragex10
        || collateral_value < state_details.min_collateral
    {
        return Err("Max leverage exceeded or collateral is too small".to_string());
    }

    let market_details = _get_market_details();

    let vault = Vault::init(market_details.vault_id);

    // levarage is always given as a multiple of ten
    let debt_value = (u128::from(leveragex10 - 10) * collateral_value) / 10;

    // Checks if user has sufficient balance and vault contains free liquidity greater or equal to debt_value and then calculate interest rate

    let (valid, interest_rate) = vault
        .create_position_validity_check(user, collateral_value, debt_value)
        .await;

    if valid == false {
        return Err("Not enough liquidity for debt".to_string());
    };

    let stopping_tick = max_or_default_max(max_tick, state_details.current_tick, long);

    match _open_position(
        account,
        long,
        order_type,
        collateral_value,
        debt_value,
        interest_rate,
        state_details.current_tick,
        stopping_tick,
    ) {
        Some((position, resulting_tick, crossed_ticks)) => {
            // update current tick
            state_details.current_tick = resulting_tick;

            _update_state_details(state_details);

            let watcher = Watcher::init(market_details.watcher_id);

            if let OrderType::Limit = order_type {
                watcher.store_tick_order(stopping_tick, account);
            } else {
                watcher.execute_ticks_orders(crossed_ticks);

                if position.debt_value != debt_value
                    || collateral_value != position.collateral_value
                {
                    let un_used_collateral = collateral_value - position.collateral_value;
                    vault.manage_position_update(
                        user,
                        collateral_value - position.collateral_value,
                        ManageDebtParams::init(position.debt_value, debt_value, un_used_collateral),
                    );
                }
            }

            return Ok(position);
        }
        None => {
            // send back
            vault.manage_position_update(
                user,
                collateral_value,
                ManageDebtParams::init(0, debt_value, 0),
            );

            return Err("Failed to open position".to_string());
        }
    }
}

///Close PositionDetails Function
///
/// Closes user position and sends back collateral
///
/// Returns
///  - Profit :The amount to send to position owner
///
/// Note
///  - if position_type is order ,the collateral is sent back and debt is sent back without interest
///
#[ic_cdk::update(name = "closePosition")]
async fn close_position(max_tick: Option<Tick>) -> Amount {
    let user = ic_cdk::caller();

    let account = user._to_subaccount();

    let mut position = _get_account_position(&account);

    let mut state_details = _get_state_details();

    assert!(state_details.not_paused);
    //
    let market_details = _get_market_details();

    // vault canister
    let vault = Vault::init(market_details.vault_id);

    let watcher = Watcher::init(market_details.watcher_id);

    match position.order_type {
        PositionOrderType::Market => {
            let current_tick = state_details.current_tick;

            let stopping_tick = max_or_default_max(max_tick, current_tick, !position.long);
            // if position type is market ,means the position is already active
            let (collateral_value, resulting_tick, crossed_ticks, manage_debt_params) = if position
                .long
            {
                _close_market_long_position(account, &mut position, current_tick, stopping_tick)
            } else {
                _close_market_short_position(account, &mut position, current_tick, stopping_tick)
            };
            // update current_tick
            state_details.current_tick = resulting_tick;

            _update_state_details(state_details);

            // send out ticks
            watcher.execute_ticks_orders(crossed_ticks);

            vault.manage_position_update(user, collateral_value, manage_debt_params);

            // return profits
            return collateral_value;
        }
        PositionOrderType::Limit(_) => {
            let (removed_collateral, manage_debt_params) = if position.long {
                _close_limit_long_position(account, &mut position)
            } else {
                _close_limit_short_position(account, &mut position)
            };

            if manage_debt_params.new_debt == 0 {
                watcher.remove_tick_order(position.entry_tick, account)
            }

            vault.manage_position_update(user, removed_collateral, manage_debt_params);

            return removed_collateral;
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
///  - This function does not check if canister is paused or not,to prevent watcher from encountering an error

#[ic_cdk::update(name = "convertPosition")]
fn convert_position(account: Subaccount) -> bool {
    // assert that only watcher can call this function

    let mut position = _get_account_position(&account);

    if let PositionOrderType::Limit(order) = position.order_type {
        //since tivk is deleted ,order has been confirmed closed
        let (_, amount_remaining) = _close_order(&order);

        let valid_close = amount_remaining == 0;
        _convert_limit_position(&mut position, 0);
        _insert_account_position(account, position);

        return valid_close;
    }

    return false;
}

///
///
/// Open PositionDetails (Private)
///
/// opens a position for user if possible
/// Params
///  - User :The owner of the position
///  - Long : Position direction ,true if long or false otherwise
///  - Limit : true for opening a limit order and false for a market order
///  - Collateral Value : amount of collateral asset being put in as collateral
///  - Debt Value : The amount of collateral_asset used as debt for opening position
///  - Interest Rate : The current interest rate for opening a position
///  - Entry Tick : The entry tick or the current state tick for this market
///  - Max Tick : The maximum tick to execute swap ,also seen as maximum price
///
/// Returns
///  - Option containing
///  - - Position Details :The details of the position created
///  - - Resulting Tick :The resuting tick from swapping
///  - - Crossed Ticks :A vector of all crossed ticks during swap
/// Note
///  - If position can not be opened it returns none and both collateral and debt gets refunded back and swap is reverted afterwards
///
fn _open_position(
    account: Subaccount,
    long: bool,
    order_type: OrderType,
    collateral_value: Amount,
    debt_value: Amount,
    interest_rate: u32,
    current_tick: Tick,
    max_tick: Tick,
) -> Option<(PositionDetails, Tick, Vec<Tick>)> {
    //
    let equivalent = |amount: Amount, tick: Tick, buy: bool| -> Amount {
        let tick_price = _tick_to_price(tick);
        _equivalent(amount, tick_price, buy)
    };
    let position: PositionDetails;

    let open_position_result; //(actual debt,resulting_tick,crossed_ticks);

    match order_type {
        OrderType::Limit => {
            let entry_tick = max_tick;
            // limit order's can't be placed at current tick
            if long && entry_tick >= current_tick {
                return None;
            } else if !long && entry_tick <= current_tick {
                return None;
            } else {
            };

            let (collateral, debt) = if long {
                (collateral_value, debt_value)
            } else {
                (
                    equivalent(collateral_value, entry_tick, true),
                    equivalent(debt_value, entry_tick, true),
                )
            };
            //
            let mut order = LimitOrder::new(collateral + debt, entry_tick, long);

            _open_order(&mut order);

            position = PositionDetails {
                long,
                entry_tick,
                collateral_value,
                debt_value,
                interest_rate,
                volume_share: 0, // not initialised yet
                order_type: PositionOrderType::Limit(order),
                timestamp: 0, //not initialised
            };

            open_position_result = (position, current_tick, Vec::new());
        }

        //
        OrderType::Market => {
            let (collateral, debt) = if long {
                (collateral_value, debt_value)
            } else {
                (
                    equivalent(collateral_value, current_tick, true),
                    equivalent(debt_value, current_tick, true),
                )
            };

            // swap is executed

            let (amount_out, amount_remaining, resulting_tick, crossed_ticks) =
                _swap(collateral + debt, long, current_tick, max_tick);

            if amount_out == 0 {
                return None;
            }

            // the amount remaining value to be refunded to
            let amount_remaining_value = if long {
                amount_remaining
            } else {
                equivalent(amount_remaining, current_tick, false)
            };

            let position_value = if long {
                collateral_value + debt_value - amount_remaining_value
            } else {
                amount_out
            };

            let (unused_collateral_value, unused_debt_value);

            if amount_remaining_value >= debt_value {
                unused_debt_value = debt_value;
                unused_collateral_value = amount_remaining - debt_value
            } else {
                unused_debt_value = amount_remaining_value;
                unused_collateral_value = 0
            }

            let resulting_debt_value = debt_value - unused_debt_value;
            let resulting_collateral_value = collateral_value - unused_collateral_value;

            let volume_share = _calc_position_volume_share(position_value, long);

            position = PositionDetails {
                long,
                entry_tick: resulting_tick,
                collateral_value: resulting_collateral_value,
                debt_value: resulting_debt_value, //actual debt
                interest_rate,
                volume_share,
                order_type: PositionOrderType::Market,
                timestamp: ic_cdk::api::time(), //change to time()
            };
            open_position_result = (position, resulting_tick, crossed_ticks);
        }
    }

    _insert_account_position(account, position);

    return Some(open_position_result);
}

fn _open_market_long_position(
    account: Subaccount,
    long: bool,
    collateral_value: Amount,
    debt_value: Amount,
    interest_rate: u32,
    current_tick: Tick,
    max_tick: Tick,
) -> Option<(PositionDetails, Tick, Vec<Tick>)> {
    let (collateral, debt) = (collateral_value, debt_value);

    let (amount_out, amount_remaining_value, resulting_tick, crossed_ticks) =
        _swap(collateral + debt, long, current_tick, max_tick);

    if amount_out == 0 {
        return None;
    }

    let position_value = collateral_value + debt_value - amount_remaining_value;

    let (unused_collateral_value, unused_debt_value);

    if amount_remaining_value >= debt_value {
        unused_debt_value = debt_value;
        unused_collateral_value = amount_remaining_value - debt_value
    } else {
        unused_debt_value = amount_remaining_value;
        unused_collateral_value = 0
    }

    let resulting_debt_value = debt_value - unused_debt_value;
    let resulting_collateral_value = collateral_value - unused_collateral_value;

    let volume_share = _calc_position_volume_share(position_value, long);

    let position = PositionDetails {
        long,
        entry_tick: resulting_tick,
        collateral_value: resulting_collateral_value,
        debt_value: resulting_debt_value, //actual debt
        interest_rate,
        volume_share,
        order_type: PositionOrderType::Market,
        timestamp: ic_cdk::api::time(), //change to time()
    };
    _insert_account_position(account, position);

    return Some((position, resulting_tick, crossed_ticks));
}

fn _open_market_short_position(
    account: Subaccount,
    long: bool,
    collateral_value: Amount,
    debt_value: Amount,
    interest_rate: u32,
    current_tick: Tick,
    max_tick: Tick,
) -> Option<(PositionDetails, Tick, Vec<Tick>)> {
    let equivalent = |amount: Amount, tick: Tick, buy: bool| -> Amount {
        let tick_price = _tick_to_price(tick);
        _equivalent(amount, tick_price, buy)
    };
    let (collateral, debt) = (
        equivalent(collateral_value, current_tick, true),
        equivalent(debt_value, current_tick, true),
    );

    // swap is executed

    let (amount_out, amount_remaining, resulting_tick, crossed_ticks) =
        _swap(collateral + debt, long, current_tick, max_tick);

    if amount_out == 0 {
        return None;
    }

    // the amount remaining value to be refunded to
    let amount_remaining_value = equivalent(amount_remaining, current_tick, false);

    let position_value = amount_out;

    let (unused_collateral_value, unused_debt_value);

    if amount_remaining_value >= debt_value {
        unused_debt_value = debt_value;
        unused_collateral_value = amount_remaining - debt_value
    } else {
        unused_debt_value = amount_remaining_value;
        unused_collateral_value = 0
    }

    let resulting_debt_value = debt_value - unused_debt_value;
    let resulting_collateral_value = collateral_value - unused_collateral_value;

    let volume_share = _calc_position_volume_share(position_value, long);

    let position = PositionDetails {
        long,
        entry_tick: resulting_tick,
        collateral_value: resulting_collateral_value,
        debt_value: resulting_debt_value, //actual debt
        interest_rate,
        volume_share,
        order_type: PositionOrderType::Market,
        timestamp: ic_cdk::api::time(), //change to time()
    };
    _insert_account_position(account, position);
    return Some((position, resulting_tick, crossed_ticks));
}

/// Close Long PositionDetails
///
///closes a user's  long position if position can be fully closed and  repays debt
///
/// Params
/// - User :The user (position owner)
/// - PositionDetails :The PositionDetails
/// - Current Tick :The current tick of market's state
/// - Stopping Tick : The max tick,corresponds to max price
/// - Vault : Vault canister
///
/// Returns
///  - Current Collateral :The amount to send to position owner after paying debt ,this amount is zero if debt is not fully paid
///  - Resulting Tick :The resulting tick from swapping
///  - Crosssed Ticks :An array of ticks that have been crossed during swapping
///   
/// Note
///  - If position can not be closed fully ,the position is partially closed (updated) and debt is paid back either fully or partially
fn _close_market_long_position(
    account: Subaccount,
    position: &mut PositionDetails,
    current_tick: Tick,
    stopping_tick: Tick,
) -> (Amount, Tick, Vec<Tick>, ManageDebtParams) {
    //
    let entry_price = _tick_to_price(position.entry_tick);
    let equivalent_at_entry_price =
        |amount: Amount, buy: bool| -> Amount { _equivalent(amount, entry_price, buy) };
    //
    let position_realised_value = _calc_position_realised_val(position.volume_share, true);
    // amount to swap

    let realised_position_size = equivalent_at_entry_price(position_realised_value, true);

    let (amount_out_value, amount_remaining, resulting_tick, crossed_ticks) =
        _swap(realised_position_size, false, current_tick, stopping_tick);

    let interest_value = _calc_interest(
        position.debt_value,
        position.interest_rate,
        position.timestamp,
    );

    let profit;

    let manage_debt_params;

    if amount_remaining > 0 {
        let amount_remaining_value = equivalent_at_entry_price(amount_remaining, false);
        //
        (profit, manage_debt_params) = _update_market_position_after_swap(
            position,
            resulting_tick,
            amount_out_value,
            amount_remaining_value,
            interest_value,
        );

        _insert_account_position(account, position.clone());
    } else {
        (profit, manage_debt_params) = (
            amount_out_value - (position.debt_value + interest_value),
            ManageDebtParams::init(0, position.debt_value, interest_value),
        );
        _remove_account_position(&account);
    }

    return (profit, resulting_tick, crossed_ticks, manage_debt_params);
}

///
/// Close Short Position
///
/// similar to Close Long Function,but for short positions
///
fn _close_market_short_position(
    account: Subaccount,
    position: &mut PositionDetails,
    current_tick: Tick,
    stopping_tick: Tick,
) -> (Amount, Tick, Vec<Tick>, ManageDebtParams) {
    let position_realised_value = _calc_position_realised_val(position.volume_share, false);

    let realised_position_size = position_realised_value;

    let (amount_out, amount_remaining_value, resulting_tick, crossed_ticks) =
        _swap(realised_position_size, true, current_tick, stopping_tick);

    let init_price = _tick_to_price(current_tick);

    // amount out value is calculated as the amount of collateral token used up in the swap
    let amount_out_value = _equivalent(amount_out, init_price, false); // position_realised_value - amount_remaining_value;

    let interest_value = _calc_interest(
        position.debt_value,
        position.interest_rate,
        position.timestamp,
    );

    let profit;
    let manage_debt_params: ManageDebtParams;

    if amount_remaining_value > 0 {
        (profit, manage_debt_params) = _update_market_position_after_swap(
            position,
            resulting_tick,
            amount_out_value,
            amount_remaining_value,
            interest_value,
        );

        _insert_account_position(account, position.clone());
    } else {
        (profit, manage_debt_params) = (
            amount_out_value - (position.debt_value + interest_value),
            ManageDebtParams::init(0, position.debt_value, interest_value),
        );
        // deletes user position
        _remove_account_position(&account);
    }

    return (profit, resulting_tick, crossed_ticks, manage_debt_params);
}

/// Close Limit Position
///
///
/// Closes a limit position at a particular tick by closing removing the limit order if the order is not filled
///
/// Params
///  - User : The owner of the position
///  - Position : The particular position to close
///  - Vault :The vault type representing the vault canister  

fn _close_limit_long_position(
    account: Subaccount,
    position: &mut PositionDetails,
) -> (Amount, ManageDebtParams) {
    match position.order_type {
        //
        PositionOrderType::Limit(order) => {
            let (amount_received, amount_remaining_value) = _close_order(&order);

            let (removed_collateral, manage_debt_params);

            if amount_received == 0 {
                (removed_collateral, manage_debt_params) = (
                    position.collateral_value,
                    ManageDebtParams::init(0, position.debt_value, 0),
                );

                _remove_account_position(&account);
            } else {
                (removed_collateral, manage_debt_params) =
                    _convert_limit_position(position, amount_remaining_value);
                //
                _insert_account_position(account, position.clone());
            };

            return (removed_collateral, manage_debt_params);
        }
        PositionOrderType::Market => (0, ManageDebtParams::default()),
    }
}

///
/// Close Limit Short Function
///
/// similar to close limit long position function but for long position
///
///
///

fn _close_limit_short_position(
    account: Subaccount,
    position: &mut PositionDetails,
) -> (Amount, ManageDebtParams) {
    match position.order_type {
        PositionOrderType::Limit(order) => {
            let (amount_received, amount_remaining) = _close_order(&order);

            let (removed_collateral, manage_debt_params);

            if amount_received == 0 {
                (removed_collateral, manage_debt_params) = (
                    position.collateral_value,
                    ManageDebtParams::init(0, position.debt_value, 0),
                );
                _remove_account_position(&account);
                //
            } else {
                let entry_price = _tick_to_price(position.entry_tick);

                let amount_remaining_value = _equivalent(amount_remaining, entry_price, false);
                (removed_collateral, manage_debt_params) =
                    _convert_limit_position(position, amount_remaining_value);
                // updates users positiion
                _insert_account_position(account, position.clone());
            };

            return (removed_collateral, manage_debt_params);
        }
        PositionOrderType::Market => return (0, ManageDebtParams::default()),
    }
}

/// Update Market Position After Swap Function
///
/// This function updates a  market position if it can not be closed i.e amount remaining after swapping to close position is greater than 0
///
/// It
///   - Updates the position debt ,the position collateral value , the position volume share
///   - Derives the update asset params that pays the debt either fully or partially
///
/// Params
///  - Position :A mutable reference to the particular position
///  - Resulting Tick : The resulting tick after swapping to closing the position
///  - Amount Out Value :The value of the amount gotten from swapping
///  - Amount Remaining Value :The value of the amount remaining after swapping
///  - Interest Value : The value of the interest accrued on current position debt
///
/// Returns
///  - Profit : The amount of profit for position owner or the amount of removable collateral from position
///  - Manage Debt Params : for repaying debt ,specifying the current debt and the previous debt and interest paid
///

fn _update_market_position_after_swap(
    position: &mut PositionDetails,
    resulting_tick: Tick,
    amount_out_value: Amount,
    amount_remaining_value: Amount,
    interest_value: Amount,
) -> (Amount, ManageDebtParams) {
    let initial_debt = position.debt_value;

    let total_fee_value = initial_debt + interest_value;

    let profit;
    let manage_debt_params;
    //
    if amount_out_value < total_fee_value {
        // if any interest can be given at all
        let interest_received_value = if amount_out_value > position.debt_value {
            amount_out_value - position.debt_value
        } else {
            0
        };
        //update new position details
        position.debt_value = total_fee_value - amount_out_value;

        manage_debt_params =
            ManageDebtParams::init(position.debt_value, initial_debt, interest_received_value);
        profit = 0;
    } else {
        position.debt_value = 0;
        position.collateral_value = amount_remaining_value;

        manage_debt_params = ManageDebtParams::init(0, position.debt_value, interest_value);
        profit = amount_out_value - total_fee_value;
    }

    let new_volume_share = _calc_position_volume_share(amount_remaining_value, position.long);
    //
    position.volume_share = new_volume_share;
    position.entry_tick = resulting_tick;

    // if position last time updated is greater than one hour ago ,position time is updated to current timestamp
    if position.timestamp + ONE_HOUR > ic_cdk::api::time() {
        position.timestamp = ic_cdk::api::time()
    }

    return (profit, manage_debt_params);
}

///
/// Convert Limit Position function
///
/// Converts a limit position into a market position after the reference limit order of that position has been filled fully or partially
/// any unfilled amount is refunded first as debt and if still remaining it is refunded back to the position owner and the position is updated to a market position
///
/// Params
///  - Position : A mutable reference to the cuurent position
///  - Amount Remaining Value : The value of the amount of  unfilled liquidity of the particular order
///
/// Returns
///  - Removed Collateral : The amount of collateral removed from that position
///  - Update Asset Details Params :The update asset details params for updating asset detailsin params   
///
fn _convert_limit_position(
    position: &mut PositionDetails,
    amount_remaining_value: Amount,
) -> (Amount, ManageDebtParams) {
    let remaining_order_value =
        position.collateral_value + position.debt_value - amount_remaining_value; // value of amount out
    let initial_debt = position.debt_value;
    //
    let removed_collateral;
    if amount_remaining_value > position.debt_value {
        removed_collateral = amount_remaining_value - position.debt_value;

        position.debt_value = 0;
        position.collateral_value -= removed_collateral;
    } else {
        removed_collateral = 0;

        position.debt_value -= amount_remaining_value;
    }
    let volume_share = _calc_position_volume_share(remaining_order_value, position.long);

    position.volume_share = volume_share;
    position.order_type = PositionOrderType::Market;
    position.timestamp = ic_cdk::api::time();

    let manage_debt_params = ManageDebtParams::init(position.debt_value, initial_debt, 0);

    return (removed_collateral, manage_debt_params);
}

///
/// Opens Order Functions
///
/// opens an order at a particular tick
///
/// Params
/// - Order :: a generic type that implements the trait Order for the type of order to close
/// - Reference Tick :: The  tick to place order

fn _open_order(order: &mut LimitOrder) {
    TICKS_DETAILS.with_borrow_mut(|ticks_details| {
        INTEGRAL_BITMAPS.with_borrow_mut(|integrals_bitmaps| {
            let mut open_order_params = OpenOrderParams {
                order,
                integrals_bitmaps,
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
fn _close_order(order: &LimitOrder) -> (Amount, Amount) {
    TICKS_DETAILS.with_borrow_mut(|ticks_details| {
        INTEGRAL_BITMAPS.with_borrow_mut(|multipliers_bitmaps| {
            let mut close_order_params = CloseOrderParams {
                order,
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
///  - resulting Tick :The last tick at which swap occured
///  - Crossed Ticks :: An vector of all ticks crossed during swap
fn _swap(
    order_size: Amount,
    buy: bool,
    init_tick: Tick,
    stopping_tick: Tick,
) -> (Amount, Amount, Tick, Vec<Tick>) {
    TICKS_DETAILS.with_borrow_mut(|ticks_details| {
        INTEGRAL_BITMAPS.with_borrow_mut(|integrals_bitmaps| {
            let mut swap_params = SwapParams {
                buy,
                init_tick,
                stopping_tick,
                order_size,
                integrals_bitmaps,
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
        None => {
            if buy {
                return current_tick + _percentage64(DEFAULT_SWAP_SLIPPAGE, current_tick);
            } else {
                return current_tick - _percentage64(DEFAULT_SWAP_SLIPPAGE, current_tick);
            }
        }
    }
    return _def_max_tick(current_tick, buy);
}

///
fn _get_market_details() -> MarketDetails {
    MARKET_DETAILS.with(|ref_market_details| ref_market_details.borrow().get().clone())
}
///
///
fn _get_state_details() -> StateDetails {
    STATE_DETAILS.with(|ref_state_detaills| *ref_state_detaills.borrow().get())
}
///
fn _update_state_details(new_state: StateDetails) {
    STATE_DETAILS.with(|ref_state_details| ref_state_details.borrow_mut().set(new_state).unwrap());
}

/// Get Account Position
///
/// Returns Account Position or Panics if account has no position
fn _get_account_position(account: &Subaccount) -> PositionDetails {
    ACCOUNTS_POSITION
        .with(|ref_position_details| ref_position_details.borrow().get(&account).unwrap())
}
///
///
/// Insert Account Position
///
/// Insert's New position for Account ,utilised when opening or updating a position
fn _insert_account_position(account: Subaccount, position: PositionDetails) {
    ACCOUNTS_POSITION
        .with(|ref_users_position| ref_users_position.borrow_mut().insert(account, position));
}
///
///
fn _remove_account_position(account: &Subaccount) {
    ACCOUNTS_POSITION.with(|ref_user_position| ref_user_position.borrow_mut().remove(account));
}

/// Get Account Error Log
///
/// Get's account's error log
fn _get_account_error_log(account: &Subaccount) -> PositionUpdateErrorLog {
    ACCOUNTS_ERROR_LOGS.with_borrow(|reference| reference.get(account).unwrap())
}

/// Insert Account Error
///
/// Insert's user error log ,User error log occurs during faled inter canister calls to repay debt and increase user's margin balance
fn _insert_account_error_log(account: Subaccount, error_log: PositionUpdateErrorLog) {
    ACCOUNTS_ERROR_LOGS.with_borrow_mut(|reference| reference.insert(account, error_log));
}

fn _remove_account_error_log(account: &Subaccount) {
    ACCOUNTS_ERROR_LOGS.with_borrow_mut(|reference| reference.remove(account));
}
///
/// Has No Position Or Pending Error Log
///
/// This function checks that an acccount currently has no opened position or any pending error log
fn _has_position_or_pending_error_log(account: &Subaccount) -> bool {
    ACCOUNTS_POSITION.with_borrow(|reference| reference.contains_key(account))
        || ACCOUNTS_ERROR_LOGS.with_borrow(|reference| reference.contains_key(account))
}
///
///Calculate Position Realised value
///
///Calculates the Realised value for a position's volume share in a particular market direction,Long or Short   
///
/// Note:This function also adjust's the volume share
fn _calc_position_realised_val(volume_share: Amount, long: bool) -> Amount {
    FUNDING_RATE_TRACKER.with_borrow_mut(|tr| {
        let mut funding_rate_tracker = tr.get().clone();
        //
        let value = funding_rate_tracker.remove_volume(volume_share, long);
        //
        tr.set(funding_rate_tracker).unwrap();
        value
    })
}

///
/// Calculate Position Volume Share
///
/// Calculates the volume share for a particular poistion volume in a market direction ,Long or Short
fn _calc_position_volume_share(position_value: Amount, long: bool) -> Amount {
    FUNDING_RATE_TRACKER.with_borrow_mut(|tr| {
        let mut funding_rate_tracker = tr.get().clone();
        //
        let value = funding_rate_tracker.add_volume(position_value, long);
        //
        tr.set(funding_rate_tracker).unwrap();
        value
    })
}
///
/// Calculate Position PNL
///
/// Calculates the current pnl in percentage  for a particular position
fn _calculate_position_pnl(position: PositionDetails) -> i128 {
    let equivalent = |amount: Amount, tick: Tick, buy: bool| {
        let tick_price = _tick_to_price(tick);
        _equivalent(amount, tick_price, buy)
    };

    let state_details = _get_state_details();

    let position_realised_value = _calc_position_realised_val(position.volume_share, position.long);

    if position.long {
        let init_position_value = (position.debt_value + position.collateral_value) as i128;

        let position_realised_size = equivalent(position_realised_value, position.entry_tick, true);

        let position_current_value =
            equivalent(position_realised_size, state_details.current_tick, false) as i128;

        let fee = _calc_interest(
            position.debt_value,
            position.interest_rate,
            position.timestamp,
        ) as i128;

        return ((position_current_value - fee - init_position_value)
            * (100 * _ONE_PERCENT as i128))
            / init_position_value;
    } else {
        let init_position_size = equivalent(
            position.debt_value + position.collateral_value,
            position.entry_tick,
            true,
        ) as i128;

        let debt_size = equivalent(position.debt_value, position.entry_tick, true);

        let fee = _calc_interest(debt_size, position.interest_rate, position.timestamp) as i128;

        let position_current_size =
            equivalent(position_realised_value, state_details.current_tick, true) as i128;

        return ((position_current_size - fee - init_position_size) * (100 * _ONE_PERCENT as i128))
            / init_position_size;
    }
}

/// Settle Funcding Rate
///
/// Settles Funding Rate by calling the XRC cansiter .fetching the Price ,calculating the premium and distributing the  fund to the right market direction,Long or Short
async fn settle_funding_rate() {
    let market_details = _get_market_details();

    let xrc = XRC::init(market_details.xrc_id);

    let request = GetExchangeRateRequest {
        base_asset: market_details.base_asset,
        quote_asset: market_details.quote_asset,
        timestamp: None,
    };

    match xrc._get_exchange_rate(request).await {
        Ok(rate_result) => {
            let state_details = _get_state_details();

            let current_price = _tick_to_price(state_details.current_tick);

            let perp_price =
                (current_price * 10u128.pow(rate_result.metadata.decimals)) / _BASE_PRICE;

            let spot_price = rate_result.rate as u128;

            _settle_funding_rate(perp_price, spot_price);
        }
        Err(_) => {
            return;
        }
    }
}

fn _settle_funding_rate(perp_price: u128, spot_price: u128) {
    let funding_rate = _calculate_funding_rate_premium(perp_price, spot_price);
    FUNDING_RATE_TRACKER.with_borrow_mut(|reference| {
        let mut funding_rate_tracker = reference.get().clone();

        funding_rate_tracker.settle_funding_rate(funding_rate.abs() as u64, funding_rate > 0);

        reference.set(funding_rate_tracker).unwrap();
    })
}

fn _calculate_funding_rate_premium(perp_price: u128, spot_price: u128) -> i64 {
    let funding_rate = ((perp_price as i128 - spot_price as i128) * 100 * _ONE_PERCENT as i128)
        / spot_price as i128;
    return funding_rate as i64;
}

/////////////////////////////////////////
/// System Functions
////////////////////////////////////////
#[ic_cdk::pre_upgrade]
fn pre_upgrade() {
    let multiplier_bitmaps =
        INTEGRAL_BITMAPS.with_borrow(|ref_mul_bitmaps| ref_mul_bitmaps.clone());
    //
    let ticks_details = TICKS_DETAILS.with_borrow(|ref_ticks_details| ref_ticks_details.clone());
    //
    storage::stable_save((multiplier_bitmaps, ticks_details)).expect("error storing data");
}

#[ic_cdk::post_upgrade]
fn post_upgrade() {
    let multiplier_bitmaps: HashMap<u64, u128>;

    let ticks_details: HashMap<Tick, TickDetails>;
    (multiplier_bitmaps, ticks_details) = storage::stable_restore().unwrap();
    INTEGRAL_BITMAPS.with(|ref_mul_bitmaps| *ref_mul_bitmaps.borrow_mut() = multiplier_bitmaps);

    TICKS_DETAILS.with(|ref_ticks_details| {
        *ref_ticks_details.borrow_mut() = ticks_details;
    })
}

//////////////////////////////////////////
/// Admin Functions
/////////////////////////////////////////
///
fn admin_guard() -> Result<(), String> {
    ADMIN.with_borrow(|admin_ref| {
        let admin = admin_ref.get();
        if ic_cdk::caller() == admin.principal_id {
            return Ok(());
        } else {
            return Err("Invalid".to_string());
        };
    })
}

#[ic_cdk::update(guard = "admin_guard", name = "updateStateDetails")]
async fn update_state_details(new_state_details: StateDetails) {
    _update_state_details(new_state_details);
}

#[ic_cdk::update(guard = "admin_guard", name = "startTimer")]
async fn start_timer() {
    ic_cdk_timers::set_timer_interval(Duration::from_secs(3600), || {
        ic_cdk::spawn(async { settle_funding_rate().await });
    });
}

/////////////////////////
///  Error Handling Functions
///////////////////////
fn trusted_canister_guard() -> Result<(), String> {
    let market_details = _get_market_details();

    let caller = ic_cdk::caller();

    if caller == market_details.vault_id || caller == market_details.watcher_id {
        return Ok(());
    } else {
        return Err("Untrusted Caller".to_string());
    }
}

#[ic_cdk::update(name = "retryError")]
async fn retry_error(index: usize) {
    let error = ERRORS.with_borrow(|reference| reference.get(index).unwrap().clone());

    let details = _get_market_details();
    //
    match error {
        ErrorType::ExecuteTicksOrderError(err) => err.retry(details),
        ErrorType::StoreTickOrderError(err) => err.retry(details),
        ErrorType::RemoveTickOrderError(err) => err.retry(details),
    };
}

#[ic_cdk::update(name = "retryAccountError")]
async fn retry_account_error(user: Principal) {
    let account = user._to_subaccount();

    let account_error_log = _get_account_error_log(&account);

    let details = _get_market_details();
    account_error_log.retry(details);
}

#[ic_cdk::update(name = "successNotification", guard = "trusted_canister_guard")]
async fn success_notif(account: Subaccount, error_index: usize) {
    let market_details = _get_market_details();

    let caller = ic_cdk::caller();

    if caller == market_details.vault_id {
        _remove_account_error_log(&account);
        return;
    }

    if caller == market_details.watcher_id {
        ERRORS.with_borrow_mut(|reference| reference.remove(error_index));
    }
}

//

#[derive(CandidType, Deserialize, Debug, Serialize, Clone, Copy)]
enum OrderType {
    Market,
    Limit,
}

#[derive(CandidType, Deserialize, Debug, Serialize, Clone, Copy)]
enum PositionOrderType {
    Market,
    Limit(LimitOrder),
}

#[derive(CandidType, Deserialize, Debug, Clone, Copy)]
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
    debt_value: Amount,
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
    ///Order Type
    ///
    ///Position Order  type can either be a
    ///
    /// Market
    ///  - This is when position is opened instantly at the current price
    ///
    /// Order
    ///   - This comprises of an order set at a particular tick and position is only opened when
    ///   that  order has been executed
    order_type: PositionOrderType,

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

    const MAX_SIZE: u32 = 200;
}

//
/// ManageDebtParams is utilised to handle debt handling and  repayment

#[derive(Copy, Clone, Default, Deserialize, CandidType)]
struct ManageDebtParams {
    new_debt: Amount,
    initial_debt: Amount,
    interest_received: Amount,
}

impl ManageDebtParams {
    fn init(new_debt: Amount, initial_debt: Amount, interest_received: Amount) -> Self {
        ManageDebtParams {
            new_debt,
            initial_debt,
            interest_received,
        }
    }
}

/////////////////////////////
///   Possible error during inter canister calls and retry api
////////////////////////////

/// Retrying Trait
///
/// Trait for all Errors related to inter canister calls
trait Retrying {
    /// Retry  Function
    ///
    /// This is used to retry the  failed inter canister call
    fn retry(&self, details: MarketDetails);
}

/// ManageDebtError
///
/// This error occurs for failed intercanister calls

#[derive(Clone, Copy, Deserialize, CandidType)]
struct PositionUpdateErrorLog {
    user: Principal,
    profit: Amount,
    debt_params: ManageDebtParams,
}
impl Retrying for PositionUpdateErrorLog {
    fn retry(&self, details: MarketDetails) {
        let _ = ic_cdk::notify(
            details.vault_id,
            "managePositionUpdate",
            (self.user, self.profit, self.debt_params),
        );
    }
}

impl Storable for PositionUpdateErrorLog {
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
}

impl BoundedStorable for PositionUpdateErrorLog {
    const IS_FIXED_SIZE: bool = true;

    const MAX_SIZE: u32 = 130;
}

#[derive(Clone)]
enum ErrorType {
    ExecuteTicksOrderError(ExecuteTicksError),
    StoreTickOrderError(StoreTickOrderError),
    RemoveTickOrderError(RemoveTickOrderError),
}

#[derive(Clone)]
struct ExecuteTicksError {
    ticks: Vec<Tick>,
}

impl Retrying for ExecuteTicksError {
    fn retry(&self, details: MarketDetails) {
        let _ = ic_cdk::notify(
            details.watcher_id,
            "executeTicksOrders",
            (self.ticks.clone(),),
        );
    }
}

#[derive(Clone, Copy)]
struct StoreTickOrderError {
    account: Subaccount,
    tick: Tick,
}

impl Retrying for StoreTickOrderError {
    fn retry(&self, details: MarketDetails) {
        let _ = ic_cdk::notify(
            details.watcher_id,
            "storeTicksOrder
            ",
            (self.account, self.tick),
        );
    }
}

#[derive(Clone, Copy)]
struct RemoveTickOrderError {
    account: Subaccount,
    tick: Tick,
}

impl Retrying for RemoveTickOrderError {
    fn retry(&self, details: MarketDetails) {
        let _ = ic_cdk::notify(
            details.watcher_id,
            "removeTickOrder",
            (self.account, self.tick),
        );
    }
}

/// Exchange Rate Canister
///
/// Utilised for fetching the price of current exchnage rate (spot price) of the market pair

struct XRC {
    canister_id: Principal,
}

impl XRC {
    fn init(canister_id: Principal) -> Self {
        XRC { canister_id }
    }

    /// tries to fetche the current exchange rate of the pair and returns the result
    async fn _get_exchange_rate(&self, request: GetExchangeRateRequest) -> GetExchangeRateResult {
        if let Ok((rate_result,)) = ic_cdk::api::call::call_with_payment128(
            self.canister_id,
            "get_exchange_rate",
            (request,),
            1_000_000_000,
        )
        .await
        {
            return rate_result;
        } else {
            panic!()
        }
    }
}

///
/// Watcher type
///
/// Represents the Watcher canister
///
/// The Watcher Canister helps in execution of positions with limit order type  by converting them to market order when the order's reference tick has been closed  

struct Watcher {
    canister_id: Principal,
}

impl Watcher {
    pub fn init(canister_id: Principal) -> Self {
        return Watcher { canister_id };
    }

    /// Store Tick Order
    ///
    /// Stores an order under a particular tick
    ///
    /// Utilised when positions are opened as limit orders
    ///
    ///
    /// - Tick    :The tickat which order is placed
    /// - Account : The account opening the position

    pub fn store_tick_order(&self, tick: Tick, account: Subaccount) {
        if let Ok(()) = ic_cdk::notify(self.canister_id, "storeTickOrder", (tick, account)) {
        } else {
            ERRORS.with_borrow_mut(|reference| {
                let error = ErrorType::StoreTickOrderError(StoreTickOrderError { account, tick });
                reference.push(error);
            })
        }
    }

    /// Remove Tick Order
    ///
    /// Removes an order under a particular tick
    ///
    /// Utilised when account owner closes a limit position before reference tick is fully crossed
    ///
    /// - Tick    :The tickat which order was placed
    /// - Account : The account closing the position

    pub fn remove_tick_order(&self, tick: Tick, account: Subaccount) {
        if let Ok(()) = ic_cdk::notify(self.canister_id, "removeTickOrder", (tick, account)) {
        } else {
            ERRORS.with_borrow_mut(|reference| {
                let error = ErrorType::RemoveTickOrderError(RemoveTickOrderError { account, tick });
                reference.push(error);
            })
        }
    }

    /// Execute Ticks Orders
    ///
    /// Notifies Watcher to execute all orders placed at those tick respectively
    ///
    /// Ticks:  An array of ticks crossed during the swap (meaning all orders at those tick has been filled)

    pub fn execute_ticks_orders(&self, ticks: Vec<Tick>) {
        if ticks.len() == 0 {
            return;
        };

        if let Ok(()) = ic_cdk::notify(self.canister_id, "executeTicksOrders", (ticks.clone(),)) {
        } else {
            ERRORS.with_borrow_mut(|reference| {
                let error = ErrorType::ExecuteTicksOrderError(ExecuteTicksError { ticks });
                reference.push(error);
            })
        }
    }
}

/// The Vault type representing vault canister that stores asset for the entire collateral's denominated market
/// it facilitates all movement of assets including collection and repayment of debt utilised for leverage

#[derive(Clone, Copy)]
struct Vault {
    canister_id: Principal,
}

impl Vault {
    // initialises the vault canister
    pub fn init(canister_id: Principal) -> Self {
        Vault { canister_id }
    }

    /// Manage Position Update
    ///
    /// Utilised when position is updated or closed
    /// Utilised when for updating user_balance,repayment of debt
    pub fn manage_position_update(
        &self,
        user: Principal,
        profit: Amount,
        manage_debt_params: ManageDebtParams,
    ) {
        if let Ok(()) = ic_cdk::notify(
            self.canister_id,
            "managePositionUpdate",
            (user, profit, manage_debt_params),
        ) {
        } else {
            let error_log = PositionUpdateErrorLog {
                user,
                profit,
                debt_params: manage_debt_params,
            };
            _insert_account_error_log(user._to_subaccount(), error_log);
        }
    }

    /// Create Position Validity Check
    ///
    /// Checks if position can be opened by checking that uswer has sufficient balance and amount to use as debt is available as free liquidity
    ///
    /// User:The Owner of Account that opened position
    /// Collateral Delta:The Amount of asset used as collateral for opening position
    /// Debt : The Amount of asset taken as debt
    ///
    /// Note :After checking that the condition holds ,the user balance is reduced by collateral amount and the free liquidity available is reduced by debt amount

    pub async fn create_position_validity_check(
        &self,
        user: Principal,
        collateral: Amount,
        debt: Amount,
    ) -> (bool, u32) {
        if let Ok((valid, interest_rate)) = ic_cdk::call(
            self.canister_id,
            "createPositionValidityCheck",
            (user, collateral, debt),
        )
        .await
        {
            return (valid, interest_rate);
        } else {
            return (false, 0);
        }
    }
}

trait UniqueSubAccount {
    const NONCE: u8;
    fn _to_subaccount(&self) -> Subaccount;
}

impl UniqueSubAccount for Principal {
    const NONCE: u8 = 1;
    fn _to_subaccount(&self) -> Subaccount {
        let mut hasher = Sha256::new();
        hasher.update(self.as_slice());
        hasher.update(&Principal::NONCE.to_be_bytes());
        let hash = hasher.finalize();
        let mut subaccount = [0u8; 32];
        subaccount.copy_from_slice(&hash[..32]);
        subaccount
    }
}

export_candid!();

pub mod corelib;
pub mod types;

#[cfg(test)]
pub mod integration_tests;
