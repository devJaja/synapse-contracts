#![no_std]

mod access;
mod events;
mod storage;
mod types;

use access::{require_admin, require_relayer};
use events::emit;
use soroban_sdk::{contract, contractimpl, Address, Env, String as SorobanString, Vec};
use storage::{admin, assets, deposits, dlq, pending_admin, relayers, settlements};
use types::{DlqEntry, Event, Settlement, Transaction, TransactionStatus};

#[contract]
pub struct SynapseContract;

#[contractimpl]
impl SynapseContract {
    // TODO(#1): prevent re-initialisation — panic if admin already set
    // TODO(#2): emit `Initialized` event on first call
    pub fn initialize(env: Env, admin: Address) {
        admin.require_auth();
        storage::admin::set(&env, &admin);
    }

    // TODO(#3): emit `RelayerGranted` event
    // TODO(#4): prevent granting relayer to the zero/invalid address
    pub fn grant_relayer(env: Env, caller: Address, relayer: Address) {
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

    /// Propose a new admin address for transfer.
    /// Only the current admin can propose a new admin.
    /// The proposed admin must accept the transfer to complete it.
    pub fn propose_admin(env: Env, caller: Address, new_admin: Address) {
        require_admin(&env, &caller);
        pending_admin::set(&env, &new_admin);
        let current_admin = admin::get(&env);
        emit(&env, Event::AdminTransferProposed(current_admin, new_admin));
    }

    /// Accept the admin transfer proposal.
    /// Only the proposed admin can accept the transfer.
    /// After acceptance, the proposed admin becomes the new admin.
    pub fn accept_admin(env: Env, caller: Address) {
        caller.require_auth();
        let pending = pending_admin::get(&env).expect("no pending admin transfer");
        if caller != pending {
            panic!("only proposed admin can accept");
        }
        let old_admin = admin::get(&env);
        admin::set(&env, &pending);
        pending_admin::clear(&env);
        emit(&env, Event::AdminTransferred(old_admin, pending));
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
    // TODO(#17): validate anchor_transaction_id is non-empty
    // TODO(#18): add `memo` field support (mirrors synapse-core CallbackPayload)
    // TODO(#19): add `memo_type` field support (text | hash | id)
    // TODO(#20): add `callback_type` field (deposit | withdrawal)
    // TODO(#21): bump persistent TTL on AnchorIdx entry after save
    // TODO(#22): bump persistent TTL on Tx entry after save
    pub fn register_deposit(
        env: Env,
        caller: Address,
        anchor_transaction_id: SorobanString,
        stellar_account: Address,
        amount: i128,
        asset_code: SorobanString,
    ) -> SorobanString {
        require_relayer(&env, &caller);
        assets::require_allowed(&env, &asset_code);

        if let Some(existing) = deposits::find_by_anchor_id(&env, &anchor_transaction_id) {
            return existing;
        }

        let tx = Transaction::new(&env, anchor_transaction_id.clone(), stellar_account, amount, asset_code);
        let id = tx.id.clone();
        deposits::save(&env, &tx);
        deposits::index_anchor_id(&env, &anchor_transaction_id, &id);
        emit(&env, Event::DepositRegistered(id.clone(), anchor_transaction_id));
        id
    }

    // TODO(#23): enforce transition guard — must be Pending
    // TODO(#24): bump Tx TTL on every status update
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

    // TODO(#58): enforce transition guard — must be Pending or Processing
    // TODO(#59): consider whether cancelled transactions should go to DLQ
    pub fn cancel_transaction(env: Env, caller: Address, tx_id: SorobanString) {
        require_admin(&env, &caller);
        let mut tx = deposits::get(&env, &tx_id);
        tx.status = TransactionStatus::Cancelled;
        tx.updated_ledger = env.ledger().sequence();
        deposits::save(&env, &tx);
        emit(&env, Event::StatusUpdated(tx_id, TransactionStatus::Cancelled));
    }

    // TODO(#29): implement — reset tx status to Pending, increment retry_count
    // TODO(#30): remove DLQ entry after successful retry
    // TODO(#31): emit `DlqRetried` event
    // TODO(#32): only admin OR original relayer should be able to retry
    pub fn retry_dlq(env: Env, caller: Address, tx_id: SorobanString) {
        require_admin(&env, &caller);
        let _ = (env, tx_id);
        panic!("not implemented")
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

    /// Get the current admin address
    pub fn get_admin(env: Env) -> Address {
        admin::get(&env)
    }

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
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{testutils::Address as _, Env};

    fn setup(env: &Env) -> (Address, Address) {
        env.mock_all_auths();
        let contract_id = env.register_contract(None, SynapseContract);
        let client = SynapseContractClient::new(env, &contract_id);
        let admin = Address::generate(env);
        client.initialize(&admin);
        (admin, contract_id)
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
    fn test_propose_and_accept_admin() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        
        // Create a new admin address
        let new_admin = Address::generate(&env);
        
        // Current admin proposes new admin
        client.propose_admin(&admin, &new_admin);
        
        // New admin accepts the transfer
        client.accept_admin(&new_admin);
        
        // Verify new admin is now the admin
        // Note: We don't have a get_admin() function yet, but we can test
        // by trying to perform admin operations
        
        // Old admin should no longer be able to perform admin operations
        // This would panic if we tried, but we can't easily test that here
        // without catching panics
        
        // New admin should be able to perform admin operations
        // Let's test by having new admin pause the contract
        client.pause(&new_admin);
        assert!(client.is_paused());
    }

    #[test]
    #[should_panic(expected = "only proposed admin can accept")]
    fn test_accept_admin_wrong_caller() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        
        let new_admin = Address::generate(&env);
        let wrong_caller = Address::generate(&env);
        
        // Current admin proposes new admin
        client.propose_admin(&admin, &new_admin);
        
        // Wrong caller tries to accept (should panic)
        client.accept_admin(&wrong_caller);
    }

    #[test]
    #[should_panic(expected = "no pending admin transfer")]
    fn test_accept_admin_no_pending() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        
        let random_address = Address::generate(&env);
        
        // Try to accept without a pending admin (should panic)
        client.accept_admin(&random_address);
    }

    #[test]
    #[should_panic(expected = "not admin")]
    fn test_propose_admin_not_admin() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        
        let non_admin = Address::generate(&env);
        let new_admin = Address::generate(&env);
        
        // Non-admin tries to propose new admin (should panic)
        client.propose_admin(&non_admin, &new_admin);
    }

    #[test]
    fn test_admin_transfer_can_be_cancelled_by_new_proposal() {
        let env = Env::default();
        let (admin, contract_id) = setup(&env);
        let client = SynapseContractClient::new(&env, &contract_id);
        
        let first_proposed = Address::generate(&env);
        let second_proposed = Address::generate(&env);
        
        // Admin proposes first admin
        client.propose_admin(&admin, &first_proposed);
        
        // Before first proposed accepts, admin proposes a different admin
        client.propose_admin(&admin, &second_proposed);
        
        // First proposed should not be able to accept anymore
        // (This would panic with "no pending admin transfer" or similar)
        // Actually, the pending admin is now second_proposed, so first_proposed
        // would panic with "only proposed admin can accept"
        
        // Second proposed should be able to accept
        client.accept_admin(&second_proposed);
        
        // Verify second proposed is now admin by having them pause
        client.pause(&second_proposed);
        assert!(client.is_paused());
    }
}
