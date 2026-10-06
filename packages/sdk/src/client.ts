// What an integration calls: the journeys, built on the generated instructions.
import {
    type Account,
    type AccountMeta,
    type Address,
    AccountRole,
    address,
    type Base58EncodedBytes,
    getAddressDecoder,
    getBase58Encoder,
    getBase64Encoder,
    getU64Encoder,
    type Instruction,
    type Option,
    type ReadonlyUint8Array,
    type Rpc,
    type SolanaRpcApi,
    type TransactionSigner,
} from '@solana/kit';

import {
    decodePolicy,
    ENGINE_ADDRESS,
    type Envelope,
    fetchAllMaybePolicy,
    fetchMaybeNonces,
    fetchMaybePolicy,
    fetchMaybeProfile,
    findNoncesPda,
    findPolicyPda,
    findProfilePda,
    getCancelInstructionAsync,
    getCloseInstruction,
    getCreateInstructionAsync,
    getFillInstruction,
    getOpenInstructionAsync,
    getPullInstruction,
    getTermsCodec,
    identifySolanaPullEvent,
    parseCancelledEvent,
    parseClosedEvent,
    parseCreatedEvent,
    parseFilledEvent,
    parseInvalidatedEvent,
    parseOpenedEvent,
    parsePulledEvent,
    type Policy,
    SOLANA_PULL_PROGRAM_ADDRESS,
    SolanaPullEvent,
    type Terms,
    type TermsArgs,
} from './generated/index.ts';

type Client = Rpc<SolanaRpcApi>;
const TOKEN_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
const CLOCK = address('SysvarC1ock11111111111111111111111111111111');
/** Nonces in one page. */
export const NONCE_BITS = 1024n;

const some = <T>(o: Option<T>): T | null => (o.__option === 'Some' ? o.value : null);
const view = (data: ReadonlyUint8Array) => new DataView(data.buffer, data.byteOffset, data.byteLength);
const pda = async (found: Promise<readonly [Address, number]>) => (await found)[0];
/** Terms as an integration writes them, in the shape the accounts hold. */
export const terms = (args: TermsArgs): Terms => getTermsCodec().decode(getTermsCodec().encode(args));

/**
 * What a token account allows the program: its balance, and whether the
 * engine is its delegate and for how much. Not `enabled` means it was never
 * enabled, another `Approve` took its place, or a capped approval ran out;
 * `enableInstruction` restores it, and every policy works again as it was.
 */
function seat(data: ReadonlyUint8Array) {
    // mint, owner, amount: u64, delegate: COption<Pubkey> at 72, ..., delegated_amount: u64 at 121
    const enabled = view(data).getUint32(72, true) === 1 && getAddressDecoder().decode(data.subarray(76, 108)) === ENGINE_ADDRESS;
    return { allowance: enabled ? view(data).getBigUint64(121, true) : 0n, balance: view(data).getBigUint64(64, true), enabled };
}

const accounts = async (rpc: Client, addresses: Address[]) => {
    const { value } = await rpc.getMultipleAccounts(addresses, { encoding: 'base64' }).send();
    return value.map((account) => (account ? new Uint8Array(getBase64Encoder().encode(account.data[0])) : null));
};

export async function fetchSeat(rpc: Client, account: Address) {
    const [data] = await accounts(rpc, [account]);
    return data ? seat(data) : null;
}

/**
 * Enable a token account: SPL `Approve` the engine as its delegate. `amount`
 * caps what every policy and intent on the account can take in total; leave
 * it out for no cap beyond their own limits.
 */
export const enableInstruction = (p: { owner: TransactionSigner; account: Address; amount?: bigint; tokenProgram?: Address }): Instruction => ({
    accounts: [
        { address: p.account, role: AccountRole.WRITABLE },
        { address: ENGINE_ADDRESS, role: AccountRole.READONLY },
        { address: p.owner.address, role: AccountRole.READONLY_SIGNER, signer: p.owner } as AccountMeta,
    ],
    data: Uint8Array.of(4, ...getU64Encoder().encode(p.amount ?? 2n ** 64n - 1n)),
    programAddress: p.tokenProgram ?? TOKEN_PROGRAM,
});

/**
 * Everything an authority signs to start a policy, as one transaction: enable
 * the token account if the engine is not its delegate, open the profile if
 * there is none, and create the policy under the next number. Returns where
 * the policy will be.
 */
export async function onboardInstructions(rpc: Client, p: { authority: TransactionSigner; payer: TransactionSigner; terms: TermsArgs; tokenProgram?: Address }) {
    const profileAddress = await pda(findProfilePda({ authority: p.authority.address }));
    const [[from], profile] = await Promise.all([accounts(rpc, [p.terms.limit.from]), fetchMaybeProfile(rpc, profileAddress)]);
    if (!from) throw new Error(`token account ${p.terms.limit.from} does not exist`);
    const index = (profile.exists ? profile.data.policies : 0n) + 1n;
    const policy = await pda(findPolicyPda({ authority: p.authority.address, index }));
    const instructions: Instruction[] = [
        ...(seat(from).enabled ? [] : [enableInstruction({ account: p.terms.limit.from, owner: p.authority, tokenProgram: p.tokenProgram })]),
        ...(profile.exists ? [] : [await getOpenInstructionAsync({ authority: p.authority, payer: p.payer })]),
        await getCreateInstructionAsync({ authority: p.authority, payer: p.payer, policy, terms: p.terms }),
    ];
    return { index, instructions, policy };
}

/** The accounts a pull moves tokens between, taken from the terms. */
async function legs(t: Terms, p: { to?: Address; payFrom?: Address; tokenProgram?: Address; payProgram?: Address }) {
    const [to, receive] = [some(t.limit.to) ?? p.to, some(t.receive)];
    if (!to) throw new Error('the terms do not name where the tokens go: pass `to`');
    if (receive && !p.payFrom) throw new Error('the authority must receive something: pass `payFrom`');
    return {
        from: t.limit.from,
        mint: t.limit.mint,
        profile: await pda(findProfilePda({ authority: t.authority })),
        to,
        tokenProgram: p.tokenProgram,
        ...(receive ? { payFrom: p.payFrom, payMint: receive.mint, payProgram: p.payProgram ?? TOKEN_PROGRAM, payTo: receive.to } : {}),
    };
}

type Legs = { to?: Address; payFrom?: Address; tokenProgram?: Address; payProgram?: Address };

/**
 * Take `amount` under a policy (`fetchPolicyStatus` or `fetchPolicies` give
 * it). `to` is where the tokens go unless the terms name it; `payFrom` is the
 * spender's account that pays what the authority must receive; `reference`
 * is the spender's own 32-byte id for this pull, which the event carries.
 */
export const pullInstruction = async (p: Legs & { spender: TransactionSigner; policy: Account<Policy>; amount: bigint; reference?: Uint8Array }) =>
    getPullInstruction({ ...(await legs(p.policy.data.terms, p)), amount: p.amount, policy: p.policy.address, reference: p.reference ?? null, spender: p.spender });

/**
 * Run a signed intent, once: `terms` the authority signed as text under
 * `nonceIndex`, with the `envelope` its wallet used (`intentMessage`).
 * `payer` funds the page of nonces if this intent is the first in it, and
 * gets it back when the page closes.
 */
export const fillInstruction = async (
    p: Legs & { spender: TransactionSigner; payer: TransactionSigner; terms: Terms; nonceIndex: bigint; envelope: Envelope; signature: Uint8Array; amount: bigint },
) =>
    getFillInstruction({
        ...(await legs(p.terms, p)),
        amount: p.amount,
        envelope: p.envelope,
        nonces: await pda(findNoncesPda({ authority: p.terms.authority, nonceIndex: p.nonceIndex, page: p.terms.salt / NONCE_BITS })),
        payer: p.payer,
        signature: p.signature,
        spender: p.spender,
        terms: p.terms,
    });

/** The authority ends one policy. It closes at once, and its rent goes back to whoever paid it. */
export const cancelPolicyInstruction = (p: { authority: TransactionSigner; policy: Account<Policy> }) =>
    getCancelInstructionAsync({ account: p.policy.address, authority: p.authority, payer: p.policy.data.payer, salt: null });

/** The authority uses up the nonce of one intent it signed under `nonceIndex`, so it can never be filled. */
export const cancelIntentInstruction = async (p: { authority: TransactionSigner; payer: TransactionSigner; nonceIndex: bigint; salt: bigint }) =>
    getCancelInstructionAsync({
        account: await pda(findNoncesPda({ authority: p.authority.address, nonceIndex: p.nonceIndex, page: p.salt / NONCE_BITS })),
        authority: p.authority,
        payer: p.payer,
        salt: p.salt,
    });

/**
 * Anyone closes a policy that can never be used again (expired, or
 * invalidated by its authority), and its rent goes back to whoever paid it.
 */
export const closePolicyInstruction = async (p: { closer: TransactionSigner; policy: Account<Policy> }) =>
    getCloseInstruction({ account: p.policy.address, closer: p.closer, payer: p.policy.data.payer, profile: await pda(findProfilePda({ authority: p.policy.data.terms.authority })) });

/** The state of a policy, as a spender needs it before it pulls. */
export type PolicyState =
    /** In force. `available` is what one pull can take now. */
    | 'active'
    /** Not on chain: never created, cancelled, or closed. */
    | 'missing'
    /** Its authority ended every policy up to this one. */
    | 'invalidated'
    | 'notStarted'
    | 'expired'
    /** The engine is not the delegate of the token account: the authority must enable it again. */
    | 'notDelegate';

/**
 * Whether a pull under `policy` can go through now, and for how much: the
 * least of what is left of the limit in this window, what is left of the
 * token account's approval, and its balance. One answer from the policy, its
 * authority's profile, the token account and the chain's clock.
 */
export async function fetchPolicyStatus(rpc: Client, policy: Address) {
    const found = await fetchMaybePolicy(rpc, policy);
    if (!found.exists) return { available: 0n, state: 'missing' as PolicyState };
    const { index, terms: t, consumed, rolled } = found.data;
    const profile = await pda(findProfilePda({ authority: t.authority }));
    const [profileData, from, clock] = await accounts(rpc, [profile, t.limit.from, CLOCK]);
    // Profile: tag, authority, policies, stale at 41. Clock: unix_timestamp at 32
    const [stale, now] = [profileData ? view(profileData).getBigUint64(41, true) : 0n, view(clock!).getBigInt64(32, true)];
    const { per, max } = t.limit;
    const window = (at: bigint) => (per.__kind === 'Total' ? 0n : (at - t.notBefore) / BigInt(per.seconds));
    const left = max - (window(now) === window(rolled) || per.__kind === 'Total' ? consumed : 0n);
    const held = from ? seat(from) : { allowance: 0n, balance: 0n, enabled: false };
    const notAfter = some(t.notAfter);
    const state: PolicyState =
        index <= stale ? 'invalidated' : now < t.notBefore ? 'notStarted' : notAfter !== null && now >= notAfter ? 'expired' : !held.enabled ? 'notDelegate' : 'active';
    const least = [left, held.allowance, held.balance].reduce((a, b) => (a < b ? a : b));
    return { allowance: held.allowance, available: state === 'active' ? least : 0n, balance: held.balance, left, policy: found, state };
}

/**
 * Every policy on chain that `authority` created, or that names `spender`:
 * what a wallet lists for its owner, and a merchant for itself. An owner's
 * are found by their numbers, with no scan of the program's accounts.
 */
export async function fetchPolicies(rpc: Client, by: { authority: Address } | { spender: Address }): Promise<Account<Policy>[]> {
    if ('authority' in by) {
        const profile = await fetchMaybeProfile(rpc, await pda(findProfilePda(by)));
        const count = profile.exists ? Number(profile.data.policies) : 0;
        const addresses = await Promise.all(Array.from({ length: count }, (_, i) => pda(findPolicyPda({ authority: by.authority, index: BigInt(i + 1) }))));
        const found: Account<Policy>[] = [];
        // getMultipleAccounts takes 100 addresses
        for (let i = 0; i < addresses.length; i += 100) {
            for (const account of await fetchAllMaybePolicy(rpc, addresses.slice(i, i + 100))) if (account.exists) found.push(account);
        }
        return found;
    }
    // Policy: a 59-byte header, then the terms: version, cluster, authority, spender: option tag at 93, key at 94
    const filters = [{ memcmp: { bytes: by.spender as string as Base58EncodedBytes, encoding: 'base58' as const, offset: 94n } }];
    const all = await rpc.getProgramAccounts(SOLANA_PULL_PROGRAM_ADDRESS, { encoding: 'base64', filters }).send();
    return all.flatMap(({ account, pubkey }) => {
        const data = new Uint8Array(getBase64Encoder().encode(account.data[0]));
        // tag 1 is a policy; a spender's key follows only a set option tag
        if (data[0] !== 1 || data[93] !== 1) return [];
        return [decodePolicy({ address: pubkey, data, executable: false, lamports: account.lamports, programAddress: SOLANA_PULL_PROGRAM_ADDRESS, space: BigInt(data.length) })];
    });
}

const used = (bits: ReadonlyUint8Array | null, salt: bigint) => {
    const bit = Number(salt % NONCE_BITS);
    return !!bits && (bits[bit >> 3] & (1 << (bit & 7))) !== 0;
};

const noncePage = async (rpc: Client, authority: Address, nonceIndex: bigint, page: bigint) => {
    const found = await fetchMaybeNonces(rpc, await pda(findNoncesPda({ authority, nonceIndex, page })));
    return found.exists ? found.data.bits : null;
};

/**
 * Whether an intent signed under `nonceIndex` can no longer run: its nonce is
 * used (it ran, or its authority cancelled it), or the authority's nonce
 * index has moved on.
 */
export async function fetchIntentUsed(rpc: Client, t: Terms, nonceIndex: bigint): Promise<boolean> {
    const profile = await fetchMaybeProfile(rpc, await pda(findProfilePda({ authority: t.authority })));
    if (!profile.exists || profile.data.nonceIndex !== nonceIndex) return true;
    return used(await noncePage(rpc, t.authority, nonceIndex, t.salt / NONCE_BITS), t.salt);
}

/**
 * The lowest salt whose nonce is free for a new intent of `authority` under
 * `nonceIndex`. Salts handed out in sequence share pages, so one page serves
 * 1,024 intents. `skip` are salts already given to intents that are signed
 * but not yet on chain.
 */
export async function nextSalt(rpc: Client, authority: Address, nonceIndex: bigint, skip: bigint[] = []): Promise<bigint> {
    for (let page = 0n; ; page++) {
        const bits = await noncePage(rpc, authority, nonceIndex, page);
        for (let salt = page * NONCE_BITS; salt < (page + 1n) * NONCE_BITS; salt++) {
            if (!used(bits, salt) && !skip.includes(salt)) return salt;
        }
    }
}

const EVENTS = {
    [SolanaPullEvent.Cancelled]: ['cancelled', parseCancelledEvent],
    [SolanaPullEvent.Closed]: ['closed', parseClosedEvent],
    [SolanaPullEvent.Created]: ['created', parseCreatedEvent],
    [SolanaPullEvent.Filled]: ['filled', parseFilledEvent],
    [SolanaPullEvent.Invalidated]: ['invalidated', parseInvalidatedEvent],
    [SolanaPullEvent.Opened]: ['opened', parseOpenedEvent],
    [SolanaPullEvent.Pulled]: ['pulled', parsePulledEvent],
} as const;

type EventOf<K extends keyof typeof EVENTS> = { type: (typeof EVENTS)[K][0] } & ReturnType<(typeof EVENTS)[K][1]>;
/** What the program did, as one of its events. */
export type PullEvent = { [K in keyof typeof EVENTS]: EventOf<K> }[keyof typeof EVENTS];

/**
 * The program's events in a confirmed transaction (`getTransaction` with
 * `encoding: 'json'`), in order; none if it failed. Each is an inner
 * instruction to the program that only its own engine can sign, so in a
 * transaction that succeeded no other program can have forged one.
 */
export function parseEvents(transaction: {
    meta: { err?: unknown; innerInstructions?: readonly { instructions: readonly { programIdIndex: number; data: string; stackHeight?: number | null }[] }[] | null; loadedAddresses?: { writable: readonly string[]; readonly: readonly string[] } | null } | null;
    transaction: { message: { accountKeys: readonly string[] } };
}): PullEvent[] {
    const keys = [...transaction.transaction.message.accountKeys, ...(transaction.meta?.loadedAddresses?.writable ?? []), ...(transaction.meta?.loadedAddresses?.readonly ?? [])];
    const events: PullEvent[] = [];
    if (transaction.meta?.err) return events;
    for (const { instructions } of transaction.meta?.innerInstructions ?? []) {
        for (const ix of instructions) {
            if (keys[ix.programIdIndex] !== SOLANA_PULL_PROGRAM_ADDRESS) continue;
            const data = new Uint8Array(getBase58Encoder().encode(ix.data));
            if (data[0] !== 255) continue;
            const [type, parse] = EVENTS[identifySolanaPullEvent(data)];
            events.push({ type, ...parse(data) } as PullEvent);
        }
    }
    return events;
}
