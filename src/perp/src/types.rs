use crate::corelib::calc_lib::{_calc_shares, _calc_shares_value};
use candid::{CandidType, Decode, Encode, Principal};
use ic_stable_structures::{storable::BoundedStorable, Storable};

use serde::Deserialize;
use std::borrow::Cow;
pub type Tick = u64;
pub type Amount = u128;
type Time = u64;

// user opens position
//the mount_in
//debt_value in collateral

#[derive(CandidType, Deserialize, Clone, Copy)]
pub struct FundingRateTracker {
    pub net_volume_long: Amount,
    pub total_long_shares: Amount,
    pub net_volume_short: Amount,
    pub total_short_shares: Amount,
}

impl FundingRateTracker {
    pub fn add_volume(&mut self, delta: Amount, long: bool) -> Amount {
        if long {
            let volume_share = _calc_shares(delta, self.total_long_shares, self.net_volume_long);
            self.total_long_shares += volume_share;
            self.net_volume_long += delta;
            return volume_share;
        } else {
            let volume_share = _calc_shares(delta, self.total_short_shares, self.net_volume_short);
            self.total_short_shares += volume_share;
            self.net_volume_short += delta;
            return volume_share;
        }
    }

    pub fn remove_volume(&mut self, delta: Amount, long: bool) -> Amount {
        if long {
            let value = _calc_shares_value(delta, self.total_long_shares, self.net_volume_long);
            self.net_volume_long -= value;
            self.total_long_shares -= delta;
            return value;
        } else {
            let value = _calc_shares_value(delta, self.total_short_shares, self.net_volume_short);
            self.net_volume_short -= value;
            self.total_short_shares -= delta;
            return value;
        }
    }
}

impl Storable for FundingRateTracker {
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
}

impl Default for FundingRateTracker {
    fn default() -> Self {
        FundingRateTracker {
            net_volume_long: 0,
            total_long_shares: 0,
            net_volume_short: 0,
            total_short_shares: 0,
        }
    }
}

impl BoundedStorable for FundingRateTracker {
    const IS_FIXED_SIZE: bool = true;

    const MAX_SIZE: u32 = 64;
}

#[derive(CandidType, Deserialize, PartialEq, Eq, PartialOrd, Ord, Copy, Clone)]
pub struct ID(pub Principal);
impl ID {
    pub fn from(principal: Principal) -> Self {
        return ID(principal);
    }
}

impl Storable for ID {
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
}

impl BoundedStorable for ID {
    const IS_FIXED_SIZE: bool = true;

    const MAX_SIZE: u32 = 30;
}

#[derive(CandidType, Deserialize, Clone, Copy)]
/// Asset Class for querying the exchange rate canister
enum AssetClass {
    FiatCurrency,
    Cryptocurrency,
}

impl Default for AssetClass {
    fn default() -> Self {
        AssetClass::Cryptocurrency
    }
}

/// Asset type
#[derive(CandidType, Default, Deserialize, Clone, Copy)]
pub struct Asset {
    /// symbol in utf-8 encoding arrays
    symbol: [u8; 3],
    /// asset class of particular asset
    asset_class: AssetClass,
}

///Market Details
#[derive(CandidType, Deserialize, Clone, Copy)]
pub struct MarketDetails {
    /// The details of the  perpetual asset also seen as the base asset  
    pub perp_asset_details: Asset,

    /// the details of the collateral token  in asset  all margin is paid
    pub collateral_asset_details: Asset,
    /// the principal  of collateral or margin token
    pub collateral_asset: ID,

    pub vault_id: ID,

    pub watcher_id: ID,
    /// token decimal of collateral token
    pub collateral_decimal: u8,
}

impl Default for MarketDetails {
    fn default() -> MarketDetails {
        return MarketDetails {
            perp_asset_details: Asset::default(),
            collateral_asset_details: Asset::default(),
            collateral_asset: ID(Principal::anonymous()),
            vault_id: ID(Principal::anonymous()),
            watcher_id: ID(Principal::anonymous()),
            collateral_decimal: 0,
        };
    }
}

impl BoundedStorable for MarketDetails {
    const IS_FIXED_SIZE: bool = true;

    const MAX_SIZE: u32 = 40;
}

impl Storable for MarketDetails {
    fn from_bytes(bytes: std::borrow::Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
}

///State Details comprises for useful parameters that change more frequently
/// compared to market details  that is  set on initialization

#[derive(CandidType, Default, Copy, Deserialize, Clone)]
pub struct StateDetails {
    /// Current Tick
    pub current_tick: Tick,
    /// Max Leverage
    ///
    /// the maximum leverage allowed for any position * 10
    ///
    /// typically leverage is set multiplied by 10 ,so a leverage of 2x would be written as 20  
    pub max_leveragex10: u8,
    /// Minimum Collateral
    ///
    /// minimum collateral or minimum margin to open a position
    ///
    /// Note:
    ///
    /// -this amount  is actuallly  reduced by the reduction i.e (10::pow(token_decimal - 6))
    pub min_collateral: Amount,
    /// Interest Rate
    ///
    /// interest rate paid for holding an executed a position
    ///
    /// Note :
    /// - order position type do not pay interest untile transaction is executed
    pub interest_rate: u32,
    /// Base Token Multiplier
    ///
    /// base token multiple for cases of perp_assets with lower value than the underlying collateral asset
    pub base_token_multiple: u8,
}

impl BoundedStorable for StateDetails {
    const IS_FIXED_SIZE: bool = true;
    const MAX_SIZE: u32 = 41;
}

impl Storable for StateDetails {
    fn from_bytes(bytes: Cow<[u8]>) -> Self {
        Decode!(bytes.as_ref(), Self).unwrap()
    }

    fn to_bytes(&self) -> Cow<[u8]> {
        Cow::Owned(Encode!(self).unwrap())
    }
}

#[derive(CandidType, Default, Deserialize, Clone, Copy)]
pub struct TickDetails {
    /// Liquidity Token0
    ///
    /// total amount of liquidity in token0 available at tick
    pub liq_token0: Amount,
    /// Liquidity Token1
    ///
    /// total amount of liquidity in token1 available at tick
    pub liq_token1: Amount,
    /// Total Shares
    ///
    /// total shares of al liqudiity providers with liquidity in this tick
    pub total_shares: Amount,
    /// Liquidity Boundary Token0
    ///
    /// liquidity bounds of token0 see (LiquidityBoundary)
    pub liq_bounds_token0: LiquidityBoundary,
    /// Liquidity Boundary Token1
    ///
    /// liquidity bounds of token1
    pub liq_bounds_token1: LiquidityBoundary,
    /// Tick Cross Time
    /// the last time crossed tracks the the last time static liquidity
    /// was completely exhausted at tick
    pub crossed_time: Time,
}

impl TickDetails {
    /// Add_liquidity function
    ///
    /// adds liquidity at current particular tick
    pub fn _add_liquidity(
        &mut self,
        buy: bool,
        amount_in: Amount,
        delta_total_shares: Amount,
        delta_liq_bound: Amount,
    ) {
        if buy {
            self.liq_bounds_token1._add_liquidity(delta_liq_bound);
            // increases token1 for a buy order
            self.liq_token1 += amount_in;
        } else {
            self.liq_bounds_token0._add_liquidity(delta_liq_bound);
            // increases token1 for a buy order
            self.liq_token0 += amount_in;
        }
        // increases shares in case order is a liquidity order
        self.total_shares += delta_total_shares;
    }

    /// Reemove_liquidity function
    ///
    /// removes liquidity from the reference tick
    ///
    /// Note;This is only called while closing trade orders and retrieving static liqudiity
    /// it's not called  while closing liquidity orders
    pub fn _remove_liquidity(&mut self, buy: bool, amount_out: Amount) {
        if buy {
            self.liq_bounds_token1._remove_liquidity(amount_out);
            self.liq_token1 -= amount_out;
        } else {
            self.liq_bounds_token0._remove_liquidity(amount_out);
            self.liq_token0 -= amount_out;
        }
    }
}

/// Liquidity Boundary tracks the amount of Static Liquidity currently at a tick
///
///   Static Liquidity refers to liquidity from  limit orders that normal traders make
///
///   while Dynamic liquidity refers to liquidity provided by orders from liquidity providers  
///
///   Dynamic because it changes form with the same tick ,going from a buy order to a sell order and vice versa

#[derive(CandidType, Deserialize, Default, Copy, Clone, PartialEq, Eq)]
pub struct LiquidityBoundary {
    /// Upper Bound
    ///
    /// upper bound of all static liquidity put into the reference tick since it's (last time crossed)
    ///
    /// Note :this includes those closed or cancelled
    pub upper_bound: Amount,
    /// Lower Boound
    ///
    /// lower bound of all static liquidity put into the reference tick since it's (last time crossed)
    ///
    /// Note:
    ///
    ///  - Lower bound tracks the amouunt of asset static liquidity utilised
    ///
    ///  - the amouunt  of dynamic liquidity at a current tick is the upper bound - lower bound
    pub lower_bound: Amount,
    /// Removed Liquidity
    ///
    /// total removed static liquidity since last time crossed
    pub removed_liquidity: Amount,
}

impl LiquidityBoundary {
    /// Reduce boundary function
    ///
    /// reduces the boundary by adding amount and the total removed liquidity to lower bound
    ///
    /// setting the removed liquidity to zero
    pub fn _reduce_boundary(&mut self, amount: Amount) {
        self.lower_bound += self.removed_liquidity + amount;
        if self.lower_bound > self.upper_bound {
            self.lower_bound = self.upper_bound
        };
        self.removed_liquidity = 0;
    }

    /// LLiquidity within
    ///
    /// calculates liquidity within a boundary
    pub fn _liquidity_within(&self) -> Amount {
        return self.upper_bound - self.lower_bound - self.removed_liquidity;
    }
    /// Add Liqudity
    ///
    /// adds liquidity to boundary to a boundary by increasing the boundary upper bound by delta
    pub fn _add_liquidity(&mut self, delta: Amount) {
        self.upper_bound += delta;
    }

    /// Remove Liqudity
    ///
    /// removes liquidity from within a boundary be increasing removed liquidity
    pub fn _remove_liquidity(&mut self, delta: Amount) {
        self.removed_liquidity += delta;
    }
}
