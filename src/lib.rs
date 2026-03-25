#![no_std]

mod access;
mod events;
mod storage;
mod types;

use access::{require_admin, require_relayer};
use events::emit;
use soroban_sdk::{contract, contractimpl, Address, Env, String as SorobanString, Vec};
use storage::{assets, deposits, dlq, max_deposit, relayers, settlements};
use types::{DlqEntry, Event, Settlement, Transaction, TransactionStatus};

#[contract]
pub struct SynapseContract;

#[contractimpl]
impl SynapseContract {
    // TODO(#1): prevent re-initialisation — panic if admin already set
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        storage::admin::set(&env, &admin);
        emit(&env, Event::Initialized(admin));
    }

    // TODO(#3): emit `RelayerGranted` event
    pub fn grant_relayer(env: Env, caller: Address, relayer: Address) {
        // Reject the all-zeros Stellar account (GAAAAAA...AWHF) as an invalid address.
        // This is the canonical "zero address" on Stellar — 32 zero bytes encoded as a G-address.
        let zero_addr = Address::from_string(&SorobanString::from_str(
            &env,
            "GAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAWHF",
        ));
        if relayer == zero_addr {
            panic!("invalid relayer address")
        }
        require_admin(&env, &caller);
        relayers::add(&env, &relayer);
    }

    // TODO(#5): emit `RelayerRevoked` event
    // TODO(#6): panic if revoking a non-existent relayer
    pub fn revoke_relayer(env: Env, caller: Address, relayer: Address) {
        require_admin(&env, &caller);
        relayers::remove(&env, &relayer);
    }

    // TODO(#7): emit `AdminTransferred` event
    // TODO(#8): two-step admin transfer (propose + accept) to prevent lockout
    pub fn transfer_admin(env: Env, caller: Address, new_admin: Address) {
        require_admin(&env, &caller);
        storage::admin::set(&env, &new_admin);
    }

    // TODO(#9): emit `ContractPaused` event
    // TODO(#10): block all state-mutating calls when paused
    pub fn pause(env: Env, caller: Address) {
        require_admin(&env, &caller);
        storage::pause::set(&env, true);
    }

    // TODO(#11): emit `ContractUnpaused` event
    pub fn unpause(env: Env, caller: Address) {
        require_admin(&env, &caller);
        storage::pause::set(&env, false);
    }

    // TODO(#12): validate asset_code is non-empty and uppercase-alphanumeric only
    // TODO(#13): cap the total number of allowed assets to bound instance storage
    pub fn add_asset(env: Env, caller: Address, asset_code: SorobanString) {
        require_admin(&env, &caller);
        assets::add(&env, &asset_code);
        emit(&env, Event::AssetAdded(asset_code));
    }

    // TODO(#14): panic if asset_code is not currently in the allowlist
    pub fn remove_asset(env: Env, caller: Address, asset_code: SorobanString) {
        require_admin(&env, &caller);
        assets::remove(&env, &asset_code);
        emit(&env, Event::AssetRemoved(asset_code));
    }

    // TODO(#15): enforce minimum deposit amount (configurable by admin)
    // TODO(#16): enforce maximum deposit amount (configurable by admin)
    // TODO(#18): add `memo` field support (mirrors synapse-core CallbackPayload)
    // TODO(#19): add `memo_type` field support (text | hash | id)
    // TODO(#21): bump persistent TTL on AnchorIdx entry after save
    pub fn register_deposit(
        env: Env,
        caller: Address,
        anchor_transaction_id: SorobanString,
        stellar_account: Address,
        amount: i128,
        asset_code: SorobanString,
        callback_type: Option<SorobanString>,
    ) -> SorobanString {
        require_relayer(&env, &caller);
        if anchor_transaction_id.len() == 0 { panic!("anchor_transaction_id must not be empty"); }
        if let Some(ref ct) = callback_type {
            let deposit = SorobanString::from_str(&env, "deposit");
            let withdrawal = SorobanString::from_str(&env, "withdrawal");
            if *ct != deposit && *ct != withdrawal {
                panic!("callback_type must be 'deposit' or 'withdrawal'");
            }
        }
        assets::require_allowed(&env, &asset_code);

        if let Some(existing) = deposits::find_by_anchor_id(&env, &anchor_transaction_id) {
            return existing;
        }

        let tx = Transaction::new(&env, anchor_transaction_id.clone(), stellar_account, amount, asset_code, callback_type);
        let id = tx.id.clone();
        deposits::save(&env, &tx);
        deposits::index_anchor_id(&env, &anchor_transaction_id, &id);
        emit(&env, Event::DepositRegistered(id.clone(), anchor_transaction_id));
        id
    }

    // TODO(#23): enforce transition guard — must be Pending
    pub fn mark_processing(env: Env, caller: Address, tx_id: SorobanString) {
        require_relayer(&env, &caller);
        let mut tx = deposits::get(&env, &tx_id);
        tx.status = TransactionStatus::Processing;
        tx.updated_ledger = env.ledger().sequence();
        deposits::save(&env, &tx);
        emit(&env, Event::StatusUpdated(tx_id, TransactionStatus::Processing));
    }

    // TODO(#25): enforce transition guard — must be Processing
    pub fn mark_completed(env: Env, caller: Address, tx_id: SorobanString) {
        require_relayer(&env, &caller);
        let mut tx = deposits::get(&env, &tx_id);
        tx.status = TransactionStatus::Completed;
        tx.updated_ledger = env.ledger().sequence();
        deposits::save(&env, &tx);
        emit(&env, Event::StatusUpdated(tx_id, TransactionStatus::Completed));
    }

    // TODO(#26): enforce transition guard — must be Pending or Processing
    // TODO(#27): cap max retry_count; emit `MaxRetriesExceeded` when hit
    // TODO(#28): validate error_reason is non-empty
    pub fn mark_failed(env: Env, caller: Address, tx_id: SorobanString, error_reason: SorobanString) {
        require_relayer(&env, &caller);
        let mut tx = deposits::get(&env, &tx_id);
        tx.status = TransactionStatus::Failed;
        tx.updated_ledger = env.ledger().sequence();
        deposits::save(&env, &tx);
        let entry = DlqEntry::new(&env, tx_id.clone(), error_reason.clone());
        dlq::push(&env, &entry);
        emit(&env, Event::MovedToDlq(tx_id, error_reason));
    }

    // TODO(#29): implement — reset tx status to Pending, increment retry_count
    // TODO(#30): remove DLQ entry after successful retry
    // TODO(#31): emit `DlqRetried` event
    // TODO(#32): only admin OR original relayer should be able to retry
    pub fn retry_dlq(env: Env, caller: Address, tx_id: SorobanString) {
        require_admin(&env, &caller);
        let entry = dlq::get(&env, &tx_id).expect("DLQ entry not found");
        let mut new_entry = entry.clone();
        new_entry.retry_count += 1;
        new_entry.last_retry_ledger = env.ledger().sequence();
        if new_entry.retry_count > types::MAX_RETRIES {
            emit(&env, Event::MaxRetriesExceeded(tx_id.clone()));
            panic!("MaxRetriesExceeded");
        }
        let mut tx = deposits::get(&env, &tx_id);
        tx.status = TransactionStatus::Pending;
        tx.updated_ledger = env.ledger().sequence();
        deposits::save(&env, &tx);
        dlq::remove(&env, &tx_id);
        emit(&env, Event::DlqRetried(tx_id));
    }

    // TODO(#33): verify each tx_id exists and has status Completed
    // TODO(#34): verify no tx_id is already linked to a settlement
    // TODO(#35): write settlement_id back onto each Transaction
    // TODO(#36): verify total_amount matches sum of tx amounts on-chain
    // TODO(#37): verify period_start <= period_end
    // TODO(#38): bump Settlement TTL after save
    // TODO(#39): emit per-tx `Settled` event in addition to batch event
    pub fn finalize_settlement(
        env: Env,
        caller: Address,
        asset_code: SorobanString,
        tx_ids: Vec<SorobanString>,
        total_amount: i128,
        period_start: u64,
        period_end: u64,
    ) -> SorobanString {
        require_relayer(&env, &caller);
        if period_start > period_end {
            panic!("period_start must be <= period_end")
        }
        let s = Settlement::new(&env, asset_code.clone(), tx_ids, total_amount, period_start, period_end);
        let id = s.id.clone();
        settlements::save(&env, &s);
        emit(&env, Event::SettlementFinalized(id.clone(), asset_code, total_amount));
        id
    }

    // TODO(#40): add `get_dlq_entry(tx_id)` query
    // TODO(#41): add `get_admin()` query
    // TODO(#43): add `get_min_deposit()` query
    // TODO(#44): add `get_max_deposit()` query

    pub fn is_paused(env: Env) -> bool {
        storage::pause::is_paused(&env)
    }

    pub fn get_transaction(env: Env, tx_id: SorobanString) -> Transaction {
        deposits::get(&env, &tx_id)
    }

    pub fn get_settlement(env: Env, settlement_id: SorobanString) -> Settlement {
        settlements::get(&env, &settlement_id)
    }

    pub fn is_asset_allowed(env: Env, asset_code: SorobanString) -> bool {
        assets::is_allowed(&env, &asset_code)
    }

    pub fn is_relayer(env: Env, address: Address) -> bool {
        relayers::has(&env, &address)
    }

    pub fn set_max_deposit(env: Env, caller: Address, amount: i128) {
        require_admin(&env, &caller);
        max_deposit::set(&env, &amount);
    }

    pub fn get_max_deposit(env: Env) -> i128 {
        max_deposit::get(&env)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::{Address as _, Events as _}, vec, Env, IntoVal, String as SorobanString, symbol_short};

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SynapseContract);
        let client = SynapseContractClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.initialize(&admin);
        (admin, contract_id)
    }

    #[test]
    fn test_initialize_emits_initialized_event() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SynapseContract);
        let client = SynapseContractClient::new(&env, &contract_id);
        let admin = Address::generate(&env);
        client.initialize(&admin);
        let events = env.events().all();
        assert_eq!(events.len(), 1);
        let (emitting_contract, topics, _data) = events.get(0).unwrap();
        assert_eq!(emitting_contract, contract_id);
        assert_eq!(topics, (symbol_short!("synapse"),).into_val(&env));
    }

    #[test]
    fn test_is_paused() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        
        // Initially should not be paused
        assert!(!client.is_paused());
        
        // Pause the contract
        client.pause(&admin);
        assert!(client.is_paused());
        
        // Unpause the contract
        client.unpause(&admin);
        assert!(!client.is_paused());
    }

    #[test]
    #[should_panic(expected = "period_start must be <= period_end")]
    fn test_finalize_settlement_panics_when_period_start_exceeds_period_end() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        let relayer = Address::generate(&env);

        client.grant_relayer(&admin, &relayer);
        client.finalize_settlement(
            &relayer,
            &SorobanString::from_str(&env, "USD"),
            &vec![&env],
            &0i128,
            &2u64,
            &1u64,
        );
    }

    #[test]
    fn test_max_deposit() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);

        // Default should be 0
        assert_eq!(client.get_max_deposit(), 0i128);

        // Set to 1000
        client.set_max_deposit(&admin, &1000i128);
        assert_eq!(client.get_max_deposit(), 1000i128);

        // Set to 5000
        client.set_max_deposit(&admin, &5000i128);
        assert_eq!(client.get_max_deposit(), 5000i128);
    }


    #[test]
    #[should_panic(expected = "anchor_transaction_id must not be empty")]
    fn test_register_deposit_panics_on_empty_anchor_id() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        let relayer = Address::generate(&env);
        let stellar = Address::generate(&env);
        let asset = SorobanString::from_str(&env, "USD");
        client.grant_relayer(&admin, &relayer);
        client.add_asset(&admin, &asset);
        client.register_deposit(&relayer, &SorobanString::from_str(&env, ""), &stellar, &1i128, &asset, &None);
    }

    #[test]
    fn test_register_deposit_with_callback_type() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        let relayer = Address::generate(&env);
        let stellar = Address::generate(&env);
        let asset = SorobanString::from_str(&env, "USD");
        client.grant_relayer(&admin, &relayer);
        client.add_asset(&admin, &asset);

        let tx_id = client.register_deposit(
            &relayer,
            &SorobanString::from_str(&env, "anchor-1"),
            &stellar,
            &100i128,
            &asset,
            &Some(SorobanString::from_str(&env, "deposit")),
        );
        let tx = client.get_transaction(&tx_id);
        assert_eq!(tx.callback_type, Some(SorobanString::from_str(&env, "deposit")));

        let tx_id2 = client.register_deposit(
            &relayer,
            &SorobanString::from_str(&env, "anchor-2"),
            &stellar,
            &100i128,
            &asset,
            &Some(SorobanString::from_str(&env, "withdrawal")),
        );
        let tx2 = client.get_transaction(&tx_id2);
        assert_eq!(tx2.callback_type, Some(SorobanString::from_str(&env, "withdrawal")));

        let tx_id3 = client.register_deposit(
            &relayer,
            &SorobanString::from_str(&env, "anchor-3"),
            &stellar,
            &100i128,
            &asset,
            &None,
        );
        let tx3 = client.get_transaction(&tx_id3);
        assert_eq!(tx3.callback_type, None);
    }

    #[test]
    #[should_panic(expected = "callback_type must be 'deposit' or 'withdrawal'")]
    fn test_register_deposit_panics_on_invalid_callback_type() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        let relayer = Address::generate(&env);
        let stellar = Address::generate(&env);
        let asset = SorobanString::from_str(&env, "USD");
        client.grant_relayer(&admin, &relayer);
        client.add_asset(&admin, &asset);
        client.register_deposit(
            &relayer,
            &SorobanString::from_str(&env, "anchor-1"),
            &stellar,
            &100i128,
            &asset,
            &Some(SorobanString::from_str(&env, "transfer")),
        );
    }
}
