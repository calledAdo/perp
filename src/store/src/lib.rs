use candid::{CandidType, Principal};

use serde::Deserialize;

use std::cell::{Cell, RefCell};
use std::collections::HashMap;

type Amount = u128;
type Tick = u64;
type Time = u64;

thread_local! {
    static MARKET:Cell<Principal> = Cell::new(Principal::anonymous());

    static USERS_POSITION :RefCell<HashMap<Principal,Position>> = RefCell::new(HashMap::new());

    static USERS_LIQUIDITY_ORDER :RefCell<HashMap<Principal,Vec<LiquidityOrder>>> = RefCell::new(HashMap::new());

}

#[ic_cdk::init]
fn init(market: Principal) {
    MARKET.with(|ref_market| {
        ref_market.set(market);
    })
}

#[ic_cdk::update(name = "putPosition")]
fn put_position(user: Principal, position: Position) {
    USERS_POSITION.with(|ref_user_position| ref_user_position.borrow_mut().insert(user, position));
}

#[ic_cdk::update(name = "removePosition")]
async fn remove_position(user: Principal) {
    USERS_POSITION.with(|ref_user_position| ref_user_position.borrow_mut().remove(&user));
}

#[ic_cdk::query(name = "userHasPosition")]
async fn user_has_position(user: Principal) -> bool {
    USERS_POSITION.with(|ref_user_position| ref_user_position.borrow().contains_key(&user))
}
#[ic_cdk::query(name = "getUserPosition")]
async fn get_user_position(user: Principal) -> Position {
    USERS_POSITION.with(|ref_user_position| ref_user_position.borrow().get(&user).unwrap().clone())
}

#[derive(Default, CandidType, Deserialize, Copy, Clone)]
struct TradeOrder {
    pub amount_in: Amount,
    pub init_upper_bound: Amount,
    pub buy: bool,
    pub ref_tick: Tick,
    pub tick_cross_time: Time,
}

#[derive(Default, CandidType, Copy, Clone)]
pub struct LiquidityOrder {
    pub amount_in: Amount,
    pub liq_shares: Amount,
    pub reference_tick: Tick,
    pub buy: bool,
}

#[derive(CandidType, Deserialize, Copy, Clone)]
struct Position {
    amount_in: Amount,
    debt: Amount,
    interest_rate: u32,
    time_stamp: Time,
}

ic_cdk::export_candid!();
