use candid::{CandidType, Deserialize, Principal};

use sha2::{Digest, Sha256};

use std::cell::RefCell;
use types::{StakeDetails, StakeSpan, Token, VaultDetails};

type Amount = u128;
type Time = u64;

use icrc_ledger_types::icrc1::account::{Account, Subaccount};

use ic_stable_structures::memory_manager::{MemoryId, MemoryManager, VirtualMemory};

use ic_stable_structures::{DefaultMemoryImpl, StableBTreeMap, StableCell};

type Memory = VirtualMemory<DefaultMemoryImpl>;

const _VAULT_DETAILS_MEMORY: MemoryId = MemoryId::new(1);
const _USERS_STAKES_DETAILS_MEMORY: MemoryId = MemoryId::new(2);
const _USERS_BALANCE_MEMORY: MemoryId = MemoryId::new(3);

thread_local! {

    static MEMORY_MANAGER:RefCell<MemoryManager<DefaultMemoryImpl>> = RefCell::new(MemoryManager::init(DefaultMemoryImpl::default())) ;


    static VAULT_DETAILS :RefCell<StableCell<VaultDetails,Memory>> = RefCell::new(StableCell::init(MEMORY_MANAGER.with_borrow(|reference|{
        reference.get(_VAULT_DETAILS_MEMORY)
    }),VaultDetails::default()).unwrap());


    static USERS_MARGIN_BALANCE :RefCell<StableBTreeMap<Subaccount,Amount,Memory>> = RefCell::new(StableBTreeMap::init(
        MEMORY_MANAGER.with_borrow(|reference|{
        reference.get(_USERS_BALANCE_MEMORY)
    })));


    static USERS_STAKES :RefCell<StableBTreeMap<(Subaccount,Time),StakeDetails,Memory>> = RefCell::new(StableBTreeMap::init(
        MEMORY_MANAGER.with_borrow(|reference|{
        reference.get(_USERS_STAKES_DETAILS_MEMORY)
    })));

}

#[ic_cdk::init]
fn init(vault_details: VaultDetails) {
    VAULT_DETAILS.with_borrow_mut(|reference| reference.set(vault_details).unwrap());
}

#[ic_cdk::query]
fn get_user_account(user: Principal) -> Account {
    return Account {
        owner: ic_cdk::id(),
        subaccount: Some(user._to_subaccount()),
    };
}

#[ic_cdk::update(name = "createPositionValidityCheck")]
async fn create_position_validity_check(
    user: Principal,
    collateral: Amount,
    debt: Amount,
) -> (bool, u32) {
    let account = user._to_subaccount();

    let account_balance = _get_user_balance(account);

    let mut vault_details = _get_vault_details();

    let valid = account_balance >= collateral && vault_details.free_liquidity >= debt;

    if valid {
        vault_details.free_liquidity -= debt;
        vault_details.debt += debt;
        _update_user_margin_balance(account, collateral, false);
    }

    return (valid, 0);
}

#[ic_cdk::update(name = "managePositionUpdate")]
async fn manage_position_update(
    user: Principal,
    margin_delta: Amount,
    manage_debt_params: ManageDebtParams,
) {
    if margin_delta != 0 {
        let account = user._to_subaccount();
        _update_user_margin_balance(account, margin_delta, true);
    }

    let mut vault_details = _get_vault_details();

    vault_details.debt += manage_debt_params.new_debt - manage_debt_params.initial_debt;
    vault_details.free_liquidity += manage_debt_params.interest_received
        + manage_debt_params.initial_debt
        - manage_debt_params.new_debt;
    vault_details.lifetime_fees += manage_debt_params.interest_received;

    if manage_debt_params.interest_received == 0 {
        return;
    }

    {
        vault_details
            .staking_details
            ._create_stake(0, vault_details.lifetime_fees, StakeSpan::None)
    };
    {
        vault_details.staking_details._create_stake(
            0,
            vault_details.lifetime_fees,
            StakeSpan::Month2,
        )
    };
    {
        vault_details.staking_details._create_stake(
            0,
            vault_details.lifetime_fees,
            StakeSpan::Month6,
        )
    };
    {
        vault_details
            .staking_details
            ._create_stake(0, vault_details.lifetime_fees, StakeSpan::Year)
    };
}

/// Funds a Traders margin account to make a thread
///
///
///
///
#[ic_cdk::update]
async fn fund_margin_account(amount: Amount, for_principal: Principal) {
    let vault_details = _get_vault_details();
    assert!(amount >= vault_details.min_amount);

    let account = ic_cdk::caller()._to_subaccount();

    let token = Token::init(vault_details.asset.principal_id);
    if token.move_asset(amount, Some(account), None).await {
        let receiver = for_principal._to_subaccount();
        _update_user_margin_balance(receiver, amount, true);
    }
}

/// Withdraw From Trade Account
///
/// moves funds from users trade balance to users funding account

#[ic_cdk::update]
async fn withdraw_from_margin_account(amount: Amount) {
    let user = ic_cdk::caller()._to_subaccount();

    let vault_details = _get_vault_details();

    let amount_to_withdraw = if vault_details.min_amount > amount {
        _get_user_balance(user)
    } else {
        amount
    };

    let tx_fee = vault_details.tx_fee;
    if amount_to_withdraw - tx_fee == 0 {
        return;
    }

    let token = Token::init(vault_details.asset.principal_id);
    if token
        .move_asset(amount_to_withdraw - tx_fee, None, Some(user))
        .await
    {
        _update_user_margin_balance(user, amount_to_withdraw, false);
    }
}

///////////////////////////
///  Stakers Functions
//////////////////////////

///
///
///  Provide Leverage Function
///
/// For providing leverage for
#[ic_cdk::update]
async fn provide_leverage(amount: Amount) {
    let user = ic_cdk::caller()._to_subaccount();
    //
    let mut vault_details = _get_vault_details();

    assert!(amount >= vault_details.min_amount);

    let token = Token::init(vault_details.asset.principal_id);

    if !(token.move_asset(amount, Some(user), None).await) {
        return;
    }

    let vtoken = Token::init(vault_details.virtaul_asset.principal_id);
    // minting asset to user
    if !(vtoken.move_asset(amount, None, Some(user)).await) {
        token.move_asset(amount, None, Some(user)).await;
        return;
    }
    vault_details.free_liquidity += amount;

    let stake = vault_details.staking_details._create_stake(
        amount,
        vault_details.lifetime_fees,
        StakeSpan::None,
    );
    _insert_user_stake(user, stake);
    _update_vault_details(vault_details);
}

/// Subaccount
///
///
///
///
#[ic_cdk::update]
async fn remove_leverage(amount: Amount) {
    let user = ic_cdk::caller()._to_subaccount();
    let mut vault_details = _get_vault_details();

    assert!(amount >= vault_details.min_amount);
    // if tokens are not much
    if vault_details.free_liquidity < amount {
        return;
    }

    let vtoken = Token::init(vault_details.virtaul_asset.principal_id);
    // minting asset to user
    if !vtoken.move_asset(amount, Some(user), None).await {
        return;
    }

    let token = Token::init(vault_details.asset.principal_id);

    let tx_fee = vault_details.tx_fee;

    if !(token.move_asset(amount - tx_fee, None, Some(user)).await) {
        // if asset can't e sent back
        // mint back
        vtoken.move_asset(amount, None, Some(user)).await;
        return;
    }

    vault_details.free_liquidity -= amount;
    _update_vault_details(vault_details);
}

/// Stake for a Particular Duration
///
///
///
#[ic_cdk::update]
async fn stake(amount: Amount, stake_span: StakeSpan) {
    if let StakeSpan::None = stake_span {
        return;
    };
    let user = ic_cdk::caller()._to_subaccount();
    let mut vault_details = _get_vault_details();

    assert!(amount >= vault_details.min_amount);

    let vtoken = Token::init(vault_details.virtaul_asset.principal_id);
    // send in asset from user to account
    assert!(
        vtoken
            .move_asset(amount, Some(user), Some(_vault_subaccount()))
            .await
    );

    let stake = vault_details.staking_details._create_stake(
        amount,
        vault_details.lifetime_fees,
        stake_span,
    );

    _insert_user_stake(user, stake);
    _update_vault_details(vault_details);
}

#[ic_cdk::update]
async fn unstake(stake_timestamp: Time) -> Result<Amount, String> {
    let user = ic_cdk::caller()._to_subaccount();
    let ref_stake = _get_user_stake(user, stake_timestamp);

    if ic_cdk::api::time() < ref_stake.expiry_time {
        return Err("Expiry time in the future".to_string());
    };

    let mut vault_details = _get_vault_details();

    let amount_out = vault_details
        .staking_details
        ._close_stake(ref_stake, vault_details.lifetime_fees);

    let vtoken = Token::init(vault_details.virtaul_asset.principal_id);

    if !(vtoken
        .move_asset(amount_out, Some(_vault_subaccount()), Some(user))
        .await)
    {
        return Err("failed".to_string());
    }
    _remove_user_stake(user, stake_timestamp);
    _update_vault_details(vault_details);

    return Ok(amount_out);
}

/// Update user balance

fn _update_user_margin_balance(user: [u8; 32], delta: Amount, deposit: bool) {
    USERS_MARGIN_BALANCE.with_borrow_mut(|reference| {
        let initial_balance = { reference.get(&user).or(Some(0)).unwrap() };
        let new_balance = if deposit {
            initial_balance + delta
        } else {
            initial_balance - delta
        };
        if new_balance == 0 {
            reference.remove(&user)
        } else {
            reference.insert(user, new_balance)
        }
    });
}

fn _get_vault_details() -> VaultDetails {
    VAULT_DETAILS.with(|reference| reference.borrow().get().clone())
}

fn _update_vault_details(new_details: VaultDetails) {
    VAULT_DETAILS.with_borrow_mut(|reference| [reference.set(new_details).unwrap()]);
}

fn _get_user_balance(user: [u8; 32]) -> Amount {
    USERS_MARGIN_BALANCE.with_borrow_mut(|reference| {
        return reference.get(&user).or(Some(0)).unwrap();
    })
}

fn _insert_user_stake(user: Subaccount, stake: StakeDetails) {
    let timestamp = ic_cdk::api::time();
    USERS_STAKES.with_borrow_mut(|reference| reference.insert((user, timestamp), stake));
}

fn _remove_user_stake(user: Subaccount, timestamp: Time) {
    USERS_STAKES.with_borrow_mut(|reference| reference.remove(&(user, timestamp)));
}

fn _get_user_stake(user: Subaccount, timestamp: Time) -> StakeDetails {
    USERS_STAKES.with_borrow(|reference| reference.get(&(user, timestamp)).unwrap())
}

#[derive(Copy, Clone, Default, Deserialize, CandidType)]
struct ManageDebtParams {
    new_debt: Amount,
    initial_debt: Amount,
    interest_received: Amount,
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

fn _vault_subaccount() -> Subaccount {
    let canister_id = ic_cdk::caller();
    return canister_id._to_subaccount();
}

pub mod types;
