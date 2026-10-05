use pull_core::terms::{Decay, Per, Receive, Terms};
use pull_tests::*;
use solana_address::Address;

const AUTHORITY: &str = "7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU";
const SPENDER: &str = "GNxM82DJMja5ux5extFCEjbQ5C88hvcG7fvsiSCQumgs";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const FROM: &str = "4dEfGh1uC6pK4CwNa5oZ2bJwmWv6kD6YQX7sKfM5tR2b";
const TO: &str = "9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM";
const MONTHLY: Per = Per::Every(2_592_000);

fn key(s: &str) -> [u8; 32] {
    Address::from_str_const(s).to_bytes()
}

fn decimals(mint: &[u8; 32]) -> Result<u8, pull_core::errors::PullError> {
    Ok(if *mint == key(USDC_MINT) { 6 } else { 9 })
}

fn decoded(terms: &Terms) -> bool {
    Terms::decode(&encode(terms)).is_ok()
}

/// The canonical text is what wallets show and authorities sign; this pins it
/// byte for byte.
#[test]
fn subscription_renders_its_canonical_text() {
    let (authority, spender, usdc, from) =
        (key(AUTHORITY), key(SPENDER), key(USDC_MINT), key(FROM));
    let limits = limit(&from, &usdc, 8 * USDC, MONTHLY);
    let terms = terms(&authority, Some(&spender), None, limits, None);

    assert_eq!(
        text(&terms, decimals),
        "Solana Pull v1
cluster: localnet
engine: PULLrgDYqK1yFKVTSbWieX3ARP7U2XUyrjxWXqKgVzA
authority: 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
SPENDER: GNxM82DJMja5ux5extFCEjbQ5C88hvcG7fvsiSCQumgs
MAY TAKE: at most 8.000000 of mint EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v from 4dEfGh1uC6pK4CwNa5oZ2bJwmWv6kD6YQX7sKfM5tR2b every 30d
VALID: from 2026-09-21T14:13:20Z until revoked
SALT: 0"
    );
}

#[test]
fn order_renders_its_canonical_text() {
    let (authority, usdc, sol) = (key(AUTHORITY), key(USDC_MINT), key(SOL_MINT));
    let (from, to) = (key(FROM), key(TO));
    let limits = limit(&from, &usdc, 100 * USDC, Per::Total);
    let receive = Receive {
        to: &to,
        mint: &sol,
        min: 520_000_000,
        decay: Some(Decay {
            t0: NOW,
            t1: NOW + 300,
            min: 500_000_000,
        }),
    };
    let terms = terms(&authority, None, Some(NOW + 600), limits, Some(receive));

    assert_eq!(
        text(&terms, decimals),
        "Solana Pull v1
cluster: localnet
engine: PULLrgDYqK1yFKVTSbWieX3ARP7U2XUyrjxWXqKgVzA
authority: 7xKXtg2CW87d97TXJSDpbD5jBkheTqA83TZRuJosgAsU
SPENDER: anyone
MAY TAKE: at most 100.000000 of mint EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v from 4dEfGh1uC6pK4CwNa5oZ2bJwmWv6kD6YQX7sKfM5tR2b in total
MUST RECEIVE: at least 0.520000000 of mint So11111111111111111111111111111111111111112 in 9WzDXwBbmkg8ZTbNMqUxvQRAyrZzDsGYdLVL9zYtAWWM for each use, moving to 0.500000000 from 2026-09-21T14:13:20Z to 2026-09-21T14:18:20Z
VALID: from 2026-09-21T14:13:20Z until 2026-09-21T14:23:20Z
SALT: 0"
    );
}

#[test]
fn terms_round_trip_and_invalid_terms_are_refused() {
    let (authority, spender, usdc, sol) =
        (key(AUTHORITY), key(SPENDER), key(USDC_MINT), key(SOL_MINT));
    let (from, to) = (key(FROM), key(TO));
    let monthly = limit(&from, &usdc, 10 * USDC, MONTHLY);
    let receive = Receive {
        to: &to,
        mint: &sol,
        min: 3,
        decay: None,
    };
    let valid = terms(&authority, Some(&spender), None, monthly, Some(receive));
    let bytes = encode(&valid);
    assert_eq!(Terms::decode(&bytes).unwrap(), valid);

    // Nobody bound: anyone may spend, and the owner receives nothing
    assert!(!decoded(&terms(&authority, None, None, monthly, None)));
    assert!(decoded(&terms(
        &authority,
        None,
        None,
        monthly,
        Some(receive)
    )));
    // A limit of nothing, and a window of no length
    let nothing = limit(&from, &usdc, 0, Per::Total);
    assert!(!decoded(&terms(
        &authority,
        Some(&spender),
        None,
        nothing,
        None
    )));
    let no_window = limit(&from, &usdc, USDC, Per::Every(0));
    assert!(!decoded(&terms(
        &authority,
        Some(&spender),
        None,
        no_window,
        None
    )));
    // An expiry that is not after the start
    assert!(!decoded(&terms(
        &authority,
        Some(&spender),
        Some(NOW),
        monthly,
        None
    )));
}

/// What a wallet shows must pin the terms exactly: changing any byte of the
/// encoding either fails to decode or changes the text. One signature can
/// then never authorize two different policies.
#[test]
fn every_byte_of_the_terms_is_visible_in_the_text() {
    let (authority, spender, usdc, sol) =
        (key(AUTHORITY), key(SPENDER), key(USDC_MINT), key(SOL_MINT));
    let (from, to) = (key(FROM), key(TO));
    let receive = Receive {
        to: &to,
        mint: &sol,
        min: 520_000_000,
        decay: Some(Decay {
            t0: NOW,
            t1: NOW + 300,
            min: 500_000_000,
        }),
    };
    let total = limit(&from, &usdc, 100 * USDC, Per::Total);
    let mut order = terms(&authority, None, Some(NOW + 600), total, Some(receive));
    order.salt = 7;
    let monthly = limit(&from, &usdc, 10 * USDC, MONTHLY);
    let subscription = terms(&authority, Some(&spender), None, monthly, None);

    let render = |bytes: &[u8]| {
        let terms = Terms::decode(bytes).ok()?;
        let mut out = Vec::new();
        pull_core::render::render(&terms, |_| Ok(6), &mut out).ok()?;
        Some(out)
    };
    for bytes in [encode(&order), encode(&subscription)] {
        let original = render(&bytes).unwrap();
        for i in 0..bytes.len() {
            for v in 0..=u8::MAX {
                let mut mutated = bytes.clone();
                if mutated[i] == v {
                    continue;
                }
                mutated[i] = v;
                if let Some(text) = render(&mutated) {
                    assert_ne!(text, original, "byte {i} = {v} is invisible in the text");
                }
            }
        }
    }
}
