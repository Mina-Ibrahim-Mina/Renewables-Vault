use ic_cdk::{query, update};
use ic_stable_structures::memory_manager::{MemoryId, MemoryManager};
use ic_stable_structures::DefaultMemoryImpl;
use icrc_ledger_types::icrc1::account::Account;
use icrc_ledger_types::icrc1::transfer::{BlockIndex, Memo, TransferArg, TransferError};
use icrc_ledger_types::icrc3::transactions::{Mint, Transaction, Transfer};
use candid::Nat;
use std::cell::RefCell;

mod types;
use types::*;

// Constants for Renewable Vault Token (RVT)
const MAX_MEMO_SIZE: usize = 64;
const PERMITTED_DRIFT_NANOS: u64 = 60_000_000_000;
const TRANSACTION_WINDOW_NANOS: u64 = 24 * 60 * 60 * 1_000_000_000;
const TOKEN_NAME: &str = "RenewablesVaultToken";
const TOKEN_SYMBOL: &str = "RVT";
const DECIMALS: u8 = 8;
const INITIAL_SUPPLY: u64 = 1_000_000_000 * 100_000_000; // 1 billion tokens with 8 decimals
const TRANSFER_FEE: u64 = 10_000; // 0.0001 RVT

// Memory management
const CONFIGURATION_MEMORY_ID: MemoryId = MemoryId::new(1);
const TRANSACTION_LOG_MEMORY_ID: MemoryId = MemoryId::new(2);

thread_local! {
    static MEMORY_MANAGER: RefCell<MemoryManager<DefaultMemoryImpl>> =
        RefCell::new(MemoryManager::init(DefaultMemoryImpl::default()));

    static STATE: RefCell<State> = MEMORY_MANAGER.with(|cell| {
        let mm = cell.borrow();
        let configuration = ConfigCell::init(mm.get(CONFIGURATION_MEMORY_ID), Configuration {
            token_name: TOKEN_NAME.to_string(),
            token_symbol: TOKEN_SYMBOL.to_string(),
            token_logo: "https://renewablesvault.com/logo.png".to_string(),
            transfer_fee: Nat::from(TRANSFER_FEE),
            decimals: DECIMALS,
            minting_account: None,
            token_created: false,
        }).expect("Failed to initialize config cell");
        
        let transaction_log = TransactionLog::init(mm.get(TRANSACTION_LOG_MEMORY_ID))
            .expect("Failed to initialize transaction log");
            
        RefCell::new(State {
            configuration,
            transaction_log,
        })
    });
}

// ================== HELPER FUNCTIONS ================== //
fn read_state<R>(f: impl FnOnce(&State) -> R) -> R {
    STATE.with(|cell| f(&cell.borrow()))
}

fn mutate_state<R>(f: impl FnOnce(&mut State) -> R) -> R {
    STATE.with(|cell| f(&mut cell.borrow_mut()))
}

fn balance(account: Account) -> Nat {
    read_state(|state| {
        state.transaction_log.iter()
            .fold(Nat::from(0u64), |mut balance, tx| {
                match &tx.0 {
                    Transaction { mint: Some(mint), .. } if mint.to == account => 
                        balance += mint.amount.clone(),
                    Transaction { burn: Some(burn), .. } if burn.from == account => 
                        balance -= burn.amount.clone(),
                    Transaction { transfer: Some(transfer), .. } => {
                        if transfer.to == account {
                            balance += transfer.amount.clone();
                        }
                        if transfer.from == account {
                            balance -= transfer.amount.clone();
                            if let Some(fee) = transfer.fee.clone() {
                                balance -= fee;
                            }
                        }
                    }
                    Transaction { approve: Some(approve), .. } 
                        if approve.from == account => 
                    {
                        if let Some(fee) = approve.fee.clone() {
                            balance -= fee;
                        }
                    }
                    _ => {}
                }
                balance
            })
    })
}

fn record_tx(tx: &StorableTransaction) -> BlockIndex {
    mutate_state(|state| {
        let idx = state.transaction_log.len();
        state.transaction_log.push(tx).expect("Failed to record transaction");
        BlockIndex::from(idx as u64)
    })
}

// ================== TOKEN OPERATIONS ================== //
#[update]
fn initialize_token() -> Result<String, String> {
    if read_state(|s| s.configuration.get().token_created) {
        return Err("Token already created".to_string());
    }

    let minting_account = Account {
        owner: ic_cdk::api::caller(),
        subaccount: None,
    };

    // Create initial mint transaction
    let init_tx = StorableTransaction(Transaction {
        kind: "mint".to_string(),
        mint: Some(Mint {
            amount: Nat::from(INITIAL_SUPPLY),
            to: minting_account.clone(),
            memo: Some(Memo::from("Initial supply for Renewables Vault".as_bytes().to_vec())),
            created_at_time: Some(ic_cdk::api::time()),
        }),
        burn: None,
        transfer: None,
        approve: None,
        timestamp: ic_cdk::api::time(),
    });

    mutate_state(|state| {
        // Record initial mint
        record_tx(&init_tx);

        // Update configuration
        state.configuration.set(Configuration {
            token_name: TOKEN_NAME.to_string(),
            token_symbol: TOKEN_SYMBOL.to_string(),
            token_logo: "https://renewablesvault.com/logo.png".to_string(),
            transfer_fee: Nat::from(TRANSFER_FEE),
            decimals: DECIMALS,
            minting_account: Some(minting_account),
            token_created: true,
        }).map_err(|_| "Failed to set token configuration".to_string())?;

        Ok("RenewablesVaultToken (RVT) initialized successfully".to_string())
    })
}

#[update]
fn mint_tokens(amount: u64, recipient: Account) -> Result<BlockIndex, String> {
    let caller = ic_cdk::api::caller();
    let minting_account = read_state(|s| 
        s.configuration.get().minting_account.clone().ok_or("Minting account not set".to_string())
    )?;

    if caller != minting_account.owner {
        return Err("Only minting account can mint tokens".to_string());
    }

    let mint_tx = StorableTransaction(Transaction {
        kind: "mint".to_string(),
        mint: Some(Mint {
            amount: Nat::from(amount),
            to: recipient,
            memo: Some(Memo::from("Renewables Vault token mint".as_bytes().to_vec())),
            created_at_time: Some(ic_cdk::api::time()),
        }),
        burn: None,
        transfer: None,
        approve: None,
        timestamp: ic_cdk::api::time(),
    });

    let index = record_tx(&mint_tx);
    Ok(index)
}

// ================== RENEWABLES VAULT SPECIFIC FUNCTIONS ================== //
fn create_subaccount(project_id: u64) -> [u8; 32] {
    let mut subaccount = [0u8; 32];
    let id_bytes = project_id.to_be_bytes();
    subaccount[..id_bytes.len()].copy_from_slice(&id_bytes);
    subaccount
}

fn calculate_rewards(_account: Account, _project_id: u64) -> Nat {
    // Placeholder implementation - replace with actual reward calculation
    Nat::from(100_000_000u64) // 1 RVT as reward (8 decimals)
}

#[update]
fn stake_tokens(amount: u64, project_id: u64) -> Result<BlockIndex, String> {
    let caller_account = Account {
        owner: ic_cdk::api::caller(),
        subaccount: None,
    };

    // Verify balance
    if balance(caller_account.clone()) < Nat::from(amount) {
        return Err("Insufficient balance".to_string());
    }

    // Create stake transaction
    let stake_tx = StorableTransaction(Transaction {
        kind: "stake".to_string(),
        transfer: Some(Transfer {
            from: caller_account.clone(),
            to: Account {
                owner: ic_cdk::id(), // Canister-controlled staking account
                subaccount: Some(create_subaccount(project_id)),
            },
            amount: Nat::from(amount),
            spender: None,
            memo: Some(Memo::from(format!("Stake for project {}", project_id).as_bytes().to_vec())),
            fee: Some(Nat::from(TRANSFER_FEE)),
            created_at_time: Some(ic_cdk::api::time()),
        }),
        mint: None,
        burn: None,
        approve: None,
        timestamp: ic_cdk::api::time(),
    });

    let index = record_tx(&stake_tx);
    Ok(index)
}

#[update]
fn claim_rewards(project_id: u64) -> Result<BlockIndex, String> {
    let caller_account = Account {
        owner: ic_cdk::api::caller(),
        subaccount: None,
    };

    let rewards = calculate_rewards(caller_account.clone(), project_id);

    let reward_tx = StorableTransaction(Transaction {
        kind: "reward".to_string(),
        mint: Some(Mint {
            amount: rewards,
            to: caller_account,
            memo: Some(Memo::from(format!("Rewards for project {}", project_id).as_bytes().to_vec())),
            created_at_time: Some(ic_cdk::api::time()),
        }),
        burn: None,
        transfer: None,
        approve: None,
        timestamp: ic_cdk::api::time(),
    });

    let index = record_tx(&reward_tx);
    Ok(index)
}

// ================== INTERNET IDENTITY INTEGRATION ================== //
#[query]
fn get_principal() -> String {
    ic_cdk::api::caller().to_string()
}

#[update]
fn associate_energy_project(_project_id: u64, _amount: u64) -> Result<(), String> {
    // Placeholder for actual implementation
    Ok(())
}

// ================== ICRC STANDARD IMPLEMENTATION ================== //
#[update]
fn icrc1_transfer(arg: TransferArg) -> Result<BlockIndex, TransferError> {
    // Simplified implementation for demo purposes
    let from = Account {
        owner: ic_cdk::api::caller(),
        subaccount: arg.from_subaccount,
    };
    
    let transfer_tx = StorableTransaction(Transaction {
        kind: "transfer".to_string(),
        transfer: Some(Transfer {
            from: from.clone(),
            to: arg.to,
            amount: arg.amount.clone(),
            spender: None,
            memo: arg.memo.clone(),
            fee: arg.fee.clone(),
            created_at_time: arg.created_at_time,
        }),
        mint: None,
        burn: None,
        approve: None,
        timestamp: ic_cdk::api::time(),
    });

    // Verify balance
    let transfer_fee = arg.fee.unwrap_or_else(|| Nat::from(TRANSFER_FEE));
    let total_debit = arg.amount + transfer_fee;
    let current_balance = balance(from);
    
    if current_balance < total_debit {
        return Err(TransferError::InsufficientFunds { balance: current_balance });
    }

    let index = record_tx(&transfer_tx);
    Ok(index)
}

#[query]
fn icrc1_balance_of(account: Account) -> Nat {
    balance(account)
}

#[query]
fn icrc1_total_supply() -> Nat {
    read_state(|state| {
        state.transaction_log.iter().fold(Nat::from(0u64), |mut supply, tx| {
            match &tx.0 {
                Transaction { mint: Some(mint), .. } => supply += mint.amount.clone(),
                Transaction { burn: Some(burn), .. } => supply -= burn.amount.clone(),
                _ => {}
            }
            supply
        })
    })
}

#[query]
fn icrc1_minting_account() -> Option<Account> {
    read_state(|s| s.configuration.get().minting_account.clone())
}

#[query]
fn icrc1_name() -> String {
    read_state(|s| s.configuration.get().token_name.clone())
}

#[query]
fn icrc1_token_symbol() -> String {
    read_state(|s| s.configuration.get().token_symbol.clone())
}

#[query]
fn icrc1_decimals() -> u8 {
    read_state(|s| s.configuration.get().decimals)
}

#[query]
fn icrc1_fee() -> Nat {
    read_state(|s| s.configuration.get().transfer_fee.clone())
}

ic_cdk::export_candid!();