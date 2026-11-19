# Solana Pull Program

A neutral debit authority for Solana token accounts. An owner states who may collect
from its account, how much, to which account and until when, and optionally what must
arrive in return. The program enforces it, and the owner never gives up custody.

This is how money already moves in banking: a payer authorizes a creditor once, under
stated limits, and every biller, payroll provider and merchant plugs into the same
scheme. A Solana token account has one delegate, so products that pull from it cannot
each bring their own authority. They need one shared, neutral one. This is meant to be
it: something subscriptions, payroll, agents and exchanges build on, not another
product beside them.

An owner gives its permission in two ways, with the same terms:

- A **mandate** is a standing permission on chain, as a direct debit mandate is. The
  owner creates it with a transaction, its spender collects under it again and again,
  and a wallet can list it and cancel it.
- An **authorization** is one payment the owner signs as text, as a cheque or a card
  authorization is. Whoever it permits runs it once. Nothing goes on chain first, and
  the owner sends no transaction.

```text
Once            Approve + Open
Mandate         Create  →  Pull, Pull, Pull …  →  Cancel
Authorization   sign    →  Fill
Undo            Cancel one mandate or one authorization · Invalidate all of either
```

| Here | In banking |
|---|---|
| Owner, spender, destination | Payer, creditor, beneficiary |
| Mandate with a limit per period | Direct debit mandate; variable recurring payment consent |
| Authorization to a named destination | Crossed cheque |
| Authorization to a named spender, "at most" | Card authorization and capture |
| What the owner must receive | Delivery versus payment |
| The reference on a pull | Payment reference |
| A named refusal (`InsufficientFunds`, `NotDelegate`) | Return reason code |
| `Cancel`, `Invalidate` | Stop one payment; cancel every mandate |

One thing banking has and this does not: there are no refunds or disputes. A pull that
the terms allow is final.

`Pull` and `Fill` move tokens out of the owner's account as its SPL delegate, within the
limit. If the terms say what the owner must receive, the same instruction moves it from
the spender to the owner and checks what arrived. Funds stay in the owner's wallet until
then, and there is nothing between the two transfers to trust. Another program can call
`Pull`, for example to collect a payment and update its own state together.

| Use | As | Spender | Out | In |
|---|---|---|---|---|
| Subscription | mandate | the merchant | 8 USDC every 30 days, only to the merchant's account | nothing |
| Agent budget | mandate | the API provider | 1 USDC every day | nothing |
| DCA | mandate | anyone | 10 USDC every day | at least 0.05 SOL each time |
| Authorize, then capture | authorization | the merchant | at most 120 USDC | nothing |
| One payment signed in advance | authorization | a relayer, or anyone | 100 USDC, only to the payee's account | nothing |
| Swap, limit order, Dutch auction | authorization | anyone | 100 USDC | at least 0.52 SOL, falling to 0.50 |

## Terms

A mandate and an authorization carry the same terms.

```text
Terms   { authority, spender: key | anyone, not_before, not_after?, salt, limit, receive? }
Limit   { from, mint, max, per: total | every(seconds), to? }
Receive { to, mint, min, decay?: (t0, t1, min) }
```

`limit` is what may go out: "at most `max` of `mint` may leave `from`", a token account
of the owner. With `to` it also says where: the tokens may go only to that token
account, so whoever holds the spender's key can deliver the money but not redirect it.
Without `to` the spender chooses.

A mandate counts the limit in total, or over fixed windows that start at `not_before`;
what is unused in a window does not carry over. An authorization runs once, so its limit is the
most that one use may take.

`receive` is what must come in: "each use, at least `min` of `mint` arrives in `to`", an
account of the owner. It is a total, not a rate: a spender that takes less still
delivers all of it. With a decay the minimum moves in a straight line between two times.

Terms are valid only if they bind something: a named spender, a named destination, or
something the owner receives. Terms that leave all three open would pay whoever finds
them.

`salt` is an authorization's nonce. A mandate does not use it.

## Profile

Each owner has one profile, opened in the transaction that enables its first token
account. It holds three numbers:

```text
Profile { mandates, stale, nonce_index }
```

The owner's mandates are numbered 1, 2, 3 as they are created, and `mandates` is the
last number used. Every mandate numbered `stale` or less is invalid. `nonce_index` is
what the owner's authorizations are signed under.

`Invalidate` ends everything of one kind in one transaction.

- For mandates it uses up one more number and sets `stale` to it. Mandates created
  afterwards work as usual. A `Create` that was signed earlier and held back names the
  address of the number that was used up, so it can no longer land.
- For authorizations it moves `nonce_index` by an amount taken from a slot hash, so
  nobody can hold a signature made ahead for the next value.

The two are separate: ending every authorization does not end a subscription.

Whatever was ended can be closed by anyone, and its rent goes back to whoever paid it.

## Mandates

A mandate is an account at a PDA of the owner and its number. It holds the terms, how
much of the limit is used, and who paid its rent. Only a transaction signed by the owner
creates one, so everything a spender can do is on chain. A wallet lists an owner's
mandates by their numbers, with no scan of the program's accounts.

Whoever pays the rent is always a separate account from whoever signs, so a merchant can
sponsor a subscriber. The owner ends a mandate at any time with `Cancel`, which closes it
at once. Anyone closes one that has expired or that the owner invalidated. Either way it
is gone for good, and the rent goes straight back to its payer.

## Authorizations

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
with the profile's current one, so an authorization signed before an `Invalidate` no longer
verifies.

An authorization runs once. The only thing it leaves on chain is one bit, its nonce. The salt
is the nonce: an owner's nonces live in pages of 1,024, one bit for every salt, so
authorizations run in any order and none runs twice. A wallet hands out salts in sequence
(`nextSalt`), which fills one page before the next. A page belongs to one `nonce_index`;
once the profile has moved on, anyone closes it and its rent goes back to its payer.

An authorization's limit is always a total, since it runs once. A named spender may take less
than the limit, as a merchant does when it captures less than it authorized. An authorization
that anyone may fill and that asks nothing back runs for its whole amount or not at all:
otherwise a stranger could use it up for nothing.

`Cancel` sets the bit of one authorization that has not run. `Invalidate` ends all of them.

A signature is not visible until it is used, and it stays valid until it runs, expires,
is cancelled or is invalidated. Anything meant to last belongs in a mandate.

## Instructions

| # | Instruction | Signer | |
|---|---|---|---|
| 3 | `Open` | the owner | create its profile, once |
| 0 | `Create` | the owner | put a mandate on chain, under the next number |
| 1 | `Pull` | the spender | take tokens under a mandate, and deliver what the owner must receive |
| 10 | `Fill` | the spender | run a signed authorization, once |
| 11 | `Cancel` | the owner | end one thing, by the account passed: close a mandate, or use up one authorization's nonce |
| 4 | `Invalidate` | the owner | end every mandate so far, every authorization so far, or both |
| 2 | `Close` | anyone | close what can never be used again, and return its rent |

| Account | PDA | Holds | Closes |
|---|---|---|---|
| Profile | `profile`, owner | `mandates`, `stale`, `nonce_index` | never |
| Mandate | `mandate`, owner, number | the terms, how much of the limit is used, who paid the rent | the owner with `Cancel`; anyone with `Close` once it expired or was invalidated |
| Nonces | `nonces`, owner, `nonce_index`, page | 1,024 used-nonce bits, who paid the rent | anyone, once the profile's `nonce_index` has moved on |

A profile never closes: one opened again would count from zero and bring old mandates
back. It costs about 0.0013 SOL, once.

A page of nonces costs about 0.0023 SOL, paid by whoever runs or cancels the first
authorization in it, and returned when it closes.

Enabling a token account is an SPL `Approve` of the program's engine address, which is a
transaction. The client's `enableInstruction` approves without a cap, so the only limits
are the ones in the terms; it takes a cap as an option, and a cap that runs out takes
the delegate away with it. A token account has one delegate, so any other `Approve` on
it stops its mandates and authorizations too. Either way the same `Approve` again restores
them as they were: nothing is created or signed anew.

Token-2022 mints work, transfer fees included: the owner never gives up more than the
limit, and never receives less than the terms say. A mint with a transfer hook does not:
the program passes the hook no accounts, so the transfer fails.

A pull that cannot move the tokens says why: `NotDelegate`, `AllowanceExceeded` or
`InsufficientFunds`. `fetchMandateStatus` answers the same before anyone tries: whether
a mandate is active, and how much one pull can take now. `fetchMandates` lists the
mandates an owner created or a spender may use.

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

Enabling, opening and creating the first mandate in one transaction measured 12.5k on a
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
Each party has one call for each thing it does.

| Who | Does | Calls |
|---|---|---|
| A site | Starts a mandate: enable, open the profile, create, in one transaction | `onboardInstructions` |
| A site | Asks the owner's wallet to sign an authorization, in whichever way that wallet signs | `requestAuthorization` |
| A merchant | Asks whether it can collect, and how much | `fetchMandateStatus` |
| A merchant | Collects, with its own reference | `pullInstruction` |
| A merchant | Reads what happened | `parseEvents` |
| A solver | Asks whether it can fill, and how much | `fetchAuthorizationStatus` |
| A solver | Fills | `fillInstruction` |
| An owner | Lists, cancels one, ends all | `fetchMandates`, `cancelMandateInstruction`, `cancelAuthorizationInstruction`, `getInvalidateInstructionAsync` |
| Anyone | Closes what was ended, for its payer | `closeMandateInstruction` |

Underneath, every instruction, account, PDA, error and event, and the codec of the
terms, are generated from the IDL ([`idl.json`](packages/sdk/idl.json), written with
Codama). An authorization travels as bytes (`encodeAuthorization`).

`npm test` checks the codec, the validity rules and the text against `pull-core`, both
ways. The scripts in [`packages/sdk/e2e`](packages/sdk/e2e) are the integration
examples: a subscription, a swap order, a payout signed in advance, and ending
everything at once.

## Wallets

A mandate is created, cancelled and invalidated by ordinary transactions, so it works in
every wallet today. Three calls make it work well:

- `describeInstruction` turns each instruction of a transaction into the rows of an
  approval: who can take what, where it goes, and until when.
- `fetchMandates` with `cancelMandateInstruction` is a list of an owner's mandates with
  a button to end each, and `Invalidate` is the button to end them all.
- A wallet should warn before any other `Approve` on an enabled token account: it
  takes the delegate, and every mandate on the account stops until it is enabled again.

An authorization is text, so a wallet needs no new method to sign one. When it is asked
to sign a message, `reviewAuthorization` says whether the message is an authorization,
what it approves, and whether to refuse it: it is for another account, it shows
amounts with decimals the mint does not have, or it is written for a nonce index that
is not the account's. Any text that is not exactly what the program would render is not
an authorization. [`packages/wallet-standard`](packages/wallet-standard) keeps
`solana:signAuthorization`, a feature where the wallet renders the text itself, as a
proposal for later.

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
verify. `requestAuthorization` picks the form the wallet has, and refuses a signature
the program would not accept before anyone pays for a transaction.

## Status

Not audited by a third party, and not deployed to mainnet. No wallet has yet been
tested signing an authorization on a real device.

The program is meant to be a neutral utility, not ours to run. Its engine is the
delegate of every enabled token account, so whoever holds its upgrade authority can
move all of them. Before mainnet that authority goes to a neutral body or the program
is made immutable.

## Layout

| Path | Holds |
|---|---|
| [`program`](program) | The program: `Open`, `Create`, `Pull`, `Close`, `Fill`, `Cancel`, `Invalidate` |
| [`packages/pull-core`](packages/pull-core) | Terms, validity rules, the canonical text and the account layouts, shared by the program and every client |
| [`packages/sdk`](packages/sdk) | The client: generated from the IDL, plus the journeys, the authorization text and what a wallet shows |
| [`packages/wallet-standard`](packages/wallet-standard) | `solana:signAuthorization`, a proposed Wallet Standard feature |
| [`tests`](tests) | LiteSVM flows, a randomized check of the limit against a reference model, and the canonical-text tests |
