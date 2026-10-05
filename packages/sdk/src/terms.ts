import { type Address, getAddressEncoder } from '@solana/kit';

import init, { decodeTerms, encodeTerms, errorName, renderText } from '../wasm/pull.js';

/**
 * Terms as JSON. Encoding, validation and the canonical text all
 * come from `pull-core` compiled to WebAssembly, the same code the program
 * runs: this package never reimplements them. Integers that can pass 2^53 are
 * strings.
 */
export type Terms = {
    authority: Address;
    /** Who may pull. `null` is anyone, which is only valid if the authority receives something. */
    spender: Address | null;
    notBefore: number;
    /** `null` runs until closed or cancelled. */
    notAfter: number | null;
    /** Tells apart otherwise identical terms. For a signed intent it is also the nonce. */
    salt: string;
    limit: Limit;
    receive: Receive | null;
};

/**
 * What may go out: at most `max` of `mint` may leave `from`, a token account
 * of the authority, in total or in every fixed window of N seconds counted
 * from `notBefore`. For an intent, which runs once, it is the most that one
 * use may take.
 */
export type Limit = {
    from: Address;
    mint: Address;
    max: string;
    per: 'total' | { every: number };
};

/**
 * Each use, the spender pays at least `min` of `mint` into `to`, a token
 * account of the authority, inside the pull, whatever amount it takes. With
 * `decay` the minimum moves in a straight line to `decay.min` between `t0`
 * and `t1`.
 */
export type Receive = {
    to: Address;
    mint: Address;
    min: string;
    decay: { t0: number; t1: number; min: string } | null;
};

let loaded: Promise<unknown> | undefined;

/** Load the WebAssembly once. Browsers fetch it; Node passes the bytes. */
export function loadWasm(module?: BufferSource) {
    loaded ??= init(module ? { module_or_path: module } : undefined);
    return loaded;
}

/** Canonical bytes: what a policy holds and a signature covers. Throws on invalid terms. */
export const encode = (terms: Terms): Uint8Array => encodeTerms(JSON.stringify(terms));

export const decode = (bytes: Uint8Array): Terms => JSON.parse(decodeTerms(bytes));

/** The text a wallet shows. `decimals` maps each mint in the terms to its decimals. */
export const text = (bytes: Uint8Array, decimals: Record<string, number>): string =>
    renderText(bytes, JSON.stringify(decimals));

/** The name of a Pull program error code (`Custom(code)` in a failed transaction). */
export const programError = (code: number): string | undefined => errorName(code);

/**
 * The exact bytes a wallet signs for signed terms: the Offchain Message v1
 * envelope (domain, version 1, one signer: the authority) and the canonical
 * text. `solana:signOffchainMessage` builds the same bytes from `text()`.
 */
export function message(bytes: Uint8Array, decimals: Record<string, number>): Uint8Array {
    const body = new TextEncoder().encode(text(bytes, decimals));
    const authority = getAddressEncoder().encode(decode(bytes).authority);
    return Uint8Array.from([0xff, ...new TextEncoder().encode('solana offchain'), 1, 1, ...authority, ...body]);
}

/** `terms_id`: sha256 of the canonical bytes. */
export async function termsId(bytes: Uint8Array): Promise<Uint8Array> {
    return new Uint8Array(await crypto.subtle.digest('SHA-256', bytes as BufferSource));
}

/** A random salt, for a policy: a new policy for terms that are otherwise the same. For an intent use `nextSalt`. */
export const randomSalt = (): string => crypto.getRandomValues(new BigUint64Array(1))[0].toString();

/**
 * A subscription: only `merchant` may take, at most `amount` in every period,
 * from the subscriber's account, until it is closed or `end` passes.
 */
export function subscriptionTerms(p: {
    subscriber: Address;
    account: Address;
    mint: Address;
    amount: bigint;
    period: number;
    merchant: Address;
    start: number;
    end?: number;
}): Terms {
    return {
        authority: p.subscriber,
        limit: { from: p.account, max: p.amount.toString(), mint: p.mint, per: { every: p.period } },
        notAfter: p.end ?? null,
        notBefore: p.start,
        receive: null,
        salt: randomSalt(),
        spender: p.merchant,
    };
}
