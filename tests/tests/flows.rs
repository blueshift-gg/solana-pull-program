#![allow(clippy::result_large_err)] // LiteSVM's own result type

use pull_core::constants::*;
use pull_core::errors::PullError;
use pull_core::terms::{Decay, Per, Receive};
use pull_tests::*;
use solana_address::Address;
use solana_signer::Signer;

const DAY: i64 = 86_400;
const MONTHLY: Per = Per::Every(30 * DAY as u32);

/// Whether `result` failed with exactly this program error.
fn refused(result: litesvm::types::TransactionResult, error: PullError) -> bool {
    let wanted = format!("Custom({})", error as u32);
    result.is_err_and(|e| format!("{:?}", e.err).contains(&wanted))
}

#[test]
fn subscription_is_one_instruction_and_resets_each_period() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (merchant, stranger) = (f.wallet(0, 0), f.wallet(0, 0));
    let (u, m, usdc) = (
        user.address().to_bytes(),
        merchant.address().to_bytes(),
        f.usdc.to_bytes(),
    );
    let user_usdc = user.usdc.to_bytes();

    // The merchant may take at most 10 USDC every 30 days, until revoked
    let limits = limit(&user_usdc, &usdc, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    let create = create(&user.address(), &user.address(), &bytes);
    f.send(&[create], &[&user.key]).unwrap();

    let pull = |f: &mut Fixture, day: i64, who: &Wallet, amount: u64| {
        f.set_time(NOW + day * DAY);
        let accounts = (user.usdc, f.usdc, who.usdc);
        let ix = pull(
            &who.address(),
            &user.address(),
            &bytes,
            accounts,
            amount,
            None,
            TOKEN,
        );
        f.send(&[ix], &[&who.key])
    };
    assert!(refused(
        pull(&mut f, 0, &stranger, USDC),
        PullError::InvalidSpender
    ));
    pull(&mut f, 0, &merchant, 10 * USDC).unwrap();
    // Nothing more this period; a new period starts full, and unused amounts do not carry over
    assert!(refused(
        pull(&mut f, 29, &merchant, 1),
        PullError::LimitExceeded
    ));
    pull(&mut f, 30, &merchant, 4 * USDC).unwrap();
    pull(&mut f, 75, &merchant, 10 * USDC).unwrap();
    assert!(refused(
        pull(&mut f, 89, &merchant, 1),
        PullError::LimitExceeded
    ));

    assert_eq!(f.balance(&user.usdc), 76 * USDC);
    assert_eq!(f.balance(&merchant.usdc), 24 * USDC);
}

#[test]
fn a_program_collects_a_subscription_by_cpi() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let merchant = f.wallet(0, 0);
    let (u, m, usdc) = (
        user.address().to_bytes(),
        merchant.address().to_bytes(),
        f.usdc.to_bytes(),
    );
    let user_usdc = user.usdc.to_bytes();

    let limits = limit(&user_usdc, &usdc, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    let create = create(&user.address(), &user.address(), &bytes);
    f.send(&[create], &[&user.key]).unwrap();

    // The merchant's own program makes the pull, as it would while renewing a membership
    let accounts = (user.usdc, f.usdc, merchant.usdc);
    let m = merchant.address();
    let ix = by_cpi(pull(
        &m,
        &user.address(),
        &bytes,
        accounts,
        10 * USDC,
        None,
        TOKEN,
    ));
    f.send(std::slice::from_ref(&ix), &[&merchant.key]).unwrap();
    assert!(refused(
        f.send(&[ix], &[&merchant.key]),
        PullError::LimitExceeded
    ));
    assert_eq!(f.balance(&merchant.usdc), 10 * USDC);
}

#[test]
fn a_signed_intent_runs_once_and_costs_its_signer_nothing() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (payee, stranger) = (f.wallet(0, 0), f.wallet(0, 0));
    let (u, p, usdc) = (
        user.address().to_bytes(),
        payee.address().to_bytes(),
        f.usdc.to_bytes(),
    );
    let user_usdc = user.usdc.to_bytes();
    let limits = limit(&user_usdc, &usdc, 10 * USDC, Per::Total);
    let (owner, spender) = (user.address(), payee.address());
    let take = (user.usdc, f.usdc, payee.usdc);

    // Signed off chain: the payee may take up to 10 USDC, once, with no
    // expiry. This is what a durable nonce is used for, with nothing set up first
    let mut intent = terms(&u, Some(&p), None, limits, None);
    let signature = sign(&intent, &user.key, f.decimals());
    let before = f.svm.get_balance(&owner).unwrap();

    // Nobody else can run it, and not for more than it says
    let elsewhere = (user.usdc, f.usdc, stranger.usdc);
    let theft = fill(
        &stranger.address(),
        &intent,
        &signature,
        elsewhere,
        USDC,
        None,
    );
    assert!(refused(
        f.send(&[theft], &[&stranger.key]),
        PullError::InvalidSpender
    ));
    let ix = fill(&spender, &intent, &signature, take, 10 * USDC + 1, None);
    assert!(refused(
        f.send(&[ix], &[&payee.key]),
        PullError::LimitExceeded
    ));

    // A year later the payee lands it, taking 6. That was its one use, for good
    f.set_time(NOW + 365 * DAY);
    let ix = fill(&spender, &intent, &signature, take, 6 * USDC, None);
    f.send(std::slice::from_ref(&ix), &[&payee.key]).unwrap();
    assert!(refused(f.send(&[ix], &[&payee.key]), PullError::NonceUsed));
    assert_eq!(f.balance(&payee.usdc), 6 * USDC);
    assert_eq!(f.svm.get_balance(&owner).unwrap(), before);

    // Every salt has its own bit: salt 1024 is bit 0 of the next page, and runs
    intent.salt = NONCE_BITS as u64;
    let signature = sign(&intent, &user.key, f.decimals());
    let ix = fill(&spender, &intent, &signature, take, USDC, None);
    f.send(&[ix], &[&payee.key]).unwrap();

    // The owner cancels one intent before anyone runs it
    intent.salt = 1;
    let signature = sign(&intent, &user.key, f.decimals());
    f.send(&[cancel(&owner, 1, false)], &[&user.key]).unwrap();
    let ix = fill(&spender, &intent, &signature, take, USDC, None);
    assert!(refused(f.send(&[ix], &[&payee.key]), PullError::NonceUsed));

    // Or a whole page at once: every intent signed with a salt in 2048..3072
    f.send(&[cancel(&owner, 2048, true)], &[&user.key]).unwrap();
    for salt in [2048, 2500, 3071] {
        intent.salt = salt;
        let signature = sign(&intent, &user.key, f.decimals());
        let ix = fill(&spender, &intent, &signature, take, USDC, None);
        assert!(refused(f.send(&[ix], &[&payee.key]), PullError::NonceUsed));
    }
}

#[test]
fn closing_a_policy_returns_the_rent_at_once() {
    let mut f = Fixture::new();
    let user = f.wallet(10 * USDC, 0);
    let (merchant, sponsor, stranger) = (f.wallet(0, 0), f.payer(), f.payer());
    let (u, m, usdc) = (
        user.address().to_bytes(),
        merchant.address().to_bytes(),
        f.usdc.to_bytes(),
    );
    let user_usdc = user.usdc.to_bytes();
    let (owner, payer) = (user.address(), sponsor.pubkey());

    // The user signs the transaction; a sponsor pays the fee and the rent.
    // One policy never expires, the other does: both close the same way
    let limits = limit(&user_usdc, &usdc, USDC, MONTHLY);
    let mut policies = Vec::new();
    for not_after in [None, Some(NOW + 365 * DAY)] {
        let bytes = encode(&terms(&u, Some(&m), not_after, limits, None));
        let create = create(&owner, &payer, &bytes);
        f.send(&[create], &[&sponsor, &user.key]).unwrap();
        policies.push(policy_pda(&owner, &bytes));
    }
    let funded = f.svm.get_balance(&payer).unwrap();

    // A stranger cannot close it, and the rent cannot go to anyone but its payer.
    // The owner can close, and so can the spender
    let ix = close(&stranger.pubkey(), &policies[0], &payer);
    assert!(refused(f.send(&[ix], &[&stranger]), PullError::NotClosable));
    let ix = close(&owner, &policies[0], &owner);
    assert!(refused(
        f.send(&[ix], &[&user.key]),
        PullError::InvalidPayer
    ));
    let ix = close(&owner, &policies[0], &payer);
    f.send(&[ix], &[&user.key]).unwrap();
    let ix = close(&merchant.address(), &policies[1], &payer);
    f.send(&[ix], &[&merchant.key]).unwrap();

    for policy in &policies {
        assert!(f.svm.get_account(policy).is_none_or(|a| a.lamports == 0));
    }
    assert!(f.svm.get_balance(&payer).unwrap() > funded);
}

#[test]
fn anyone_fills_a_signed_order_for_what_the_owner_must_receive() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (first, second) = (f.wallet(0, 10 * SOL), f.wallet(0, 10 * SOL));
    let (u, usdc, sol) = (
        user.address().to_bytes(),
        f.usdc.to_bytes(),
        f.sol.to_bytes(),
    );
    let (user_usdc, user_sol) = (user.usdc.to_bytes(), user.sol.to_bytes());

    // Out: at most 100 USDC, to anyone. In: at least 0.52 SOL, falling to
    // 0.50 over five minutes
    let limits = limit(&user_usdc, &usdc, 100 * USDC, Per::Total);
    let receive = Receive {
        to: &user_sol,
        mint: &sol,
        min: 520_000_000,
        decay: Some(Decay {
            t0: NOW,
            t1: NOW + 300,
            min: 500_000_000,
        }),
    };
    let order = terms(&u, None, Some(NOW + 600), limits, Some(receive));
    let signature = sign(&order, &user.key, f.decimals());

    // Halfway down, a solver fills it for 0.51 SOL in one instruction: the
    // signature, both transfers and the check of what arrived
    f.set_time(NOW + 150);
    let fill_by = |f: &Fixture, solver: &Wallet, amount: u64| {
        let take = (user.usdc, f.usdc, solver.usdc);
        let pay = Some((solver.sol, f.sol, user.sol));
        fill(&solver.address(), &order, &signature, take, amount, pay)
    };
    let ix = fill_by(&f, &first, 100 * USDC);
    f.send(&[ix], &[&first.key]).unwrap();
    assert_eq!(f.balance(&user.usdc), 0);
    assert_eq!(f.balance(&user.sol), 510_000_000);
    assert_eq!(f.balance(&first.usdc), 100 * USDC);

    // It was one use: a second solver gets nothing
    let ix = fill_by(&f, &second, USDC);
    assert!(refused(f.send(&[ix], &[&second.key]), PullError::NonceUsed));
}

#[test]
fn dca_runs_once_a_day_for_anyone_who_delivers() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (first, second) = (f.wallet(0, 10 * SOL), f.wallet(0, 10 * SOL));
    let (u, usdc, sol) = (
        user.address().to_bytes(),
        f.usdc.to_bytes(),
        f.sol.to_bytes(),
    );
    let (user_usdc, user_sol) = (user.usdc.to_bytes(), user.sol.to_bytes());
    let owner = user.address();

    // Out: at most 10 USDC every day, to anyone. In: at least 0.05 SOL each time
    let limits = limit(&user_usdc, &usdc, 10 * USDC, Per::Every(DAY as u32));
    let receive = Receive {
        to: &user_sol,
        mint: &sol,
        min: 50_000_000,
        decay: None,
    };
    let bytes = encode(&terms(&u, None, None, limits, Some(receive)));
    f.send(&[create(&owner, &owner, &bytes)], &[&user.key])
        .unwrap();

    let run = |f: &mut Fixture, keeper: &Wallet, amount: u64| {
        let take = (user.usdc, f.usdc, keeper.usdc);
        let pay = Some((keeper.sol, f.sol, user.sol));
        let ix = pull(&keeper.address(), &owner, &bytes, take, amount, pay, TOKEN);
        f.send(&[ix], &[&keeper.key])
    };
    run(&mut f, &first, 10 * USDC).unwrap();
    assert!(refused(run(&mut f, &second, 1), PullError::LimitExceeded));
    f.set_time(NOW + DAY);
    run(&mut f, &second, 10 * USDC).unwrap();

    assert_eq!(
        (f.balance(&user.usdc), f.balance(&user.sol)),
        (80 * USDC, 100_000_000)
    );
    assert_eq!(
        (f.balance(&first.usdc), f.balance(&second.usdc)),
        (10 * USDC, 10 * USDC)
    );
}

#[test]
fn a_policy_reaches_only_its_authoritys_accounts() {
    let mut f = Fixture::new();
    let victim = f.wallet(100 * USDC, 0);
    let thief = f.wallet(0, 0);
    let (t, usdc) = (thief.address().to_bytes(), f.usdc.to_bytes());
    let victim_usdc = victim.usdc.to_bytes();

    // The engine is the victim's delegate too, but this policy is the thief's
    let limits = limit(&victim_usdc, &usdc, 100 * USDC, Per::Total);
    let bytes = encode(&terms(&t, Some(&t), None, limits, None));
    let key = thief.address();
    f.send(&[create(&key, &key, &bytes)], &[&thief.key])
        .unwrap();
    let accounts = (victim.usdc, f.usdc, thief.usdc);
    let ix = pull(&key, &key, &bytes, accounts, USDC, None, TOKEN);
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidTarget
    ));
    assert_eq!(f.balance(&victim.usdc), 100 * USDC);
}

/// A Token-2022 transfer fee comes out of what arrives. The payer never gives
/// up more than the limit, and a payment in such a token is refused unless
/// the authority receives all of it.
#[test]
fn a_fee_token_never_shorts_the_authority() {
    use spl_token_2022_interface::extension::transfer_fee::instruction::initialize_transfer_fee_config;
    use spl_token_2022_interface::extension::ExtensionType;
    use spl_token_2022_interface::instruction::{approve, initialize_mint2, mint_to};
    use spl_token_2022_interface::{state::Mint, ID as TOKEN_2022};

    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let merchant = f.wallet(0, 0);
    let issuer = f.payer();

    // A mint that keeps 1% of every transfer
    let mint = Address::new_unique();
    let len = ExtensionType::try_calculate_account_len::<Mint>(&[ExtensionType::TransferFeeConfig])
        .unwrap();
    let mut account = solana_account::Account::new(SOL, len, &TOKEN_2022);
    account.lamports = f.svm.minimum_balance_for_rent_exemption(len);
    f.svm.set_account(mint, account).unwrap();
    let issuer_key = issuer.pubkey();
    let setup = [
        initialize_transfer_fee_config(&TOKEN_2022, &mint, None, None, 100, u64::MAX).unwrap(),
        initialize_mint2(&TOKEN_2022, &mint, &issuer_key, None, 6).unwrap(),
    ];
    f.send(&setup, &[&issuer]).unwrap();
    let mut account = |owner: &solana_keypair::Keypair| {
        let ata = litesvm_token::CreateAssociatedTokenAccount::new(&mut f.svm, owner, &mint)
            .token_program_id(&TOKEN_2022)
            .send()
            .unwrap();
        let fund = mint_to(&TOKEN_2022, &mint, &ata, &issuer_key, &[], 100 * USDC).unwrap();
        f.send(&[fund], &[&issuer]).unwrap();
        ata
    };
    let (user_fee, merchant_fee) = (account(&user.key), account(&merchant.key));
    let owner = user.address();
    let enable = approve(&TOKEN_2022, &user_fee, &ENGINE_KEY, &owner, &[], u64::MAX).unwrap();
    f.send(&[enable], &[&user.key]).unwrap();
    let held = |f: &Fixture, account: &Address| {
        let data = f.svm.get_account(account).unwrap().data;
        u64::from_le_bytes(data[64..72].try_into().unwrap())
    };
    let (u, m, fee, usdc) = (
        owner.to_bytes(),
        merchant.address().to_bytes(),
        mint.to_bytes(),
        f.usdc.to_bytes(),
    );
    let (user_fee_key, user_usdc) = (user_fee.to_bytes(), user.usdc.to_bytes());
    let spender = merchant.address();

    // Taking the fee token: the user gives up exactly the limit, the merchant gets it less the fee
    let limits = limit(&user_fee_key, &fee, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    f.send(&[create(&owner, &owner, &bytes)], &[&user.key])
        .unwrap();
    let accounts = (user_fee, mint, merchant_fee);
    let ix = pull(
        &spender,
        &owner,
        &bytes,
        accounts,
        10 * USDC,
        None,
        TOKEN_2022,
    );
    f.send(&[ix], &[&merchant.key]).unwrap();
    assert_eq!(held(&f, &user_fee), 90 * USDC);
    assert_eq!(held(&f, &merchant_fee), 110 * USDC - USDC / 10);

    // Receiving the fee token: what arrives is short, so the pull is refused
    let limits = limit(&user_usdc, &usdc, 10 * USDC, Per::Total);
    let receive = Receive {
        to: &user_fee_key,
        mint: &fee,
        min: 10 * USDC,
        decay: None,
    };
    let bytes = encode(&terms(&u, None, None, limits, Some(receive)));
    f.send(&[create(&owner, &owner, &bytes)], &[&user.key])
        .unwrap();
    let take = (user.usdc, f.usdc, merchant.usdc);
    let pay = Some((merchant_fee, mint, user_fee));
    let mut ix = pull(&spender, &owner, &bytes, take, 10 * USDC, pay, TOKEN);
    ix.accounts
        .push(solana_instruction::AccountMeta::new_readonly(
            TOKEN_2022, false,
        ));
    assert!(refused(
        f.send(&[ix], &[&merchant.key]),
        PullError::NotReceived
    ));
    assert_eq!(f.balance(&user.usdc), 100 * USDC);
}

/// A reference model of the limit, run against the program on random
/// policies, pulls and waits: the program accepts exactly what the model
/// accepts, so no sequence of pulls gets past the limit.
#[test]
fn random_pulls_never_pass_a_limit() {
    let mut seed = 0x9E37_79B9_7F4A_7C15u64;
    let mut random = |below: u64| {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed % below
    };

    let mut f = Fixture::new();
    let user = f.wallet(u64::MAX / 2, 0);
    let merchant = f.wallet(0, 0);
    let (u, m, usdc) = (
        user.address().to_bytes(),
        merchant.address().to_bytes(),
        f.usdc.to_bytes(),
    );
    let user_usdc = user.usdc.to_bytes();
    let (mut now, mut paid) = (NOW, 0u64);
    let (mut accepted_count, mut refused_count) = (0, 0);

    for salt in 0..150 {
        // One limit: a lifetime total, or a window of random length
        let per = match random(2) {
            0 => Per::Total,
            _ => Per::Every(1 + random(1_000) as u32),
        };
        let limits = limit(&user_usdc, &usdc, 1 + random(1_000), per);
        let mut terms = terms(&u, Some(&m), None, limits, None);
        terms.salt = salt;
        let bytes = encode(&terms);
        let create = create(&user.address(), &user.address(), &bytes);
        f.send(&[create], &[&user.key]).unwrap();

        let (mut consumed, mut rolled) = (0u64, 0i64);
        for _ in 0..25 {
            now += random(600) as i64;
            f.set_time(now);
            let amount = random(1_200);

            // The model: what the limit has spent now, and whether this pull fits
            let spent = match limits.per {
                Per::Total => consumed,
                Per::Every(seconds) => {
                    let window = |t: i64| (t - NOW).div_euclid(seconds as i64);
                    match window(now) == window(rolled) {
                        true => consumed,
                        false => 0,
                    }
                }
            };
            let fits = spent + amount <= limits.max;

            let accounts = (user.usdc, f.usdc, merchant.usdc);
            let spender = merchant.address();
            let ix = pull(
                &spender,
                &user.address(),
                &bytes,
                accounts,
                amount,
                None,
                TOKEN,
            );
            let accepted = f.send(&[ix], &[&merchant.key]).is_ok();
            assert_eq!(accepted, fits, "policy {salt}: {limits:?} pull {amount}");

            if accepted {
                paid += amount;
                consumed = spent + amount;
                rolled = now;
                accepted_count += 1;
            } else {
                refused_count += 1;
            }
        }
    }
    assert_eq!(f.balance(&merchant.usdc), paid);
    // The run must exercise both outcomes to mean anything
    assert!(accepted_count > 200 && refused_count > 200);
}

#[test]
fn engine_constant_is_the_derived_pda() {
    assert_eq!(pda(&[ENGINE_SEED]), ENGINE_KEY);
    let (_, bump) = Address::find_program_address(&[ENGINE_SEED], &PROGRAM);
    assert_eq!(bump, ENGINE_BUMP);
}
