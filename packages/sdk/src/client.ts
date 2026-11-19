// What an integration calls: the journeys, built on the generated instructions.
import {
    type Account,
    type AccountMeta,
    type Address,
    AccountRole,
    address,
    type Base58EncodedBytes,
    getAddressDecoder,
    getAddressEncoder,
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
    decodeMandate,
    ENGINE_ADDRESS,
    Envelope,
    fetchAllMaybeMandate,
    fetchMaybeNonces,
    fetchMaybeMandate,
    fetchMaybeProfile,
    findNoncesPda,
    findMandatePda,
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
    type Mandate,
    SOLANA_PULL_PROGRAM_ADDRESS,
    SolanaPullEvent,
    type Terms,
    type TermsArgs,
} from './generated/index.ts';
import { authorizationMessage, renderAuthorization } from './text.ts';

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
 * `enableInstruction` restores it, and every mandate works again as it was.
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
 * caps what every mandate and authorization on the account can take in total; leave
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
 * Everything an authority signs to start a mandate, as one transaction: enable
 * the token account if the engine is not its delegate, open the profile if
 * there is none, and create the mandate under the next number. Returns where
 * the mandate will be.
 */
export async function onboardInstructions(rpc: Client, p: { authority: TransactionSigner; payer: TransactionSigner; terms: TermsArgs; tokenProgram?: Address }) {
    const profileAddress = await pda(findProfilePda({ authority: p.authority.address }));
    const [[from], profile] = await Promise.all([accounts(rpc, [p.terms.limit.from]), fetchMaybeProfile(rpc, profileAddress)]);
    if (!from) throw new Error(`token account ${p.terms.limit.from} does not exist`);
    const index = (profile.exists ? profile.data.mandates : 0n) + 1n;
    const mandate = await pda(findMandatePda({ authority: p.authority.address, index }));
    const instructions: Instruction[] = [
        ...(seat(from).enabled ? [] : [enableInstruction({ account: p.terms.limit.from, owner: p.authority, tokenProgram: p.tokenProgram })]),
        ...(profile.exists ? [] : [await getOpenInstructionAsync({ authority: p.authority, payer: p.payer })]),
        await getCreateInstructionAsync({ authority: p.authority, payer: p.payer, mandate, terms: p.terms }),
    ];
    return { index, instructions, mandate };
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
 * Take `amount` under a mandate (`fetchMandateStatus` or `fetchMandates` give
 * it). `to` is where the tokens go unless the terms name it; `payFrom` is the
 * spender's account that pays what the authority must receive; `reference`
 * is the spender's own 32-byte id for this pull, which the event carries.
 */
export const pullInstruction = async (p: Legs & { spender: TransactionSigner; mandate: Account<Mandate>; amount: bigint; reference?: Uint8Array }) =>
    getPullInstruction({ ...(await legs(p.mandate.data.terms, p)), amount: p.amount, mandate: p.mandate.address, reference: p.reference ?? null, spender: p.spender });

/** A signed authorization, as it travels from the owner's wallet to whoever fills it. */
export type Authorization = {
    terms: Terms;
    /** The owner's nonce index when it signed. The authorization works only while it is the profile's. */
    nonceIndex: bigint;
    /** What the owner's wallet put before the text in the bytes it signed. */
    envelope: Envelope;
    signature: ReadonlyUint8Array;
};

/** An authorization as bytes, to hand to whoever fills it: envelope, nonce index, signature, terms. */
export function encodeAuthorization(a: Authorization): Uint8Array {
    return Uint8Array.from([a.envelope, ...getU64Encoder().encode(a.nonceIndex), ...a.signature, ...getTermsCodec().encode(a.terms)]);
}

export function decodeAuthorization(bytes: ReadonlyUint8Array): Authorization {
    if (bytes.length < 73 || bytes[0] > 1) throw new Error('not an authorization');
    return { envelope: bytes[0] as Envelope, nonceIndex: view(bytes).getBigUint64(1, true), signature: bytes.slice(9, 73), terms: getTermsCodec().decode(bytes.subarray(73)) };
}

/** The decimals of each mint the terms name, read from the mints themselves: what the text shows amounts with. */
export async function fetchDecimals(rpc: Client, t: Terms): Promise<Record<string, number>> {
    const mints = [t.limit.mint, ...(some(t.receive) ? [some(t.receive)!.mint] : [])];
    const found = await accounts(rpc, mints);
    return Object.fromEntries(
        mints.map((mint, i) => {
            if (!found[i]) throw new Error(`mint ${mint} does not exist`);
            // Mint: mint_authority: COption<Pubkey>, supply: u64, decimals at 44
            return [mint, found[i]![44]];
        }),
    );
}

/** Whether the signature is the authority's over exactly this authorization, as the program will check it. */
export async function verifyAuthorization(a: Authorization, decimals: Record<string, number>): Promise<boolean> {
    const key = await crypto.subtle.importKey('raw', getAddressEncoder().encode(a.terms.authority) as Uint8Array<ArrayBuffer>, 'Ed25519', false, ['verify']);
    const message = authorizationMessage(a.terms, a.nonceIndex, decimals, a.envelope);
    return crypto.subtle.verify('Ed25519', key, a.signature as Uint8Array<ArrayBuffer>, message as Uint8Array<ArrayBuffer>);
}

/** A wallet, as the Wallet Standard presents one, and the account that signs. */
type SigningWallet = { features: Readonly<Record<string, unknown>> };
type SigningAccount = { address: string; publicKey: ReadonlyUint8Array };
type SignMessage = { signMessage(input: { account: SigningAccount; message: Uint8Array }): Promise<readonly { signedMessage: ReadonlyUint8Array; signature: ReadonlyUint8Array }[]> };
type SignOffchainMessage = {
    supportedMessageVersions: readonly number[];
    signOffchainMessage(input: { messageVersion: 1; account: SigningAccount; message: string; requiredSigners: readonly ReadonlyUint8Array[] }): Promise<readonly { signedOffchainMessage: ReadonlyUint8Array; signature: ReadonlyUint8Array }[]>;
};

const same = (a: ReadonlyUint8Array, b: ReadonlyUint8Array) => a.length === b.length && a.every((byte, i) => byte === b[i]);

/**
 * Ask the owner's wallet to sign an authorization, in whichever way the
 * wallet can: as an Offchain Message if it has `solana:signOffchainMessage`,
 * as plain text through `solana:signMessage` otherwise. Leave `salt` out to
 * take the next free one. The result is checked before it is returned: the
 * wallet signed exactly the text, and the signature is the one the program
 * will accept. The owner must have a profile (`onboardInstructions`, or
 * `getOpenInstructionAsync`) and the token account must be enabled.
 */
export async function requestAuthorization(rpc: Client, p: { wallet: SigningWallet; account: SigningAccount; terms: Omit<TermsArgs, 'salt'> & { salt?: bigint }; skip?: bigint[] }): Promise<Authorization> {
    const authority = p.terms.authority;
    if (authority !== p.account.address) throw new Error('the terms name another account as their authority');
    const profile = await fetchMaybeProfile(rpc, await pda(findProfilePda({ authority })));
    if (!profile.exists) throw new Error('this account has no profile yet: open one first');
    const { nonceIndex } = profile.data;
    const t = terms({ ...p.terms, salt: p.terms.salt ?? (await nextSalt(rpc, authority, nonceIndex, p.skip)) });
    const decimals = await fetchDecimals(rpc, t);
    const text = renderAuthorization(t, nonceIndex, decimals);

    const offchain = p.wallet.features['solana:signOffchainMessage'] as SignOffchainMessage | undefined;
    const plain = p.wallet.features['solana:signMessage'] as SignMessage | undefined;
    const envelope = offchain?.supportedMessageVersions.includes(1) ? Envelope.OffchainMessage : Envelope.Text;
    const expected = authorizationMessage(t, nonceIndex, decimals, envelope);
    let signed: { bytes: ReadonlyUint8Array; signature: ReadonlyUint8Array };
    if (envelope === Envelope.OffchainMessage) {
        const [out] = await offchain!.signOffchainMessage({ account: p.account, message: text, messageVersion: 1, requiredSigners: [p.account.publicKey] });
        signed = { bytes: out.signedOffchainMessage, signature: out.signature };
    } else {
        if (!plain) throw new Error('this wallet cannot sign messages');
        const [out] = await plain.signMessage({ account: p.account, message: expected });
        signed = { bytes: out.signedMessage, signature: out.signature };
    }
    // A wallet may wrap a message before it signs, as some do for a hardware key; the program would refuse that
    if (!same(signed.bytes, expected)) throw new Error('this wallet did not sign the text as it was given, so the program cannot verify it');
    const authorization = { envelope, nonceIndex, signature: signed.signature, terms: t };
    if (!(await verifyAuthorization(authorization, decimals))) throw new Error("the wallet's signature does not verify");
    return authorization;
}

/**
 * Run a signed authorization, once. `payer` funds the page of nonces if this
 * authorization is the first in it, and gets it back when the page closes.
 */
export const fillInstruction = async (p: Legs & { spender: TransactionSigner; payer: TransactionSigner; authorization: Authorization; amount: bigint }) => {
    const { terms: t, nonceIndex, envelope, signature } = p.authorization;
    return getFillInstruction({
        ...(await legs(t, p)),
        amount: p.amount,
        envelope,
        nonces: await pda(findNoncesPda({ authority: t.authority, nonceIndex, page: t.salt / NONCE_BITS })),
        payer: p.payer,
        signature,
        spender: p.spender,
        terms: t,
    });
};

/** The authority ends one mandate. It closes at once, and its rent goes back to whoever paid it. */
export const cancelMandateInstruction = (p: { authority: TransactionSigner; mandate: Account<Mandate> }) =>
    getCancelInstructionAsync({ account: p.mandate.address, authority: p.authority, payer: p.mandate.data.payer, salt: null });

/** The authority uses up the nonce of one authorization it signed under `nonceIndex`, so it can never be filled. */
export const cancelAuthorizationInstruction = async (p: { authority: TransactionSigner; payer: TransactionSigner; nonceIndex: bigint; salt: bigint }) =>
    getCancelInstructionAsync({
        account: await pda(findNoncesPda({ authority: p.authority.address, nonceIndex: p.nonceIndex, page: p.salt / NONCE_BITS })),
        authority: p.authority,
        payer: p.payer,
        salt: p.salt,
    });

/**
 * Anyone closes a mandate that can never be used again (expired, or
 * invalidated by its authority), and its rent goes back to whoever paid it.
 */
export const closeMandateInstruction = async (p: { closer: TransactionSigner; mandate: Account<Mandate> }) =>
    getCloseInstruction({ account: p.mandate.address, closer: p.closer, payer: p.mandate.data.payer, profile: await pda(findProfilePda({ authority: p.mandate.data.terms.authority })) });

/** The state of a mandate, as a spender needs it before it pulls. */
export type MandateState =
    /** In force. `available` is what one pull can take now. */
    | 'active'
    /** Not on chain: never created, cancelled, or closed. */
    | 'missing'
    /** Its authority ended every mandate up to this one. */
    | 'invalidated'
    | 'notStarted'
    | 'expired'
    /** The engine is not the delegate of the token account: the authority must enable it again. */
    | 'notDelegate';

/**
 * Whether a pull under `mandate` can go through now, and for how much: the
 * least of what is left of the limit in this window, what is left of the
 * token account's approval, and its balance. One answer from the mandate, its
 * authority's profile, the token account and the chain's clock.
 */
export async function fetchMandateStatus(rpc: Client, mandate: Address) {
    const found = await fetchMaybeMandate(rpc, mandate);
    if (!found.exists) return { available: 0n, state: 'missing' as MandateState };
    const { index, terms: t, consumed, rolled } = found.data;
    const profile = await pda(findProfilePda({ authority: t.authority }));
    const [profileData, from, clock] = await accounts(rpc, [profile, t.limit.from, CLOCK]);
    // Profile: tag, authority, mandates, stale at 41. Clock: unix_timestamp at 32
    const [stale, now] = [profileData ? view(profileData).getBigUint64(41, true) : 0n, view(clock!).getBigInt64(32, true)];
    const { per, max } = t.limit;
    const window = (at: bigint) => (per.__kind === 'Total' ? 0n : (at - t.notBefore) / BigInt(per.seconds));
    const left = max - (window(now) === window(rolled) || per.__kind === 'Total' ? consumed : 0n);
    const held = from ? seat(from) : { allowance: 0n, balance: 0n, enabled: false };
    const notAfter = some(t.notAfter);
    const state: MandateState =
        index <= stale ? 'invalidated' : now < t.notBefore ? 'notStarted' : notAfter !== null && now >= notAfter ? 'expired' : !held.enabled ? 'notDelegate' : 'active';
    const least = [left, held.allowance, held.balance].reduce((a, b) => (a < b ? a : b));
    return { allowance: held.allowance, available: state === 'active' ? least : 0n, balance: held.balance, left, mandate: found, state };
}

/**
 * Every mandate on chain that `authority` created, or that names `spender`:
 * what a wallet lists for its owner, and a merchant for itself. An owner's
 * are found by their numbers, with no scan of the program's accounts.
 */
export async function fetchMandates(rpc: Client, by: { authority: Address } | { spender: Address }): Promise<Account<Mandate>[]> {
    if ('authority' in by) {
        const profile = await fetchMaybeProfile(rpc, await pda(findProfilePda(by)));
        const count = profile.exists ? Number(profile.data.mandates) : 0;
        const addresses = await Promise.all(Array.from({ length: count }, (_, i) => pda(findMandatePda({ authority: by.authority, index: BigInt(i + 1) }))));
        const found: Account<Mandate>[] = [];
        // getMultipleAccounts takes 100 addresses
        for (let i = 0; i < addresses.length; i += 100) {
            for (const account of await fetchAllMaybeMandate(rpc, addresses.slice(i, i + 100))) if (account.exists) found.push(account);
        }
        return found;
    }
    // Mandate: a 59-byte header, then the terms: version, cluster, authority, spender: option tag at 93, key at 94
    const filters = [{ memcmp: { bytes: by.spender as string as Base58EncodedBytes, encoding: 'base58' as const, offset: 94n } }];
    const all = await rpc.getProgramAccounts(SOLANA_PULL_PROGRAM_ADDRESS, { encoding: 'base64', filters }).send();
    return all.flatMap(({ account, pubkey }) => {
        const data = new Uint8Array(getBase64Encoder().encode(account.data[0]));
        // tag 1 is a mandate; a spender's key follows only a set option tag
        if (data[0] !== 1 || data[93] !== 1) return [];
        return [decodeMandate({ address: pubkey, data, executable: false, lamports: account.lamports, programAddress: SOLANA_PULL_PROGRAM_ADDRESS, space: BigInt(data.length) })];
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

/** The state of a signed authorization, as whoever would fill it needs it. */
export type AuthorizationState =
    /** It can run now. `available` is the most one fill can take. */
    | 'fillable'
    /** The signature is not the authority's over these terms. */
    | 'invalidSignature'
    /** The authority ended every authorization it had signed, or has no profile. */
    | 'invalidated'
    /** It ran already, or the authority cancelled it. */
    | 'used'
    | 'notStarted'
    | 'expired'
    /** The engine is not the delegate of the token account. */
    | 'notDelegate';

/**
 * Whether an authorization can be filled now, and for how much, before
 * anyone pays for a transaction: the signature, the authority's profile, the
 * nonce, the window and the token account, in the order the program checks.
 */
export async function fetchAuthorizationStatus(rpc: Client, a: Authorization) {
    const t = a.terms;
    const [profile, page] = await Promise.all([pda(findProfilePda({ authority: t.authority })), pda(findNoncesPda({ authority: t.authority, nonceIndex: a.nonceIndex, page: t.salt / NONCE_BITS }))]);
    const [[profileData, pageData, from, clock], decimals] = await Promise.all([accounts(rpc, [profile, page, t.limit.from, CLOCK]), fetchDecimals(rpc, t)]);
    // Profile: nonce_index at 49. Nonces: bits at 81. Clock: unix_timestamp at 32
    const now = view(clock!).getBigInt64(32, true);
    const held = from ? seat(from) : { allowance: 0n, balance: 0n, enabled: false };
    const notAfter = some(t.notAfter);
    const state: AuthorizationState = !(await verifyAuthorization(a, decimals))
        ? 'invalidSignature'
        : !profileData || view(profileData).getBigUint64(49, true) !== a.nonceIndex
          ? 'invalidated'
          : used(pageData?.subarray(81) ?? null, t.salt)
            ? 'used'
            : now < t.notBefore
              ? 'notStarted'
              : notAfter !== null && now >= notAfter
                ? 'expired'
                : !held.enabled
                  ? 'notDelegate'
                  : 'fillable';
    const least = [t.limit.max, held.allowance, held.balance].reduce((x, y) => (x < y ? x : y));
    return { allowance: held.allowance, available: state === 'fillable' ? least : 0n, balance: held.balance, state };
}

/**
 * The lowest salt whose nonce is free for a new authorization of `authority` under
 * `nonceIndex`. Salts handed out in sequence share pages, so one page serves
 * 1,024 authorizations. `skip` are salts already given to authorizations that are signed
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
