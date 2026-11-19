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

The owner's permission is a **mandate**: a standing permission on chain, as a direct
debit mandate is. The owner creates it with a transaction, its spender collects under
it again and again, and a wallet can list it and end it.

```text
Once      Approve + Open
Mandate   Create  →  Pull, Pull, Pull …  →  Close
Undo      Close one mandate · Invalidate all of them
```

| Here | In banking |
|---|---|
| Owner, spender, destination | Payer, creditor, beneficiary |
| Mandate with a limit per period | Direct debit mandate; variable recurring payment consent |
| Mandate to a named destination | Crossed cheque |
| What must come back | Delivery versus payment |
| The reference on a pull | Payment reference |
| A named refusal (`InsufficientFunds`, `NotDelegate`) | Return reason code |
| `Close`, `Invalidate` | Stop one mandate; cancel every mandate |

One thing banking has and this does not: there are no refunds or disputes. A pull that
the terms allow is final.

`Pull` moves tokens out of the owner's account as its SPL delegate, within the limit. If
the terms say what must come back, the same instruction moves it from the
spender to the owner and checks what arrived. Funds stay in the owner's wallet until
then, and there is nothing between the two transfers to trust. Another program can call
`Pull`, for example to collect a payment and update its own state together.

| Use | Spender | Out | In |
|---|---|---|---|
| Subscription | the merchant | 8 USDC every 30 days, only to the merchant's account | nothing |
| Agent budget | the API provider | 1 USDC every day | nothing |
| DCA | anyone | 10 USDC every day | at least 0.05 SOL each time |
| Payout set up in advance | a relayer, or anyone | 100 USDC in total, only to the payee's account | nothing |
| Swap, limit order | anyone | 100 USDC in total | at least 0.52 SOL |

## Terms

```text
Terms { spender?, not_before, not_after?, out, in? }
Out   { from, to?, max, period? }
In    { to, min }
```

The authority is not part of the terms. It is whoever signs `Create`, and the mandate
records it.

`out` is what may go out: "at most `max` may leave `from`", a token account of the
owner, in its mint. With `to` it also says where: the tokens may go only to that token
account, so whoever holds the spender's key can deliver the money but not redirect it.
Without `to` the spender chooses.

The limit counts over fixed windows of `period` seconds that start at `not_before`, and
what is unused in a window does not carry over. With no period it counts in total.

`in` is what must come back: "each pull, at least `min` arrives in `to`", an account of
the owner, in its mint. It is a total, not a rate: a spender that takes less still
delivers all of it. `out.to` is where the owner's tokens go; `in.to` is where the
payment for them lands.

The terms name token accounts, not mints. A token account's mint never changes, and
`Pull` checks that the mints it is given are the ones of `out.from` and `in.to`.

Terms are valid only if they bind something: a named spender, a named destination, or
something that comes back. Terms that leave all three open would pay whoever finds
them. No spender means anyone may pull.

## Profile

Each owner has one profile, opened in the transaction that enables its first token
account. It holds two numbers:

```text
Profile { mandates, stale }
```

The owner's mandates are numbered 1, 2, 3 as they are created, and `mandates` is the
last number used. Every mandate numbered `stale` or less is invalid.

`Invalidate` ends every mandate so far in one transaction. It uses up one more number
and sets `stale` to it. Mandates created afterwards work as usual. A `Create` that was
signed earlier and held back names the address of the number that was used up, so it
can no longer land.

## Mandates

A mandate is an account at a PDA of the owner and its number. Only a transaction signed
by the owner creates one, so everything a spender can do is on chain. A wallet lists an
owner's mandates by their numbers, with no scan of the program's accounts.

```text
Mandate   PDA ["mandate", authority, index]                    213 bytes, 253 with `in`
  header  tag · authority · payer · index                      who, written once
  usage   consumed · window                                    the only bytes Pull writes
  terms   spender · not_before · not_after · out · in?         what, written once
```

Every field sits at a fixed offset: the spender at 89, `out.from` at 137, `out.to` at
169. An absent value is all zeroes, never a valid present one: the zero key is the
system program, which is no token account and never signs, and an expiry at 0 or a
period of zero seconds is invalid. `in` is a suffix, present only when the terms have
one.

`authority` is checked against the owner of `out.from` on every pull. A mandate belongs
to the owner that signed it, not to the token account: when an account changes owner,
the token program clears its delegate, and the new owner enabling it again does not
bring the old owner's mandates back. `window` is the window of the limit that
`consumed` counts: windows of the period from `not_before`, or always 0 for a limit in
total.

Whoever pays the rent is always a separate account from whoever signs, so a merchant can
sponsor a subscriber. The owner ends a mandate at any time with `Close`. Anyone closes
one that can never pull again: it expired, the owner invalidated it, or its limit in
total is used up. Either way it is gone for good, and the rent goes straight back to its
payer.

A one-off payment is a mandate whose limit counts in total. Once it is used up, anyone
can close it.

A mandate costs about 0.0024 SOL of rent, 0.0027 SOL with `in`, returned when it closes.

## Instructions

| # | Instruction | Signer | |
|---|---|---|---|
| 3 | `Open` | the owner | create its profile, once |
| 0 | `Create` | the owner | put a mandate on chain, under the next number |
| 1 | `Pull` | the spender | take tokens under a mandate, and deliver what must come back |
| 2 | `Close` | the owner, or anyone once it can never pull again | end a mandate and return its rent |
| 4 | `Invalidate` | the owner | end every mandate so far |

| Account | PDA | Holds | Closes |
|---|---|---|---|
| Profile | `profile`, owner | `mandates`, `stale` | never |
| Mandate | `mandate`, owner, number | the terms, how much of the limit is used, who paid the rent | the owner at any time; anyone once it expired, was invalidated or is used up |

A profile never closes: one opened again would count from zero and bring old mandates
back. It costs about 0.0012 SOL, once.

Enabling a token account is an SPL `Approve` of the program's engine address, which is a
transaction. The client's `enableInstruction` approves without a cap, so the only limits
are the ones in the terms; it takes a cap as an option, and a cap that runs out takes
the delegate away with it. A token account has one delegate, so any other `Approve` on
it stops its mandates too. Either way the same `Approve` again restores them as they
were: nothing is created anew.

Token-2022 mints work, transfer fees included: the owner never gives up more than the
limit, and never receives less than the terms say. A mint with a transfer hook does not:
the program passes the hook no accounts, so the transfer fails.

A pull must take something (`ZeroAmount`). One that cannot move the tokens says why:
`NotDelegate`, `AllowanceExceeded` or `InsufficientFunds`. `fetchMandateStatus` answers the same before anyone tries: whether
a mandate is active, and how much one pull can take now. `fetchMandates` lists the
mandates an owner created or a spender may use.

`Pull` takes an optional 32-byte reference, the spender's own id for an invoice or an
order. The program does not store it; the event carries it.

## Cost

LiteSVM, the instruction alone, one run each:

| | CU |
|---|---|
| `Open` | 5.1k |
| `Create` | 5.5k |
| `Pull` | 4.0k |
| `Pull` that also delivers to the owner | 5.5k |
| `Close` | 2.0k |
| `Invalidate` | 1.7k |

Enabling, opening and creating the first mandate in one transaction measured 12.2k on a
mainnet fork.

`Open` and `Create` pay a PDA bump search, which varies with the address: `Create`
measured from 5.5k to 13k. The search is what makes each address canonical, so a wallet
that lists an owner's mandates by number finds every one. The program is 42 KB.

## Build

```sh
cargo build-sbf --manifest-path program/Cargo.toml
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
| A merchant | Asks whether it can collect, and how much | `fetchMandateStatus` |
| A merchant | Collects, with its own reference | `pullInstruction` |
| A merchant | Reads what happened | `parseEvents` |
| An owner | Lists, ends one, ends all | `fetchMandates`, `closeMandateInstruction`, `getInvalidateInstructionAsync` |
| Anyone | Closes what was ended, for its payer | `closeMandateInstruction` |

Underneath, every instruction, account, PDA, error and event, and the codec of the
terms, are generated from the IDL ([`idl.json`](packages/sdk/idl.json), written with
Codama).

`npm test` checks the codec and the validity rules against `pull-core`. The scripts in
[`packages/sdk/e2e`](packages/sdk/e2e) are the integration examples: a subscription, a
swap order, a payout set up in advance, and ending everything at once.

## Wallets

A mandate is created, closed and invalidated by ordinary transactions, so it works in
every wallet today. Three calls make it work well:

- `describeInstruction` turns each instruction of a transaction into the rows of an
  approval: who can take what, where it goes, and until when. Amounts are shown with
  the decimals and symbol the wallet knows for each of the owner's token accounts.
- `fetchMandates` with `closeMandateInstruction` is a list of an owner's mandates with
  a button to end each, and `Invalidate` is the button to end them all.
- A wallet should warn before any other `Approve` on an enabled token account: it
  takes the delegate, and every mandate on the account stops until it is enabled again.

## Status

Not audited by a third party, and not deployed to mainnet.

The program is meant to be a neutral utility, not ours to run. Its engine is the
delegate of every enabled token account, so whoever holds its upgrade authority can
move all of them. Before mainnet that authority goes to a neutral body or the program
is made immutable.

## Layout

| Path | Holds |
|---|---|
| [`program`](program) | The program: `Open`, `Create`, `Pull`, `Close`, `Invalidate` |
| [`packages/pull-core`](packages/pull-core) | Terms, validity rules and the account layouts, shared by the program and every client |
| [`packages/sdk`](packages/sdk) | The client: generated from the IDL, plus the journeys and what a wallet shows |
| [`tests`](tests) | LiteSVM flows, a randomized check of the limit against a reference model, and the terms codec |
