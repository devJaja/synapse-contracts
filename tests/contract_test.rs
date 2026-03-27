#![cfg(test)]

use soroban_sdk::{
    symbol_short,
    testutils::{Address as _, Events as_},
    vec, Address, Env, IntoVal, String as SorobanString, TryFromVal, Val,
};
use synapse_contract::{
    types::{Event, TransactionStatus, MAX_RETRIES},
    SynapseContract,
    SynapseContractClient,
};

fn setup(env: &Env) -> (Address, Address, SynapseContractClient<'_>) {
    env.mock_all_auths();
    let id = env.register_contract(None, SynapseContract);
    let client = SynapseContractClient::new(env, &id);
    let admin = Address::generate(env);
    client.initialize(&admin);
    (admin, id, client)
}

fn event_data(env: &Env, raw: Val) -> (Event, u32) {
    <(Event, u32)>::try_from_val(env, &raw).unwrap()
}

fn usd(env: &Env) -> SorobanString {
    SorobanString::from_str(env, "USD")
}

fn reg(
    client: &SynapseContractClient,
    relayer: &Address,
    anchor: &str,
    stellar: &Address,
    amount: i128,
    env: &Env,
) -> SorobanString {
    client.register_deposit(relayer, &SorobanString::from_str(env, anchor), stellar, &amount, &usd(env), &None, &None)
}


#[test]
#[should_panic(expected = "already initialised")]
fn initialize_twice_panics() {
    let env = Env::default();
    let (admin, _, client) = setup(&env);
    client.initialize(&admin);
}
