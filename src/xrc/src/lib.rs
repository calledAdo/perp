pub mod types;
use candid::Principal;
use ic_cdk::export_candid;
use types::{Asset, AssetClass, GetExchangeRateRequest, GetExchangeRateResult};

#[ic_cdk::update]
async fn call(
    base_asset_symbol: String,
    quote_asset_symbol: String,
) -> Option<GetExchangeRateResult> {
    let base_asset = Asset {
        symbol: base_asset_symbol,
        class: AssetClass::Cryptocurrency,
    };

    let quote_asset = Asset {
        symbol: quote_asset_symbol,
        class: AssetClass::Cryptocurrency,
    };

    let request = GetExchangeRateRequest {
        base_asset,
        quote_asset,
        timestamp: None,
    };
    if let Ok((res,)) = ic_cdk::api::call::call_with_payment128(
        Principal::from_text("uf6dk-hyaaa-aaaaq-qaaaq-cai").unwrap(),
        "get_exchange_rate",
        (request,),
        1_000_000_000,
    )
    .await
    {
        return Some(res);
    };
    return None;
}

export_candid!();

#[test]
fn test() {
    println!("{}", 2_994_950_530_978u128 - 2_994_445_760_471u128)
}

//504_770_507

// 2_994_950_530_978
// 2_994_445_760_471
