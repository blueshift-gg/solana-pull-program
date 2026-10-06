# `solana:signIntent`

> A proposal for later. Today an intent is signed as a message a wallet already knows
> how to sign, and a wallet recognises it with `describeIntent` from `@solana-pull/sdk`.

The Wallet Standard feature a wallet implements to sign intents for the Pull program.
It's written the way it would land in `@solana/wallet-standard-features`, following the
shape of `solana:signOffchainMessage`, but it lives in this repo.

## Summary

Adds `solana:signIntent`. A dapp passes canonical terms, never text. The wallet decodes
and validates them, reads mint decimals and the account's profile from its own RPC,
renders the canonical text itself under the profile's nonce index, shows it, and signs
it as an Offchain Message v1.

The output type is `SolanaSignOffchainMessageOutput`, byte for byte. Verifiers and the
on-chain program need nothing new.

```ts
const [{ signature }] = await wallet.features['solana:signIntent'].signIntent({ account, terms });
```

## Why a feature, when `solana:signOffchainMessage` can already sign the text

1. **The wallet renders; the dapp doesn't.** With `signOffchainMessage` the dapp supplies
   the text. The program still rejects any text that isn't the canonical rendering, so
   nothing unsafe gets through. But only a wallet that renders from bytes can show a
   structured approval it computed itself, including decimals it read from chain.
2. **Capability detection.** A dapp can tell whether the wallet understands intents and
   pick the right path:

| Wallet supports | Dapp does |
|---|---|
| `solana:signIntent` | sends terms; the wallet shows an approval sheet |
| only `solana:signOffchainMessage` (v1) | renders the canonical text itself; the wallet shows it raw |

Either way the spender fills the same intent, and the program enforces it the same way.

## Wallet requirements

- Reject terms that don't decode as canonical, valid terms for the account's cluster.
- Reject when `account` is not the terms' authority.
- Reject when the account has no profile: an intent is signed under its nonce index.
- Render with the canonical renderer (`pull-core`) and sign
  `"\xffsolana offchain" ‖ 0x01 ‖ 0x01 ‖ authority ‖ text`.
- Show the `SPENDER`, the `MAY TAKE` line and the `MUST RECEIVE` line.
- For terms that are valid `until revoked`, ask for a separate, explicit confirmation:
  the intent stays usable until it is filled, cancelled or invalidated.

The enabling `Approve`, `Open`, `Invalidate`, and a policy's `Create` and `Close` are
ordinary transactions, so they need no feature.

```sh
npm install && npm run typecheck
```
