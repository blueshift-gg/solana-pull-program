use pull_core::terms::{In, Terms};
use pull_tests::*;
use solana_address::Address;

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
    let spender = key(SPENDER);
    let (from, to) = (key(FROM), key(TO));
    let monthly = out(&from, 10 * USDC, MONTHLY);
    let receive = In { to: &to, min: 3 };
    let valid = terms(Some(&spender), None, monthly, Some(receive));
    let bytes = encode(&valid);
    assert_eq!(Terms::decode(&bytes).unwrap(), valid);

    // Nothing bound: anyone may spend, to anywhere, and the owner receives nothing
    assert!(!decoded(&terms(None, None, monthly, None)));
    // Anyone may send it on, but only to the account the terms name
    let mut pinned = monthly;
    pinned.to = Some(&to);
    assert!(decoded(&terms(None, None, pinned, None)));
    assert!(decoded(&terms(None, None, monthly, Some(receive))));
    // A limit of nothing
    let nothing = out(&from, 0, None);
    assert!(!decoded(&terms(Some(&spender), None, nothing, None)));
    // A window of no length is not a window: on the wire it is a limit in total
    let no_window = terms(Some(&spender), None, out(&from, USDC, Some(0)), None);
    assert!(no_window.validate().is_err());
    let bytes = encode(&no_window);
    assert_eq!(Terms::decode(&bytes).unwrap().out.period, None);
    // An expiry that is not after the start
    assert!(!decoded(&terms(Some(&spender), Some(NOW), monthly, None)));
}
