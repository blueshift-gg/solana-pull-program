#![allow(clippy::result_large_err)] // LiteSVM's own result type

use pull_core::constants::*;
use pull_core::errors::PullError;
use pull_core::terms::{Per, Receive};
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
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();

    // The merchant may take at most 10 USDC every 30 days, until revoked
    let limits = limit(&user_usdc, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    let create = create(&user.address(), &user.address(), 1, &bytes);
    f.send(&[create], &[&user.key]).unwrap();

    let pull = |f: &mut Fixture, day: i64, who: &Wallet, amount: u64| {
        f.set_time(NOW + day * DAY);
        let accounts = (user.usdc, f.usdc, who.usdc);
        let ix = pull(
            &who.address(),
            &user.address(),
            1,
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
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();

    let limits = limit(&user_usdc, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    let create = create(&user.address(), &user.address(), 1, &bytes);
    f.send(&[create], &[&user.key]).unwrap();

    // The merchant's own program makes the pull, as it would while renewing a membership
    let accounts = (user.usdc, f.usdc, merchant.usdc);
    let m = merchant.address();
    let ix = by_cpi(pull(
        &m,
        &user.address(),
        1,
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
fn the_owner_ends_every_mandate_at_once_and_they_close() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (merchant, other) = (f.wallet(0, 0), f.wallet(0, 0));
    let (sponsor, stranger) = (f.payer(), f.payer());
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();
    let (owner, payer) = (user.address(), sponsor.pubkey());

    // Mandates are numbered as they are created: 1, 2, 3
    let limits = limit(&user_usdc, USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    for index in 1..=3 {
        // A mandate cannot take a number out of turn
        let skip = create(&owner, &payer, index + 1, &bytes);
        assert!(refused(
            f.send(&[skip], &[&sponsor, &user.key]),
            PullError::InvalidSeeds
        ));
        let create = create(&owner, &payer, index, &bytes);
        f.send(&[create], &[&sponsor, &user.key]).unwrap();
    }
    let pull = |f: &mut Fixture, index: u64| {
        let accounts = (user.usdc, f.usdc, merchant.usdc);
        let spender = merchant.address();
        let ix = pull(&spender, &owner, index, accounts, USDC, None, TOKEN);
        f.send(&[ix], &[&merchant.key])
    };
    pull(&mut f, 1).unwrap();

    // Another wallet's profile does not vouch for this one's mandates
    let accounts = (user.usdc, f.usdc, merchant.usdc);
    let spender = merchant.address();
    let mut ix = pull_for(&spender, &owner, 2, accounts, USDC, None, TOKEN, &[]);
    ix.accounts[2].pubkey = profile_pda(&other.address());
    assert!(refused(
        f.send(&[ix], &[&merchant.key]),
        PullError::InvalidProfile
    ));

    // One transaction ends all three, and only the owner can send it
    let mut forged = invalidate(&owner);
    forged.accounts[0].pubkey = stranger.pubkey();
    assert!(refused(
        f.send(&[forged], &[&stranger]),
        PullError::InvalidProfile
    ));
    // A fourth `Create`, signed now and held back, as a durable-nonce transaction can be
    let held = create(&owner, &payer, 4, &bytes);
    f.send(&[invalidate(&owner)], &[&user.key]).unwrap();
    // Ending them uses up number 4 too, so what was held can never land
    assert_eq!(f.profile(&owner), (4, 4));
    assert!(refused(
        f.send(&[held], &[&sponsor, &user.key]),
        PullError::InvalidSeeds
    ));
    for index in 1..=3 {
        assert!(refused(pull(&mut f, index), PullError::Stale));
    }

    // Anyone closes what was ended, and the rent goes back to who paid it
    let funded = f.svm.get_balance(&payer).unwrap();
    for index in 1..=3 {
        let mandate = mandate_pda(&owner, index);
        let ix = close(&stranger.pubkey(), &mandate, &payer, &owner);
        f.send(&[ix], &[&stranger]).unwrap();
        assert!(f.svm.get_account(&mandate).is_none_or(|a| a.lamports == 0));
    }
    assert!(f.svm.get_balance(&payer).unwrap() > funded);

    // The next mandate takes the next number, and works
    let create = create(&owner, &payer, 5, &bytes);
    f.send(&[create], &[&sponsor, &user.key]).unwrap();
    pull(&mut f, 5).unwrap();
    assert_eq!(f.balance(&merchant.usdc), 2 * USDC);
}

#[test]
fn closing_a_mandate_returns_the_rent_at_once() {
    let mut f = Fixture::new();
    let user = f.wallet(10 * USDC, 0);
    let (merchant, sponsor, stranger) = (f.wallet(0, 0), f.payer(), f.payer());
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();
    let (owner, payer) = (user.address(), sponsor.pubkey());

    // The user signs the transaction; a sponsor pays the fee and the rent.
    // One mandate never expires, the other does
    let limits = limit(&user_usdc, USDC, MONTHLY);
    let mandates = [mandate_pda(&owner, 1), mandate_pda(&owner, 2)];
    for (index, not_after) in [(1, None), (2, Some(NOW + 365 * DAY))] {
        let bytes = encode(&terms(&u, Some(&m), not_after, limits, None));
        let create = create(&owner, &payer, index, &bytes);
        f.send(&[create], &[&sponsor, &user.key]).unwrap();
    }
    let funded = f.svm.get_balance(&payer).unwrap();

    // Only the owner ends a mandate that is in force: not a stranger, not its
    // spender. And the rent goes only to its payer
    for other in [&stranger, &merchant.key] {
        let ix = close(&other.pubkey(), &mandates[0], &payer, &owner);
        assert!(refused(f.send(&[ix], &[other]), PullError::NotClosable));
    }
    let ix = close(&owner, &mandates[0], &owner, &owner);
    assert!(refused(
        f.send(&[ix], &[&user.key]),
        PullError::InvalidPayer
    ));
    let ix = close(&owner, &mandates[0], &payer, &owner);
    f.send(&[ix], &[&user.key]).unwrap();

    // Once a mandate has expired, anyone closes it
    let ix = close(&stranger.pubkey(), &mandates[1], &payer, &owner);
    assert!(refused(
        f.send(std::slice::from_ref(&ix), &[&stranger]),
        PullError::NotClosable
    ));
    f.set_time(NOW + 365 * DAY);
    f.send(&[ix], &[&stranger]).unwrap();

    for mandate in &mandates {
        assert!(f.svm.get_account(mandate).is_none_or(|a| a.lamports == 0));
    }
    assert!(f.svm.get_balance(&payer).unwrap() > funded);
}

#[test]
fn dca_runs_once_a_day_for_anyone_who_delivers() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (first, second) = (f.wallet(0, 10 * SOL), f.wallet(0, 10 * SOL));
    let u = user.address().to_bytes();
    let (user_usdc, user_sol) = (user.usdc.to_bytes(), user.sol.to_bytes());
    let owner = user.address();

    // Out: at most 10 USDC every day, to anyone. In: at least 0.05 SOL each time
    let limits = limit(&user_usdc, 10 * USDC, Per::Every(DAY as u32));
    let receive = Receive {
        to: &user_sol,
        min: 50_000_000,
    };
    let bytes = encode(&terms(&u, None, None, limits, Some(receive)));
    f.send(&[create(&owner, &owner, 1, &bytes)], &[&user.key])
        .unwrap();

    let run = |f: &mut Fixture, keeper: &Wallet, amount: u64| {
        let take = (user.usdc, f.usdc, keeper.usdc);
        let pay = Some((keeper.sol, f.sol, user.sol));
        let ix = pull(&keeper.address(), &owner, 1, take, amount, pay, TOKEN);
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
fn a_mandate_reaches_only_its_authoritys_accounts() {
    let mut f = Fixture::new();
    let victim = f.wallet(100 * USDC, 0);
    let thief = f.wallet(0, 0);
    let t = thief.address().to_bytes();
    let victim_usdc = victim.usdc.to_bytes();

    // The engine is the victim's delegate too, but this mandate is the thief's
    let limits = limit(&victim_usdc, 100 * USDC, Per::Total);
    let bytes = encode(&terms(&t, Some(&t), None, limits, None));
    let key = thief.address();
    f.send(&[create(&key, &key, 1, &bytes)], &[&thief.key])
        .unwrap();
    let accounts = (victim.usdc, f.usdc, thief.usdc);
    let ix = pull(&key, &key, 1, accounts, USDC, None, TOKEN);
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidTarget
    ));
    assert_eq!(f.balance(&victim.usdc), 100 * USDC);
}

/// The owner names where the money goes. Then whoever holds the spender's
/// key, or anyone if there is no spender, can only deliver it there.
#[test]
fn terms_that_name_a_destination_pay_only_there() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let (relayer, payee, thief) = (f.wallet(0, 0), f.wallet(0, 0), f.wallet(0, 0));
    let (u, r) = (user.address().to_bytes(), relayer.address().to_bytes());
    let (user_usdc, payee_usdc) = (user.usdc.to_bytes(), payee.usdc.to_bytes());
    let owner = user.address();

    // A mandate: the relayer may move 10 USDC a month, only to the payee
    let mut limits = limit(&user_usdc, 10 * USDC, MONTHLY);
    limits.to = Some(&payee_usdc);
    let bytes = encode(&terms(&u, Some(&r), None, limits, None));
    f.send(&[create(&owner, &owner, 1, &bytes)], &[&user.key])
        .unwrap();
    let key = relayer.address();
    let elsewhere = (user.usdc, f.usdc, relayer.usdc);
    let ix = pull(&key, &owner, 1, elsewhere, USDC, None, TOKEN);
    assert!(refused(
        f.send(&[ix], &[&relayer.key]),
        PullError::InvalidDestination
    ));
    let there = (user.usdc, f.usdc, payee.usdc);
    let ix = pull(&key, &owner, 1, there, USDC, None, TOKEN);
    f.send(&[ix], &[&relayer.key]).unwrap();

    // A mandate with no spender: anyone lands the payout, and it still goes to the payee
    let mut limits = limit(&user_usdc, 50 * USDC, Per::Total);
    limits.to = Some(&payee_usdc);
    let bytes = encode(&terms(&u, None, None, limits, None));
    f.send(&[create(&owner, &owner, 2, &bytes)], &[&user.key])
        .unwrap();
    let stolen = (user.usdc, f.usdc, thief.usdc);
    let key = thief.address();
    let ix = pull(&key, &owner, 2, stolen, 50 * USDC, None, TOKEN);
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidDestination
    ));
    let ix = pull(&key, &owner, 2, there, 50 * USDC, None, TOKEN);
    f.send(&[ix], &[&thief.key]).unwrap();

    assert_eq!(f.balance(&payee.usdc), 51 * USDC);
    assert_eq!((f.balance(&relayer.usdc), f.balance(&thief.usdc)), (0, 0));
}

/// Whoever is recorded as the payer signed for it. Otherwise a merchant
/// funds the address ahead and names a payer the rent can never go back to,
/// and the owner can never cancel.
#[test]
fn the_payer_of_a_mandate_signed_for_it() {
    let mut f = Fixture::new();
    let user = f.wallet(10 * USDC, 0);
    let merchant = f.wallet(0, 0);
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();
    let owner = user.address();
    let bytes = encode(&terms(
        &u,
        Some(&m),
        None,
        limit(&user_usdc, USDC, MONTHLY),
        None,
    ));

    // The address holds its rent already, so no transfer needs the payer
    let mandate = mandate_pda(&owner, 1);
    f.svm.airdrop(&mandate, SOL).unwrap();
    for payer in [mandate, solana_address::Address::new_unique()] {
        let mut ix = create(&owner, &owner, 1, &bytes);
        ix.accounts[1] = solana_instruction::AccountMeta::new(payer, false);
        assert!(refused(f.send(&[ix], &[&user.key]), PullError::NotSigner));
    }
    // With a payer that signs, the owner creates it and can close it
    f.send(&[create(&owner, &owner, 1, &bytes)], &[&user.key])
        .unwrap();
    f.send(&[close(&owner, &mandate, &owner, &owner)], &[&user.key])
        .unwrap();
}

/// Every account an instruction takes is the one the terms or the authority
/// name. Each of these swaps one for another that looks the part.
#[test]
fn substituted_accounts_are_refused() {
    let mut f = Fixture::new();
    let (victim, thief) = (f.wallet(100 * USDC, 0), f.wallet(100 * USDC, 10 * SOL));
    let user = f.wallet(100 * USDC, 0);
    let (t, u, usdc) = (
        thief.address().to_bytes(),
        user.address().to_bytes(),
        f.usdc.to_bytes(),
    );
    let (victim_usdc, user_usdc, user_sol) = (
        victim.usdc.to_bytes(),
        user.usdc.to_bytes(),
        user.sol.to_bytes(),
    );
    let (owner, key) = (user.address(), thief.address());
    let take = (user.usdc, f.usdc, thief.usdc);

    // A mandate the thief creates for the victim's account, under its own profile
    let stolen = terms(
        &t,
        Some(&t),
        None,
        limit(&victim_usdc, USDC, Per::Total),
        None,
    );
    f.send(&[create(&key, &key, 1, &encode(&stolen))], &[&thief.key])
        .unwrap();
    let from_victim = (victim.usdc, f.usdc, thief.usdc);
    let ix = pull(&key, &key, 1, from_victim, USDC, None, TOKEN);
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidTarget
    ));

    // A token-program account of multisig length, dressed as the victim's token account
    let fake = solana_address::Address::new_unique();
    let mut data = vec![0; 355];
    data[..32].copy_from_slice(&usdc);
    data[32..64].copy_from_slice(&t);
    data[64..72].copy_from_slice(&USDC.to_le_bytes());
    data[72..76].copy_from_slice(&[1, 0, 0, 0]);
    data[76..108].copy_from_slice(&ENGINE);
    data[121..129].copy_from_slice(&u64::MAX.to_le_bytes());
    data[165] = 2;
    let mut account = solana_account::Account::new(SOL, 355, &TOKEN);
    account.data = data;
    f.svm.set_account(fake, account).unwrap();
    let fake_key = fake.to_bytes();
    let mut forged = stolen;
    forged.limit.from = &fake_key;
    f.send(&[create(&key, &key, 2, &encode(&forged))], &[&thief.key])
        .unwrap();
    let from_fake = (fake, f.usdc, thief.usdc);
    let ix = pull(&key, &key, 2, from_fake, USDC, None, TOKEN);
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidTarget
    ));

    // The user sells 10 USDC for 1 SOL, to anyone
    let receive = Receive {
        to: &user_sol,
        min: SOL,
    };
    let limits = limit(&user_usdc, 10 * USDC, Per::Total);
    let order = encode(&terms(&u, None, None, limits, Some(receive)));
    f.send(&[create(&owner, &owner, 1, &order)], &[&user.key])
        .unwrap();
    let pull_with = |f: &mut Fixture, change: &dyn Fn(&mut solana_instruction::Instruction)| {
        let pay = Some((thief.sol, f.sol, user.sol));
        let mut ix = pull(&key, &owner, 1, take, 10 * USDC, pay, TOKEN);
        change(&mut ix);
        f.send(&[ix], &[&thief.key])
    };
    // Accounts of `Pull`: 1 mandate, 2 profile, then the legs; 9 pay_from, 11 pay_to
    // Paying into its own account instead of the user's
    assert!(refused(
        pull_with(&mut f, &|ix| ix.accounts[11].pubkey = thief.sol),
        PullError::InvalidTarget
    ));
    // "Paying" the user from the user's own account, or from an account no token program owns
    assert!(pull_with(&mut f, &|ix| ix.accounts[9].pubkey = user.sol).is_err());
    assert!(pull_with(&mut f, &|ix| ix.accounts[9].pubkey = profile_pda(&owner)).is_err());
    // Another authority's profile, or the mandate passed as its own profile
    assert!(refused(
        pull_with(&mut f, &|ix| ix.accounts[2].pubkey = profile_pda(&key)),
        PullError::InvalidProfile
    ));
    assert!(refused(
        pull_with(&mut f, &|ix| ix.accounts[2].pubkey = mandate_pda(&owner, 1)),
        PullError::InvalidProfile
    ));
    assert_eq!(f.balance(&user.usdc), 100 * USDC);
    pull_with(&mut f, &|_| ()).unwrap();
    assert_eq!(f.balance(&user.sol), SOL);

    // `Close` takes only a mandate
    let mandate = mandate_pda(&owner, 1);
    // The thief ended its own mandates; that does not end the user's
    f.send(&[invalidate(&key)], &[&thief.key]).unwrap();
    let ix = close(&key, &mandate, &owner, &key);
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidProfile
    ));
    let mut ix = close(&key, &profile_pda(&owner), &owner, &owner);
    ix.accounts[1].is_writable = true;
    assert!(refused(
        f.send(&[ix], &[&thief.key]),
        PullError::InvalidProfile
    ));
}

/// A pull the token program would refuse says why in this program's own
/// codes, so a merchant knows whether to ask the owner to enable again or to
/// top up.
#[test]
fn a_refused_pull_names_its_cause() {
    let mut f = Fixture::new();
    let user = f.fresh_wallet(100 * USDC, 0);
    let merchant = f.wallet(0, 0);
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();

    let limits = limit(&user_usdc, 1_000 * USDC, Per::Total);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    let create = create(&user.address(), &user.address(), 1, &bytes);
    f.send(&[create], &[&user.key]).unwrap();

    let pull = |f: &mut Fixture, amount: u64| {
        let accounts = (user.usdc, f.usdc, merchant.usdc);
        let m = merchant.address();
        let ix = pull(&m, &user.address(), 1, accounts, amount, None, TOKEN);
        f.send(&[ix], &[&merchant.key])
    };
    // Never enabled
    assert!(refused(pull(&mut f, USDC), PullError::NotDelegate));
    // Enabled for 5 USDC: more is refused, and all of it uses the approval up
    f.enable(&user, &user.usdc, 5 * USDC);
    assert!(refused(
        pull(&mut f, 6 * USDC),
        PullError::AllowanceExceeded
    ));
    pull(&mut f, 5 * USDC).unwrap();
    assert!(refused(pull(&mut f, USDC), PullError::NotDelegate));
    // Enabled again, without a cap: the balance is what is left to run out
    f.enable(&user, &user.usdc, u64::MAX);
    assert!(refused(
        pull(&mut f, 96 * USDC),
        PullError::InsufficientFunds
    ));
    pull(&mut f, 95 * USDC).unwrap();
}

/// A spender tags a pull with its own id, and the event carries it.
#[test]
fn a_pull_carries_the_spenders_reference() {
    let mut f = Fixture::new();
    let user = f.wallet(100 * USDC, 0);
    let merchant = f.wallet(0, 0);
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();

    let limits = limit(&user_usdc, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    let create = create(&user.address(), &user.address(), 1, &bytes);
    f.send(&[create], &[&user.key]).unwrap();

    let pull = |f: &mut Fixture, reference: &[u8]| {
        let accounts = (user.usdc, f.usdc, merchant.usdc);
        let (m, u) = (merchant.address(), user.address());
        let ix = pull_for(&m, &u, 1, accounts, USDC, None, TOKEN, reference);
        f.send(&[ix], &[&merchant.key])
    };
    let event = |meta: litesvm::types::TransactionMetadata| {
        let inner = meta.inner_instructions.concat();
        inner.last().unwrap().instruction.data.clone()
    };
    let invoice = [7; 32];
    assert!(event(pull(&mut f, &invoice).unwrap()).ends_with(&invoice));
    assert!(event(pull(&mut f, &[]).unwrap()).ends_with(&[0; 32]));
    // A reference is 32 bytes or absent
    assert!(pull(&mut f, &[7; 31]).is_err());
    assert_eq!(f.balance(&merchant.usdc), 2 * USDC);
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
    let (u, m) = (owner.to_bytes(), merchant.address().to_bytes());
    let (user_fee_key, user_usdc) = (user_fee.to_bytes(), user.usdc.to_bytes());
    let spender = merchant.address();

    // Taking the fee token: the user gives up exactly the limit, the merchant gets it less the fee
    let limits = limit(&user_fee_key, 10 * USDC, MONTHLY);
    let bytes = encode(&terms(&u, Some(&m), None, limits, None));
    f.send(&[create(&owner, &owner, 1, &bytes)], &[&user.key])
        .unwrap();
    let accounts = (user_fee, mint, merchant_fee);
    let ix = pull(&spender, &owner, 1, accounts, 10 * USDC, None, TOKEN_2022);
    f.send(&[ix], &[&merchant.key]).unwrap();
    assert_eq!(held(&f, &user_fee), 90 * USDC);
    assert_eq!(held(&f, &merchant_fee), 110 * USDC - USDC / 10);

    // Receiving the fee token: what arrives is short, so the pull is refused
    let limits = limit(&user_usdc, 10 * USDC, Per::Total);
    let receive = Receive {
        to: &user_fee_key,
        min: 10 * USDC,
    };
    let bytes = encode(&terms(&u, None, None, limits, Some(receive)));
    f.send(&[create(&owner, &owner, 2, &bytes)], &[&user.key])
        .unwrap();
    let take = (user.usdc, f.usdc, merchant.usdc);
    let pay = Some((merchant_fee, mint, user_fee));
    let mut ix = pull(&spender, &owner, 2, take, 10 * USDC, pay, TOKEN);
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
/// mandates, pulls and waits: the program accepts exactly what the model
/// accepts, so no sequence of pulls gets past the limit. The model steps
/// through the windows where the program divides, so the two do not share
/// a formula.
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
    let (u, m) = (user.address().to_bytes(), merchant.address().to_bytes());
    let user_usdc = user.usdc.to_bytes();
    let (mut now, mut paid) = (NOW, 0u64);
    let (mut accepted_count, mut refused_count) = (0, 0);

    for salt in 0..150 {
        // One limit: a lifetime total, or a window of random length
        let per = match random(2) {
            0 => Per::Total,
            _ => Per::Every(1 + random(1_000) as u32),
        };
        let limits = limit(&user_usdc, 1 + random(1_000), per);
        // Windows count from the start of the terms, which is not when they are created
        let start = now - random(5_000) as i64;
        let mut terms = terms(&u, Some(&m), None, limits, None);
        terms.not_before = start;
        let bytes = encode(&terms);
        let create = create(&user.address(), &user.address(), salt + 1, &bytes);
        f.send(&[create], &[&user.key]).unwrap();

        // The model walks the windows one by one instead of dividing: what is
        // spent in the window that holds `now`, and where that window ends
        let (mut spent, mut window_end) = (0u64, start);
        for _ in 0..25 {
            now += random(600) as i64;
            f.set_time(now);
            let amount = random(1_200);

            if let Per::Every(seconds) = limits.per {
                while now >= window_end {
                    window_end += seconds as i64;
                    spent = 0;
                }
            }
            let fits = spent + amount <= limits.max;

            let accounts = (user.usdc, f.usdc, merchant.usdc);
            let spender = merchant.address();
            let ix = pull(
                &spender,
                &user.address(),
                salt + 1,
                accounts,
                amount,
                None,
                TOKEN,
            );
            let accepted = f.send(&[ix], &[&merchant.key]).is_ok();
            assert_eq!(accepted, fits, "mandate {salt}: {limits:?} pull {amount}");

            if accepted {
                paid += amount;
                spent += amount;
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
