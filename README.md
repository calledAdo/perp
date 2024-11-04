be2us-64aaa-aaaaa-qaabq-cai

:
Funding Rate= Base Rate + ((Total Short Positions − Total Long Positions)/Total Open Interest)\*Sensitivity

```bash


dfx start --clean

```

### Stop the replica

```bash
dfx stop
```

### Build canister into Wasm

```bash
cargo build --release --target wasm32-unknown-unknown --package xrc

candid-extractor target/wasm32-unknown-unknown/release/xrc.wasm > src/xrc/xrc.did
```

```bash

dfx deploy perp --argument "(record {
    perp_asset_details = record {
        asset_class = variant {Cryptocurrency};
        symbol = \"ETH\"
    };
    collateral_asset_details = record {
        asset_class = variant {Cryptocurrency};
        symbol = \"ICP\"
    };
   watcher_id = record { principal_id = principal \"cvwul-djb3r-e6krd-nbnfl-tuhox-n4omu-kejey-3lku7-ae3bx-icbu7-yae\"};
   vault_id = record {principal_id = principal \"cvwul-djb3r-e6krd-nbnfl-tuhox-n4omu-kejey-3lku7-ae3bx-icbu7-yae\"};
   collateral_decimal = 1;
   collateral_asset = record {principal_id = principal \"cvwul-djb3r-e6krd-nbnfl-tuhox-n4omu-kejey-3lku7-ae3bx-icbu7-yae\"} 
})"

```

SET STATE

```bash
 dfx canister call perp update_state_details  "( record {not_paused=true;current_tick = 200_00_000;
 max_leveragex10 = 100;
 min_collateral = 0;
 interest_rate = 0 ;
 base_token_multiple = 1})"

```

```bash
 dfx canister call perp open_position "(1000000000000000,opt 199_50_000,20,true,variant {Limit = record {buy = false;init_lower_bound = 0;init_removed_liquidity = 0 ;order_size = 0;ref_tick = 0}},0,0)"

```

```bash
    dfx canister call perp open_position "(100000,null,20,false,variant {Market},0,0)"

```

```bash
 dfx canister call perp close_position "(null)"
```

## Deploy XRC details

### DFX CANISTER ENVIRONMENT VARIABLES

DFX_VERSION='0.23.0' <br>
DFX_NETWORK='ic' <br>
CANISTER_ID_XRC='c3xu2-bqaaa-aaaak-qlsgq-cai' <br>
CANISTER_ID='c3xu2-bqaaa-aaaak-qlsgq-cai' <br>
CANISTER_CANDID_PATH='/home/adokiye/perp/src/xrc/xrc.did' <br>

### END DFX CANISTER ENVIRONMENT VARIABLES