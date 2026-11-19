use pull_core::terms::{Receive, Terms};
use pull_tests::*;
use solana_address::Address;

const AUTHORITY: &str = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";
const SPENDER: &str = "GNxM82DJMja5ux5extFCEjbQ5C88hvcG7fvsiSCQumgs";
const FROM: &str = "4dEfGh1uC6pK4CwNa5oZ2bJwmWv6kD6YQX7sKfM5tR2b";
const TO: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const MONTHLY: Option<u32> = Some(2_592_000);

fn key(s: &str) -> [u8; 32] {
    Address::from_str_const(s).to_bytes()
}

fn decoded(terms: &Terms) -> bool {
    Terms::decode(&encode(terms)).is_ok()
}

#[test]
fn terms_round_trip_and_invalid_terms_are_refused() {
    let (authority, spender) = (key(AUTHORITY), key(SPENDER));
    let (from, to) = (key(FROM), key(TO));
    let monthly = limit(&from, 10 * USDC, MONTHLY);
    let receive = Receive { to: &to, min: 3 };
    let valid = terms(&authority, Some(&spender), None, monthly, Some(receive));
    let bytes = encode(&valid);
    assert_eq!(Terms::decode(&bytes).unwrap(), valid);

    // Nothing bound: anyone may spend, to anywhere, and the owner receives nothing
    assert!(!decoded(&terms(&authority, None, None, monthly, None)));
    // Anyone may send it on, but only to the account the terms name
    let mut pinned = monthly;
    pinned.to = Some(&to);
    assert!(decoded(&terms(&authority, None, None, pinned, None)));
    assert!(decoded(&terms(
        &authority,
        None,
        None,
        monthly,
        Some(receive)
    )));
    // A limit of nothing
    let nothing = limit(&from, 0, None);
    assert!(!decoded(&terms(
        &authority,
        Some(&spender),
        None,
        nothing,
        None
    )));
    // A window of no length is not a window: on the wire it is a limit in total
    let no_window = terms(
        &authority,
        Some(&spender),
        None,
        limit(&from, USDC, Some(0)),
        None,
    );
    assert!(no_window.validate().is_err());
    let bytes = encode(&no_window);
    assert_eq!(Terms::decode(&bytes).unwrap().limit.period, None);
    // An expiry that is not after the start
    assert!(!decoded(&terms(
        &authority,
        Some(&spender),
        Some(NOW),
        monthly,
        None
    )));
}
