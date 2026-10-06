# Solana Pull Program

A Solana program that lets a wallet owner permit someone to take tokens from its
account, within a limit, without giving up custody.

An owner states what may go out and, if it wants, what must come in. It can give that
permission in two ways:

- A **policy** is a standing permission on chain. The owner creates it with a
  transaction, its spender uses it again and again, and a wallet can list it and close it.
- An **intent** is one action the owner signs as text. Whoever it permits runs it once.
  Nothing goes on chain first, and the owner sends no transaction.

```text
Policy    Create  →  Pull, Pull, Pull …  →  Close
Intent    sign    →  Fill
```

`Pull` and `Fill` move tokens out of the owner's account as its SPL delegate, within the
limit. If the terms say what the owner must receive, the same instruction moves it from
the spender to the owner and checks what arrived. Funds stay in the owner's wallet until
then, and there is nothing between the two transfers to trust. Another program can call
`Pull`, for example to collect a payment and update its own state together.

| Use | As | Spender | Out | In |
|---|---|---|---|---|
| Subscription | policy | the merchant | 8 USDC every 30 days | nothing |
| Agent budget | policy | the API provider | 1 USDC every day | nothing |
| DCA | policy | anyone | 10 USDC every day | at least 0.05 SOL each time |
| Authorize, then capture | intent | the merchant | at most 120 USDC | nothing |
| One payment signed in advance | intent | the payee | 100 USDC | nothing |
| Swap, limit order, Dutch auction | intent | anyone | 100 USDC | at least 0.52 SOL, falling to 0.50 |

## Terms

A policy and an intent carry the same terms.

```text
Terms   { authority, spender: key | anyone, not_before, not_after?, salt, limit, receive? }
Limit   { from, mint, max, per: total | every(seconds) }
Receive { to, mint, min, decay?: (t0, t1, min) }
```

`limit` is what may go out: "at most `max` of `mint` may leave `from`", a token account
of the owner. A policy counts it in total, or over fixed windows that start at
`not_before`; what is unused in a window does not carry over. An intent runs once, so
its limit is the most that one use may take.

`receive` is what must come in: "each use, at least `min` of `mint` arrives in `to`", an
account of the owner. It is a total, not a rate: a spender that takes less still
delivers all of it. With a decay the minimum moves in a straight line between two times.

Terms are valid only if they bind someone: a named spender, or something the owner
receives. Terms that leave both open would pay whoever finds them.

## Policies

A policy is an account at a PDA of the owner and the hash of its terms. It holds the
terms, how much of the limit is used, and who paid its rent. Only a transaction signed
by the owner creates one, so everything a spender can do is on chain, where a wallet can
decode it, list it and close it.

Whoever pays the rent is always a separate account from whoever signs, so a merchant can
sponsor a subscriber. The owner or the spender can close a policy at any time, and anyone
can once it has expired. Closing is final, and the rent goes straight back to its payer.

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
SALT: 0
```

The text is an Offchain Message v1. `Fill` takes the terms as binary, renders them back
to this text, and verifies Ed25519 over it in-program. The text is all a wallet has to
show, and a byte of the terms cannot change without changing it
(`every_byte_of_the_terms_is_visible_in_the_text`).

An intent runs once. The only thing it leaves on chain is one bit, its nonce, and that
bit stays set forever, as an authorization's nonce does in ERC-3009. The salt is the
nonce: an owner's nonces live in pages of 1,024, one bit for every salt, so intents run
in any order and none ever runs twice. A wallet hands out salts in sequence
(`nextSalt`), which fills one page before the next.

`Cancel` sets the bit of an intent that has not run, or of every intent in its page at
once. An intent may expire but does not have to. With no expiry it does the job of a
durable nonce for a token movement: sign now, and the spender lands it whenever it is
due, with nothing set up first.

A signature is not visible until it is used, and it stays valid until it runs, expires
or is cancelled. Anything meant to last belongs in a policy.

## Instructions

| # | Instruction | Signer | |
|---|---|---|---|
| 0 | `Create` | the owner | put a policy on chain |
| 1 | `Pull` | the spender | take tokens under a policy, and deliver what the owner must receive |
| 2 | `Close` | the owner or the spender; anyone after the expiry | close a policy and return its rent |
| 10 | `Fill` | the spender | run a signed intent, once |
| 11 | `Cancel` | the owner | use up an intent's nonce, or a whole page of them |

| Account | Holds | Closes |
|---|---|---|
| Policy | the terms, how much of the limit is used, and who paid the rent | the owner or the spender, at any time; anyone after its expiry |
| Nonces | 1,024 used-nonce bits of one owner | never: a used nonce stays used |

A page of nonces costs about 0.002 SOL once, paid by whoever runs or cancels the first
intent in it.

Enabling a token account is an SPL `Approve` of the program's engine address, which is a
transaction. The SDK's `getEnableInstruction` approves without a cap, so the only limits
are the ones in the terms; it takes a cap as an option, and a cap that runs out takes
the delegate away with it. A token account has one delegate, so any other `Approve` on
it stops its policies and intents too. Either way the same `Approve` again restores
them as they were: nothing is created or signed anew.

A pull that cannot move the tokens says why: `NotDelegate`, `AllowanceExceeded` or
`InsufficientFunds`. `fetchSeat` reads the same for a token account before anyone
tries, and `fetchPolicies` lists the policies an owner created or a spender may use.

`Pull` takes an optional 32-byte reference, the spender's own id for an invoice or an
order. The program does not store it; the event carries it.

## Cost

LiteSVM, the instruction alone, one run each:

| | CU |
|---|---|
| `Pull` | 4.3k |
| `Pull` that also delivers to the owner | 5.8k |
| `Create` | 6k to 10k |
| `Close` | 2.2k |
| `Fill` | 63k |
| `Fill` that also delivers to the owner, with a decay | 86k |
| `Cancel` | 1.9k |

`Fill` pays for SHA-512 and Ed25519 in-program, and the cost grows with the text.
`Create`, and the first `Fill` or `Cancel` in a page of nonces, also pay a PDA bump
search, which adds a few thousand.

## Build

```sh
cargo build-sbf --manifest-path program/Cargo.toml --features localnet
cargo build-sbf --manifest-path tests/caller/Cargo.toml    # a test fixture that calls by CPI
cargo test --workspace

npm install
npm run dev                  # examples/agents on a Surfpool mainnet fork
npm run dev:subscriptions    # examples/subscriptions
```

## Layout

| Path | Holds |
|---|---|
| [`program`](program) | The program: `Create`, `Pull`, `Close`, `Fill`, `Cancel` |
| [`packages/pull-core`](packages/pull-core) | Terms, validity rules, the canonical text and the account layouts, shared by the program and every client |
| [`packages/sdk`](packages/sdk) | `@solana/kit` builders plus `pull-core` compiled to WebAssembly |
| [`packages/wallet-standard`](packages/wallet-standard) | `solana:signIntent`, the one feature a wallet adds to sign intents |
| [`examples`](examples) | The agent demo and the subscription site, both built on policies |
| [`tests`](tests) | LiteSVM flows, a randomized check of the limit against a reference model, and the canonical-text tests |
