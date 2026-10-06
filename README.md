# Solana Pull Program

A Solana program that lets a wallet owner permit someone to take tokens from its
account, within a limit, without giving up custody.

An owner states what may go out and, if it wants, what must come in. It can give that
permission in two ways:

- A **policy** is a standing permission on chain. The owner creates it with a
  transaction, its spender uses it again and again, and a wallet can list it and cancel it.
- An **intent** is one action the owner signs as text. Whoever it permits runs it once.
  Nothing goes on chain first, and the owner sends no transaction.

```text
Once      Approve + Open
Policy    Create  →  Pull, Pull, Pull …  →  Cancel
Intent    sign    →  Fill
Undo      Cancel one policy or one intent · Invalidate all of either
```

`Pull` and `Fill` move tokens out of the owner's account as its SPL delegate, within the
limit. If the terms say what the owner must receive, the same instruction moves it from
the spender to the owner and checks what arrived. Funds stay in the owner's wallet until
then, and there is nothing between the two transfers to trust. Another program can call
`Pull`, for example to collect a payment and update its own state together.

| Use | As | Spender | Out | In |
|---|---|---|---|---|
| Subscription | policy | the merchant | 8 USDC every 30 days, only to the merchant's account | nothing |
| Agent budget | policy | the API provider | 1 USDC every day | nothing |
| DCA | policy | anyone | 10 USDC every day | at least 0.05 SOL each time |
| Authorize, then capture | intent | the merchant | at most 120 USDC | nothing |
| One payment signed in advance | intent | a relayer, or anyone | 100 USDC, only to the payee's account | nothing |
| Swap, limit order, Dutch auction | intent | anyone | 100 USDC | at least 0.52 SOL, falling to 0.50 |

## Terms

A policy and an intent carry the same terms.

```text
Terms   { authority, spender: key | anyone, not_before, not_after?, salt, limit, receive? }
Limit   { from, mint, max, per: total | every(seconds), to? }
Receive { to, mint, min, decay?: (t0, t1, min) }
```

`limit` is what may go out: "at most `max` of `mint` may leave `from`", a token account
of the owner. With `to` it also says where: the tokens may go only to that token
account, so whoever holds the spender's key can deliver the money but not redirect it.
Without `to` the spender chooses.

A policy counts the limit in total, or over fixed windows that start at `not_before`;
what is unused in a window does not carry over. An intent runs once, so its limit is the
most that one use may take.

`receive` is what must come in: "each use, at least `min` of `mint` arrives in `to`", an
account of the owner. It is a total, not a rate: a spender that takes less still
delivers all of it. With a decay the minimum moves in a straight line between two times.

Terms are valid only if they bind something: a named spender, a named destination, or
something the owner receives. Terms that leave all three open would pay whoever finds
them.

`salt` is an intent's nonce. A policy does not use it.

## Profile

Each owner has one profile, opened in the transaction that enables its first token
account. It holds three numbers:

```text
Profile { policies, stale, nonce_index }
```

`policies` counts the owner's policies, which are numbered 1, 2, 3 as they are created.
Every policy numbered `stale` or less is invalid. `nonce_index` is what the owner's
intents are signed under.

`Invalidate` ends everything of one kind in one transaction. For policies it sets `stale`
to `policies`; policies created afterwards work as usual. For intents it moves
`nonce_index`, by an amount taken from a slot hash, so nobody can hold a signature made
ahead for the next value. The two are separate: ending every intent does not end a
subscription.

Whatever was ended can be closed by anyone, and its rent goes back to whoever paid it.

## Policies

A policy is an account at a PDA of the owner and its number. It holds the terms, how
much of the limit is used, and who paid its rent. Only a transaction signed by the owner
creates one, so everything a spender can do is on chain. A wallet lists an owner's
policies by their numbers, with no scan of the program's accounts.

Whoever pays the rent is always a separate account from whoever signs, so a merchant can
sponsor a subscriber. The owner ends a policy at any time with `Cancel`, which closes it
at once. Anyone closes one that has expired or that the owner invalidated. Either way it
is gone for good, and the rent goes straight back to its payer.

## Intents

An owner signs this text:

```text
Solana Pull v1
cluster: localnet
engine: PULLrgDYqK1yFKVTSbWieX3ARP7U2XUyrjxWXqKgVzA
authority: A9XwnWUxXn1HH1MPzxe5MqfYaKvEHtdQMCoDd72QjPLN
SPENDER: anyone
MAY TAKE: at most 100.000000 of mint EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v from 6NSx1jcpyqzHDFHwC7RXm4LZy53gNsMZWpV3P8vr8k4M in total
MUST RECEIVE: at least 0.520000000 of mint So11111111111111111111111111111111111111112 in FAUD3SfhKYyynnZsS8pKVgZ9ea5VQohFqpaxKpAwzEGA for each use, moving to 0.500000000 from 2026-10-01T08:16:00Z to 2026-10-01T08:21:00Z
VALID: from 2026-10-01T08:16:00Z until 2026-10-01T09:16:00Z
INDEX: 10883731594980722182
SALT: 0
```

`Fill` takes the terms as binary, renders them back to this text, and verifies Ed25519
over it in-program. The text is all a wallet has to show, and a byte of the terms cannot
change without changing it (`every_byte_of_the_terms_is_visible_in_the_text`).

Wallets do not agree on one way to sign a message, so the program verifies two, and the
filler says which the owner's wallet used:

| Signed as | Through | Bytes |
|---|---|---|
| text | `solana:signMessage` | the text alone |
| Offchain Message v1 | `solana:signOffchainMessage` | `"\xffsolana offchain"`, 1, 1, the owner's key, the text |

The text needs no envelope to be safe: it is ASCII that cannot be read as a
transaction, and it names the program, the cluster and the owner itself.

`INDEX` is the profile's `nonce_index` when the owner signed. `Fill` renders the text
with the profile's current one, so an intent signed before an `Invalidate` no longer
verifies.

An intent runs once. The only thing it leaves on chain is one bit, its nonce. The salt
is the nonce: an owner's nonces live in pages of 1,024, one bit for every salt, so
intents run in any order and none runs twice. A wallet hands out salts in sequence
(`nextSalt`), which fills one page before the next. A page belongs to one `nonce_index`;
once the profile has moved on, anyone closes it and its rent goes back to its payer.

An intent's limit is always a total, since it runs once. A named spender may take less
than the limit, as a merchant does when it captures less than it authorized. An intent
that anyone may fill and that asks nothing back runs for its whole amount or not at all:
otherwise a stranger could use it up for nothing.

`Cancel` sets the bit of one intent that has not run. `Invalidate` ends all of them.

A signature is not visible until it is used, and it stays valid until it runs, expires,
is cancelled or is invalidated. Anything meant to last belongs in a policy.

## Instructions

| # | Instruction | Signer | |
|---|---|---|---|
| 3 | `Open` | the owner | create its profile, once |
| 0 | `Create` | the owner | put a policy on chain, under the next number |
| 1 | `Pull` | the spender | take tokens under a policy, and deliver what the owner must receive |
| 10 | `Fill` | the spender | run a signed intent, once |
| 11 | `Cancel` | the owner | end one thing, by the account passed: close a policy, or use up one intent's nonce |
| 4 | `Invalidate` | the owner | end every policy so far, every intent so far, or both |
| 2 | `Close` | anyone | close what can never be used again, and return its rent |

| Account | PDA | Holds | Closes |
|---|---|---|---|
| Profile | `profile`, owner | `policies`, `stale`, `nonce_index` | never |
| Policy | `policy`, owner, number | the terms, how much of the limit is used, who paid the rent | the owner with `Cancel`; anyone with `Close` once it expired or was invalidated |
| Nonces | `nonces`, owner, `nonce_index`, page | 1,024 used-nonce bits, who paid the rent | anyone, once the profile's `nonce_index` has moved on |

A profile never closes: one opened again would count from zero and bring old policies
back. It costs about 0.0013 SOL, once.

A page of nonces costs about 0.0023 SOL, paid by whoever runs or cancels the first
intent in it, and returned when it closes.

Enabling a token account is an SPL `Approve` of the program's engine address, which is a
transaction. The client's `enableInstruction` approves without a cap, so the only limits
are the ones in the terms; it takes a cap as an option, and a cap that runs out takes
the delegate away with it. A token account has one delegate, so any other `Approve` on
it stops its policies and intents too. Either way the same `Approve` again restores
them as they were: nothing is created or signed anew.

Token-2022 mints work, transfer fees included: the owner never gives up more than the
limit, and never receives less than the terms say. A mint with a transfer hook does not:
the program passes the hook no accounts, so the transfer fails.

A pull that cannot move the tokens says why: `NotDelegate`, `AllowanceExceeded` or
`InsufficientFunds`. `fetchPolicyStatus` answers the same before anyone tries: whether
a policy is active, and how much one pull can take now. `fetchPolicies` lists the
policies an owner created or a spender may use.

`Pull` takes an optional 32-byte reference, the spender's own id for an invoice or an
order. The program does not store it; the event carries it.

## Cost

LiteSVM, the instruction alone, one run each:

| | CU |
|---|---|
| `Open` | 5.3k |
| `Create` | 5.6k |
| `Pull` | 4.4k |
| `Pull` that also delivers to the owner | 6.1k |
| `Close` | 2.3k |
| `Fill` | 60k |
| `Fill`, the first in a page of nonces | 65k |
| `Fill` that also delivers to the owner | 73k |
| `Cancel` | 1.9k |
| `Invalidate` | 1.9k |

Enabling, opening and creating the first policy in one transaction measured 12.5k on a
mainnet fork.

`Fill` pays for SHA-512 and Ed25519 in-program, and the cost grows with the text.
`Open`, `Create`, and the first `Fill` or `Cancel` in a page of nonces also pay a PDA
bump search, which varies by a few thousand with the address.

## Build

```sh
cargo build-sbf --manifest-path program/Cargo.toml --features localnet
cargo build-sbf --manifest-path tests/caller/Cargo.toml    # a test fixture that calls by CPI
cargo test --workspace

npm install
npm run generate -w @solana-pull/sdk    # idl.json and the generated client, from packages/sdk/idl.mjs
npm test -w @solana-pull/sdk            # the client against the Rust core, on 2,000 random terms
npm run e2e -w @solana-pull/sdk         # every journey, on a Surfpool fork of mainnet
```

## Client

[`packages/sdk`](packages/sdk) is one package for `@solana/kit`, in pure TypeScript.

- **Generated from the IDL** ([`idl.json`](packages/sdk/idl.json), written with Codama):
  every instruction, account, PDA, error and event, and the codec of the terms.
- **The journeys**: `onboardInstructions` (enable, open and create in one transaction),
  `pullInstruction`, `fillInstruction`, `cancelPolicyInstruction`,
  `cancelIntentInstruction`, `fetchPolicyStatus`, `fetchPolicies` and `parseEvents`.
- **The intent text**: `renderIntent`, `intentMessage`, and `parseIntent`, which reads a
  text back into its terms and refuses any text the program would not render itself.
- **For wallets**: `describeInstruction` and `describeIntent` turn a transaction or a
  message into the rows of an approval.

`npm test` checks the codec, the validity rules and the text against `pull-core`, both
ways. The scripts in [`packages/sdk/e2e`](packages/sdk/e2e) are the integration
examples: a subscription, a swap order, a payout signed in advance, and ending
everything at once.

## Wallets

A policy is created, cancelled and invalidated by ordinary transactions, so it works in
every wallet today. `describeInstruction` is what a wallet needs to show one well, and
`fetchPolicies` with `cancelPolicyInstruction` is a list of subscriptions with a button
to end each.

An intent is text, so a wallet needs no new method to sign one: it is asked to sign a
message, `describeIntent` recognises it by its first line, and the wallet shows what it
approves. [`packages/wallet-standard`](packages/wallet-standard) keeps
`solana:signIntent`, a feature where the wallet renders the text itself, as a proposal
for later.

Which wallet signs which form, from reading their shipping code in October 2026 and not
yet from testing on devices:

| Wallet | Text | Offchain Message v1 |
|---|---|---|
| Phantom | yes | no |
| Solflare | yes | yes |
| Backpack | yes | as bytes it is handed, not as a feature |
| Jupiter Wallet | not checked | yes |
| With a Ledger | no | only on app 1.16 or later |

A Ledger behind Phantom or Backpack signs a third, older form that the program does not
verify.

## Status

Not audited by a third party, and not deployed to mainnet. The program is upgradeable,
and its engine is the delegate of every enabled token account, so whoever holds the
upgrade authority can move all of them: before mainnet that key belongs to a multisig
with a timelock, or the program is made immutable. No wallet has yet been tested
signing an intent on a real device.

## Layout

| Path | Holds |
|---|---|
| [`program`](program) | The program: `Open`, `Create`, `Pull`, `Close`, `Fill`, `Cancel`, `Invalidate` |
| [`packages/pull-core`](packages/pull-core) | Terms, validity rules, the canonical text and the account layouts, shared by the program and every client |
| [`packages/sdk`](packages/sdk) | The client: generated from the IDL, plus the journeys, the intent text and what a wallet shows |
| [`packages/wallet-standard`](packages/wallet-standard) | `solana:signIntent`, a proposed Wallet Standard feature |
| [`tests`](tests) | LiteSVM flows, a randomized check of the limit against a reference model, and the canonical-text tests |
