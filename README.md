# Solana Pull Program

A Solana program that lets a wallet owner permit someone to move its tokens, within
limits, without giving up custody.

There are two things an owner can give, with the same terms:

- A **policy** is a standing permission on chain. The owner creates it with a
  transaction, and its spender uses it again and again.
- An **intent** is one action the owner signs as text. Whoever it permits runs it once.
  Nothing goes on chain first, and the owner sends no transaction.

```text
Policy    Create  →  Pull, Pull, Pull …  →  Close
Intent    sign    →  Fill
```

`Pull` and `Fill` move tokens out of the owner's account as its SPL delegate, inside
every limit. If the terms say what the owner must receive, the same instruction moves it
from the spender to the owner and checks what arrived. Funds stay in the owner's wallet
until then, and there is nothing between the two transfers to trust. Another program
can call `Pull`, for example to collect a payment and update its own state together.

| Use | As | Spender | Out | In |
|---|---|---|---|---|
| Subscription | policy | the merchant | 8 USDC every 30 days | nothing |
| Agent budget | policy | the agent's key | 5 per use, 12 a day, 100 in total | nothing |
| DCA | policy | anyone | 10 USDC every day | at least 0.05 SOL each time |
| Authorize, then capture | intent | the merchant | at most 120 USDC, before Friday | nothing |
| One payment signed in advance | intent | the payee | 100 USDC, before a date | nothing |
| Swap, limit order, Dutch auction | intent | anyone | 100 USDC | at least 0.52 SOL, falling to 0.50 |

## Terms

```text
Terms   { authority, spender: key | anyone, not_before, not_after?, salt, limits, receive? }
Limit   { from, mint, max, per: total | every(seconds) | use }
Receive { to, mint, min, decay?: (t0, t1, min) }
```

A limit is what may go out: "at most `max` of `mint` may leave `from`", counted over the
life of a policy, over fixed windows that start at `not_before`, or over a single pull.
Limits on one account stack: a pull must fit every one of them. What is unused in a
window does not carry over.

`receive` is what must come in: "each use, at least `min` of `mint` arrives in `to`", an
account of the owner. It is a total, not a rate: a spender that takes less still
delivers all of it. With a decay the minimum moves in a straight line between two times.

Terms are valid only if they bind someone: a named spender, or something the owner
receives. A per-use limit alone bounds nothing across pulls, so it needs a limit beside
it that persists.

## What an intent signs

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
bit stays set forever. The salt is the nonce: an owner's nonces live in pages of 1,024,
one bit for every salt, so intents run in any order and none ever runs twice. A wallet
hands out salts in sequence (`nextSalt`), which fills one page before the next.

`Cancel` sets the bit of an intent that has not run, or of every intent in its page at
once. An intent may expire, but does not have to: with no expiry it is a replacement for
a durable nonce when the action is a token movement. Sign now, and the spender lands it
whenever it is due, with nothing set up first.

## Instructions

| # | Instruction | Signer | |
|---|---|---|---|
| 0 | `Create` | the owner | put a policy on chain |
| 1 | `Pull` | the spender | take tokens under a policy, and deliver what the owner must receive |
| 2 | `Close` | the owner or the spender; anyone after the expiry | close a policy and return its rent |
| 10 | `Fill` | the spender | run a signed intent, once |
| 11 | `Cancel` | the owner | use up an intent's nonce, or a whole page of them |

Two account types. Whoever pays the rent is always a separate account from whoever
signs.

| Account | Holds | Closes |
|---|---|---|
| Policy | the terms, what each limit has consumed, and who paid the rent | the owner or the spender, at any time; anyone after its expiry |
| Nonces | 1,024 used-nonce bits of one owner | never: a used nonce stays used |

A policy is created only by a transaction, so closing it is final and the rent returns
at once. A page of nonces costs about 0.002 SOL once, paid by whoever runs or cancels the
first intent in it.

## Cost

LiteSVM, the instruction alone, one run each:

| | CU |
|---|---|
| `Pull` | 4.4k |
| `Pull` that also delivers to the owner | 6.0k |
| `Create` | 7k to 9k |
| `Close` | 2.1k |
| `Fill` | 65k |
| `Fill` that also delivers to the owner, with a decay | 82k |
| `Cancel` | 2.1k |

`Fill` pays for SHA-512 and Ed25519 in-program, and the cost grows with the text.
`Create`, and the first `Fill` or `Cancel` in a page of nonces, also pay a PDA bump
search, which varies by a few thousand.

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
| [`program`](program) | The engine |
| [`packages/pull-core`](packages/pull-core) | Terms, validity rules, the canonical text and the account layouts, shared by the program and every client |
| [`packages/sdk`](packages/sdk) | `@solana/kit` builders plus `pull-core` compiled to WebAssembly |
| [`packages/wallet-standard`](packages/wallet-standard) | `solana:signIntent`, the one feature a wallet adds to sign intents |
| [`examples`](examples) | The agent demo and the subscription site, both built on policies |
| [`tests`](tests) | LiteSVM flows, a randomized check of the limits against a reference model, and the canonical-text tests |

