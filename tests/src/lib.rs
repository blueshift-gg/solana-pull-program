//! Test fixture: LiteSVM with the Pull program, a USDC-like and a SOL-like
//! mint, funded wallets with a profile that enabled the engine, and a builder
//! per instruction.
//!
//! Build the program and the CPI caller fixture first:
//! `cargo build-sbf --manifest-path program/Cargo.toml --features localnet`
//! `cargo build-sbf --manifest-path tests/caller/Cargo.toml`.

use ed25519_dalek::{Signer as _, SigningKey};
use litesvm::{types::TransactionResult, LiteSVM};
use litesvm_token::{
    get_spl_account, spl_token, Approve, CreateAssociatedTokenAccount, CreateMint, MintTo,
};
use pull_core::render::render;
pub use pull_core::render::Envelope;
use pull_core::terms::{Limit, Per, Receive, Terms};
use pull_core::{constants::*, errors::PullError};
use solana_address::Address;
use solana_clock::Clock;
use solana_instruction::{AccountMeta, Instruction};
use solana_keypair::Keypair;
use solana_signer::Signer;
use solana_transaction::Transaction;

pub const PROGRAM: Address = Address::new_from_array(pull_core::ID);
/// Where the fixture loads `tests/caller`, a program that forwards to the Pull program.
pub const CALLER: Address = Address::from_str_const("Ca11er1111111111111111111111111111111111111");
pub const ENGINE_KEY: Address = Address::new_from_array(ENGINE);
pub const TOKEN: Address = litesvm_token::TOKEN_ID;
pub const SYSTEM: Address = Address::new_from_array([0; 32]);
/// 2026-09-21T14:13:20Z.
pub const NOW: i64 = 1_790_000_000;
pub const USDC: u64 = 1_000_000;
pub const SOL: u64 = 1_000_000_000;

pub struct Wallet {
    pub key: Keypair,
    pub usdc: Address,
    pub sol: Address,
}

impl Wallet {
    pub fn address(&self) -> Address {
        self.key.pubkey()
    }
}

pub struct Fixture {
    pub svm: LiteSVM,
    pub usdc: Address,
    pub sol: Address,
    mint_authority: Keypair,
}

impl Default for Fixture {
    fn default() -> Self {
        Self::new()
    }
}

impl Fixture {
    pub fn new() -> Self {
        let mut svm = LiteSVM::new();
        let so = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/deploy/pull_program.so"
        );
        svm.add_program_from_file(PROGRAM, so)
            .expect("build the program first");
        let caller = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../target/deploy/pull_caller.so"
        );
        svm.add_program_from_file(CALLER, caller)
            .expect("build tests/caller first");
        let mint_authority = Keypair::new();
        svm.airdrop(&mint_authority.pubkey(), 10 * SOL).unwrap();
        let usdc = CreateMint::new(&mut svm, &mint_authority)
            .decimals(6)
            .send()
            .unwrap();
        let sol = CreateMint::new(&mut svm, &mint_authority)
            .decimals(9)
            .send()
            .unwrap();
        let mut f = Self {
            svm,
            usdc,
            sol,
            mint_authority,
        };
        f.set_time(NOW);
        f
    }

    /// A funded wallet whose token accounts delegate to the engine.
    pub fn wallet(&mut self, usdc: u64, sol: u64) -> Wallet {
        let wallet = self.fresh_wallet(usdc, sol);
        self.enable(&wallet, &wallet.usdc, u64::MAX);
        self.enable(&wallet, &wallet.sol, u64::MAX);
        wallet
    }

    /// A funded wallet with a profile, whose token accounts are not enabled yet.
    pub fn fresh_wallet(&mut self, usdc: u64, sol: u64) -> Wallet {
        let wallet = self.unopened_wallet(usdc, sol);
        let key = wallet.address();
        self.send(&[open(&key, &key)], &[&wallet.key]).unwrap();
        wallet
    }

    /// A funded wallet that has no profile.
    pub fn unopened_wallet(&mut self, usdc: u64, sol: u64) -> Wallet {
        let key = Keypair::new();
        let Self {
            svm,
            mint_authority,
            ..
        } = self;
        svm.airdrop(&key.pubkey(), 10 * SOL).unwrap();
        let mut account = |mint: Address, amount: u64| {
            let ata = CreateAssociatedTokenAccount::new(svm, &key, &mint)
                .send()
                .unwrap();
            MintTo::new(svm, mint_authority, &mint, &ata, amount)
                .send()
                .unwrap();
            ata
        };
        let (usdc, sol) = (account(self.usdc, usdc), account(self.sol, sol));
        Wallet { key, usdc, sol }
    }

    /// A funded keypair with no token accounts: a solver that holds nothing.
    pub fn payer(&mut self) -> Keypair {
        let key = Keypair::new();
        self.svm.airdrop(&key.pubkey(), 10 * SOL).unwrap();
        key
    }

    /// The one-time setup for a token: approve the engine as delegate. `cap`
    /// is the budget every policy on this account shares; `u64::MAX` for none.
    pub fn enable(&mut self, wallet: &Wallet, account: &Address, cap: u64) {
        Approve::new(&mut self.svm, &wallet.key, &ENGINE_KEY, account, cap)
            .send()
            .unwrap();
    }

    /// What the engine may still pull from `account`, across every policy.
    pub fn allowance(&self, account: &Address) -> u64 {
        get_spl_account::<spl_token::state::Account>(&self.svm, account)
            .unwrap()
            .delegated_amount
    }

    pub fn balance(&self, account: &Address) -> u64 {
        get_spl_account::<spl_token::state::Account>(&self.svm, account)
            .unwrap()
            .amount
    }

    /// The profile of `authority`: `(policies, stale, nonce_index)`.
    pub fn profile(&self, authority: &Address) -> (u64, u64, u64) {
        let data = self.svm.get_account(&profile_pda(authority)).unwrap().data;
        let field = |at: usize| u64::from_le_bytes(data[at..at + 8].try_into().unwrap());
        (field(33), field(41), field(49))
    }

    /// What an intent of `authority` is signed under now.
    pub fn nonce_index(&self, authority: &Address) -> u64 {
        self.profile(authority).2
    }

    pub fn set_time(&mut self, unix_timestamp: i64) {
        let mut clock: Clock = self.svm.get_sysvar();
        clock.unix_timestamp = unix_timestamp;
        self.svm.set_sysvar(&clock);
    }

    /// Send `ixs`, paid by the first signer.
    #[allow(clippy::result_large_err)] // LiteSVM's own result type
    pub fn send(&mut self, ixs: &[Instruction], signers: &[&Keypair]) -> TransactionResult {
        self.svm.expire_blockhash();
        let tx = Transaction::new_signed_with_payer(
            ixs,
            Some(&signers[0].pubkey()),
            signers,
            self.svm.latest_blockhash(),
        );
        self.svm.send_transaction(tx)
    }

    pub fn decimals(&self) -> impl Fn(&[u8; 32]) -> Result<u8, PullError> + '_ {
        |mint| match Address::new_from_array(*mint) {
            m if m == self.usdc => Ok(6),
            m if m == self.sol => Ok(9),
            _ => Err(PullError::InvalidTarget),
        }
    }
}

pub fn pda(seeds: &[&[u8]]) -> Address {
    Address::find_program_address(seeds, &PROGRAM).0
}

pub fn encode(terms: &Terms) -> Vec<u8> {
    let mut bytes = Vec::new();
    terms.write(&mut bytes);
    bytes
}

/// The canonical text a wallet shows for an intent signed under nonce `index`.
pub fn text(
    terms: &Terms,
    index: u64,
    decimals: impl Fn(&[u8; 32]) -> Result<u8, PullError>,
) -> String {
    let mut out = Vec::new();
    render(terms, index, decimals, &mut out).unwrap();
    String::from_utf8(out).unwrap()
}

/// What a wallet signs through `solana:signOffchainMessage`: the text in an
/// Offchain Message v1.
pub fn sign(
    terms: &Terms,
    index: u64,
    key: &Keypair,
    decimals: impl Fn(&[u8; 32]) -> Result<u8, PullError>,
) -> [u8; 64] {
    sign_as(Envelope::OffchainMessage, terms, index, key, decimals)
}

/// What a wallet signs for an intent, with or without an envelope.
pub fn sign_as(
    envelope: Envelope,
    terms: &Terms,
    index: u64,
    key: &Keypair,
    decimals: impl Fn(&[u8; 32]) -> Result<u8, PullError>,
) -> [u8; 64] {
    let mut message = Vec::new();
    envelope.put(terms.authority, &mut message);
    render(terms, index, decimals, &mut message).unwrap();
    let secret: [u8; 32] = key.to_bytes()[..32].try_into().unwrap();
    SigningKey::from_bytes(&secret).sign(&message).to_bytes()
}

/// "At most `max` of `mint` may leave `from`", to wherever the spender says.
pub fn limit<'a>(from: &'a [u8; 32], mint: &'a [u8; 32], max: u64, per: Per) -> Limit<'a> {
    Limit {
        from,
        mint,
        max,
        per,
        to: None,
    }
}

/// Terms valid from `NOW`, with salt 0.
pub fn terms<'a>(
    authority: &'a [u8; 32],
    spender: Option<&'a [u8; 32]>,
    not_after: Option<i64>,
    limit: Limit<'a>,
    receive: Option<Receive<'a>>,
) -> Terms<'a> {
    Terms {
        cluster: CLUSTER,
        authority,
        spender,
        not_before: NOW,
        not_after,
        salt: 0,
        limit,
        receive,
    }
}

pub fn profile_pda(authority: &Address) -> Address {
    pda(&[PROFILE_SEED, authority.as_ref()])
}

/// Create the profile of `authority`: once, before its first policy or intent.
pub fn open(authority: &Address, payer: &Address) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new(profile_pda(authority), false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(ENGINE_KEY, false),
            AccountMeta::new_readonly(PROGRAM, false),
        ],
        data: vec![3],
    }
}

pub const POLICIES: u8 = 1;
pub const INTENTS: u8 = 2;

/// End every policy of `authority` so far, every intent, or both.
pub fn invalidate(authority: &Address, what: u8) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(profile_pda(authority), false),
            AccountMeta::new_readonly(ENGINE_KEY, false),
            AccountMeta::new_readonly(PROGRAM, false),
        ],
        data: vec![4, what],
    }
}

/// The `index`-th policy of `authority`, counted from 1.
pub fn policy_pda(authority: &Address, index: u64) -> Address {
    pda(&[POLICY_SEED, authority.as_ref(), &index.to_le_bytes()])
}

/// Put a policy on chain as the authority's `index`-th: the authority signs,
/// `payer` funds the rent.
pub fn create(authority: &Address, payer: &Address, index: u64, bytes: &[u8]) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(*payer, true),
            AccountMeta::new(profile_pda(authority), false),
            AccountMeta::new(policy_pda(authority, index), false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(ENGINE_KEY, false),
            AccountMeta::new_readonly(PROGRAM, false),
        ],
        data: [&[0][..], bytes].concat(),
    }
}

/// `(from, mint, to)`: the token account taken from, its mint, and where the tokens go.
pub type Take = (Address, Address, Address);
/// `(pay_from, pay_mint, pay_to)`: the spender's account, the mint the authority receives, the account the terms name.
pub type Pay = (Address, Address, Address);

fn legs((from, mint, to): Take, payment: Option<Pay>, token_program: Address) -> Vec<AccountMeta> {
    let mut accounts = vec![
        AccountMeta::new(from, false),
        AccountMeta::new_readonly(mint, false),
        AccountMeta::new(to, false),
        AccountMeta::new_readonly(ENGINE_KEY, false),
        AccountMeta::new_readonly(PROGRAM, false),
        AccountMeta::new_readonly(token_program, false),
    ];
    if let Some((pay_from, pay_mint, pay_to)) = payment {
        accounts.extend([
            AccountMeta::new(pay_from, false),
            AccountMeta::new_readonly(pay_mint, false),
            AccountMeta::new(pay_to, false),
        ]);
    }
    accounts
}

/// `spender` takes `amount` under the `index`-th policy of `authority`,
/// paying what the authority receives if the terms say so.
pub fn pull(
    spender: &Address,
    authority: &Address,
    index: u64,
    take: Take,
    amount: u64,
    payment: Option<Pay>,
    token_program: Address,
) -> Instruction {
    pull_for(
        spender,
        authority,
        index,
        take,
        amount,
        payment,
        token_program,
        &[],
    )
}

/// A pull that carries the spender's `reference` for it: 32 bytes, or none.
#[allow(clippy::too_many_arguments)]
pub fn pull_for(
    spender: &Address,
    authority: &Address,
    index: u64,
    take: Take,
    amount: u64,
    payment: Option<Pay>,
    token_program: Address,
    reference: &[u8],
) -> Instruction {
    let mut accounts = vec![
        AccountMeta::new_readonly(*spender, true),
        AccountMeta::new(policy_pda(authority, index), false),
        AccountMeta::new_readonly(profile_pda(authority), false),
    ];
    accounts.extend(legs(take, payment, token_program));
    Instruction {
        program_id: PROGRAM,
        accounts,
        data: [&[1][..], &amount.to_le_bytes(), reference].concat(),
    }
}

/// The page that holds the nonce of an intent of `authority` with this salt,
/// signed under nonce `index`.
pub fn nonces_pda(authority: &Address, index: u64, salt: u64) -> Address {
    let page = (salt / NONCE_BITS as u64).to_le_bytes();
    pda(&[NONCES_SEED, authority.as_ref(), &index.to_le_bytes(), &page])
}

/// `spender` runs an intent signed under nonce `index` once, taking `amount`.
/// It pays for the page of nonces. The signature is over an Offchain Message
/// v1, as `sign` makes it.
pub fn fill(
    spender: &Address,
    terms: &Terms,
    index: u64,
    signature: &[u8; 64],
    take: Take,
    amount: u64,
    payment: Option<Pay>,
) -> Instruction {
    let authority = Address::new_from_array(*terms.authority);
    let mut accounts = vec![
        AccountMeta::new_readonly(*spender, true),
        AccountMeta::new(*spender, true),
        AccountMeta::new_readonly(profile_pda(&authority), false),
        AccountMeta::new(nonces_pda(&authority, index, terms.salt), false),
        AccountMeta::new_readonly(SYSTEM, false),
    ];
    accounts.extend(legs(take, payment, TOKEN));
    Instruction {
        program_id: PROGRAM,
        accounts,
        data: [
            &[10][..],
            &amount.to_le_bytes(),
            &[1],
            signature,
            &encode(terms),
        ]
        .concat(),
    }
}

/// The authority uses up the nonce of one intent signed under nonce `index`.
pub fn cancel(authority: &Address, index: u64, salt: u64) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(*authority, true),
            AccountMeta::new_readonly(profile_pda(authority), false),
            AccountMeta::new(nonces_pda(authority, index, salt), false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(ENGINE_KEY, false),
            AccountMeta::new_readonly(PROGRAM, false),
        ],
        data: [&[11][..], &salt.to_le_bytes()].concat(),
    }
}

/// The authority ends one policy; its rent goes to `payer`, the account that paid it.
pub fn cancel_policy(authority: &Address, policy: &Address, payer: &Address) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(*authority, true),
            AccountMeta::new(*payer, false),
            AccountMeta::new_readonly(profile_pda(authority), false),
            AccountMeta::new(*policy, false),
            AccountMeta::new_readonly(SYSTEM, false),
            AccountMeta::new_readonly(ENGINE_KEY, false),
            AccountMeta::new_readonly(PROGRAM, false),
        ],
        data: vec![11],
    }
}

/// Anyone closes a policy or a page of nonces of `authority` that can never
/// be used again; its rent goes to `payer`, the account that paid it.
pub fn close(
    closer: &Address,
    account: &Address,
    payer: &Address,
    authority: &Address,
) -> Instruction {
    Instruction {
        program_id: PROGRAM,
        accounts: vec![
            AccountMeta::new_readonly(*closer, true),
            AccountMeta::new(*account, false),
            AccountMeta::new(*payer, false),
            AccountMeta::new_readonly(profile_pda(authority), false),
            AccountMeta::new_readonly(ENGINE_KEY, false),
            AccountMeta::new_readonly(PROGRAM, false),
        ],
        data: vec![2],
    }
}

/// The same instruction, sent to the caller fixture, which forwards it by CPI.
pub fn by_cpi(mut ix: Instruction) -> Instruction {
    ix.program_id = CALLER;
    ix
}
