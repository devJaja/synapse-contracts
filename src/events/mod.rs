use crate::types::Event;
use soroban_sdk::{symbol_short, Address, Env};

// TODO(#66): include `old_status` in StatusUpdated event payload for full audit trail

pub fn emit(env: &Env, caller: &Address, event: Event) {
    let ledger = env.ledger().sequence();
    env.events().publish((symbol_short!("synapse"),), (event, caller.clone(), ledger));
}
