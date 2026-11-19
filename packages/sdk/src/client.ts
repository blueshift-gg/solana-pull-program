// What an integration calls: the journeys, built on the generated instructions.
import {
    type Account,
    type AccountMeta,
    type Address,
    AccountRole,
    address,
    type Base58EncodedBytes,
    getAddressDecoder,
    getBase58Decoder,
    getBase58Encoder,
    getBase64Encoder,
    getU64Encoder,
    type Instruction,
    unwrapOption,
    type ReadonlyUint8Array,
    type Rpc,
    type SolanaRpcApi,
    type TransactionSigner,
} from '@solana/kit';

import {
    decodeMandate,
    ENGINE_ADDRESS,
    fetchAllMaybeMandate,
    fetchMaybeMandate,
    fetchMaybeProfile,
    findMandatePda,
    findProfilePda,
    getCloseInstruction,
    getCreateInstructionAsync,
    getOpenInstructionAsync,
    getPullInstruction,
    getMandateTagBytes,
    getProfileDecoder,
    identifySolanaPullEvent,
    parseClosedEvent,
    parseCreatedEvent,
    parseInvalidatedEvent,
    parseOpenedEvent,
    parsePulledEvent,
    type Mandate,
    SOLANA_PULL_PROGRAM_ADDRESS,
    SolanaPullEvent,
    EVENT_DISCRIMINATOR,
    MANDATE_HEADER_LENGTH,
    MAX_TIME,
    type Terms,
    type TermsArgs,
    TOKEN_PROGRAM_ADDRESS,
} from './generated/index.ts';

const SYSTEM_PROGRAM = '11111111111111111111111111111111';

type Client = Rpc<SolanaRpcApi>;
const CLOCK = address('SysvarC1ock11111111111111111111111111111111');

const view = (data: ReadonlyUint8Array) => new DataView(data.buffer, data.byteOffset, data.byteLength);
const pda = async (found: Promise<readonly [Address, number]>) => (await found)[0];

/**
 * Whether the program accepts these terms, by the same rules as `pull-core`
 * (`npm test` checks both agree). Something must be bound: who takes, where
 * it goes, or what comes back.
 */
export function isValid(t: Terms): boolean {
    const time = (at: bigint) => at >= 0n && at <= MAX_TIME;
    const [notAfter, back] = [unwrapOption(t.notAfter), unwrapOption(t.in)];
    const window = time(t.notBefore) && (notAfter === null || (notAfter > t.notBefore && time(notAfter)));
    const bound = unwrapOption(t.spender) !== null || unwrapOption(t.out.to) !== null || back !== null;
    // The zero key is no account: `in` must name one
    const into = !back || (back.to !== SYSTEM_PROGRAM && back.min > 0n);
    const perPull = unwrapOption(t.out.maxPerPull);
    const out = t.out.max > 0n && (perPull === null || (perPull > 0n && perPull <= t.out.max));
    return window && out && into && bound;
}

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

const fetchAccounts = async (rpc: Client, addresses: Address[]) => {
    const { value } = await rpc.getMultipleAccounts(addresses, { encoding: 'base64' }).send();
    return value.map((account) => (account ? { data: new Uint8Array(getBase64Encoder().encode(account.data[0])), owner: account.owner } : null));
};
const accounts = async (rpc: Client, addresses: Address[]) => (await fetchAccounts(rpc, addresses)).map((a) => a?.data ?? null);

export async function fetchSeat(rpc: Client, account: Address) {
    const [data] = await accounts(rpc, [account]);
    return data ? seat(data) : null;
}

/**
 * Enable a token account: SPL `Approve` the engine as its delegate. `amount`
 * caps what every mandate on the account can take in total; leave it out for
 * no cap beyond their own limits.
 */
export const enableInstruction = (p: { owner: TransactionSigner; account: Address; amount?: bigint; tokenProgram?: Address }): Instruction => ({
    accounts: [
        { address: p.account, role: AccountRole.WRITABLE },
        { address: ENGINE_ADDRESS, role: AccountRole.READONLY },
        { address: p.owner.address, role: AccountRole.READONLY_SIGNER, signer: p.owner } as AccountMeta,
    ],
    data: Uint8Array.of(4, ...getU64Encoder().encode(p.amount ?? 2n ** 64n - 1n)),
    programAddress: p.tokenProgram ?? TOKEN_PROGRAM_ADDRESS,
});

/**
 * Everything an authority signs to start a mandate, as one transaction: enable
 * the token account if the engine is not its delegate, open the profile if
 * there is none, and create the mandate under the next number. Returns where
 * the mandate will be.
 */
export async function onboardInstructions(rpc: Client, p: { authority: TransactionSigner; payer: TransactionSigner; terms: TermsArgs }) {
    const profileAddress = await pda(findProfilePda({ authority: p.authority.address }));
    const [[from], profile] = await Promise.all([fetchAccounts(rpc, [p.terms.out.from]), fetchMaybeProfile(rpc, profileAddress)]);
    if (!from) throw new Error(`token account ${p.terms.out.from} does not exist`);
    const index = (profile.exists ? profile.data.mandates : 0n) + 1n;
    const mandate = await pda(findMandatePda({ authority: p.authority.address, index }));
    const instructions: Instruction[] = [
        ...(seat(from.data).enabled ? [] : [enableInstruction({ account: p.terms.out.from, owner: p.authority, tokenProgram: from.owner })]),
        ...(profile.exists ? [] : [await getOpenInstructionAsync({ authority: p.authority, payer: p.payer })]),
        await getCreateInstructionAsync({ authority: p.authority, payer: p.payer, mandate, terms: p.terms }),
    ];
    return { index, instructions, mandate };
}

/**
 * Take `amount` under a mandate (`fetchMandateStatus` or `fetchMandates` give
 * it). `to` is where the tokens go unless the terms name it; `payFrom` is the
 * spender's account that pays what must come back, and `pay` how much it
 * sends: `in.min` unless the mint takes a transfer fee, which the spender
 * adds. `reference` is the spender's own 32-byte id for this pull, which the
 * event carries. The mints and token programs are read from the token
 * accounts the terms name.
 */
export async function pullInstruction(rpc: Client, p: { spender: TransactionSigner; mandate: Account<Mandate>; amount: bigint; to?: Address; payFrom?: Address; pay?: bigint; reference?: Uint8Array }) {
    const { authority, terms: t } = p.mandate.data;
    const [to, back] = [unwrapOption(t.out.to) ?? p.to, unwrapOption(t.in)];
    if (!to) throw new Error('the terms do not name where the tokens go: pass `to`');
    if (back && !p.payFrom) throw new Error('something must come back: pass `payFrom`');
    const [from, payTo] = await fetchAccounts(rpc, [t.out.from, ...(back ? [back.to] : [])]);
    if (!from || (back && !payTo)) throw new Error('a token account the terms name does not exist');
    // Token account: mint at 0
    const mint = (a: { data: Uint8Array }) => getAddressDecoder().decode(a.data.subarray(0, 32));
    return getPullInstruction({
        amount: p.amount,
        from: t.out.from,
        mandate: p.mandate.address,
        mint: mint(from),
        pay: back ? (p.pay ?? back.min) : 0n,
        profile: await pda(findProfilePda({ authority })),
        reference: p.reference ?? null,
        spender: p.spender,
        to,
        tokenProgram: from.owner,
        ...(back ? { payFrom: p.payFrom, payMint: mint(payTo!), payProgram: payTo!.owner, payTo: back.to } : {}),
    });
}

/**
 * End a mandate and return its rent to whoever paid it: its authority at any
 * time, anyone once it can never pull again: expired, invalidated or used up.
 */
export const closeMandateInstruction = async (p: { closer: TransactionSigner; mandate: Account<Mandate> }) =>
    getCloseInstruction({ closer: p.closer, mandate: p.mandate.address, payer: p.mandate.data.payer, profile: await pda(findProfilePda({ authority: p.mandate.data.authority })) });

/** The state of a mandate, as a spender needs it before it pulls. */
export type MandateState =
    /** In force. `available` is what one pull can take now. */
    | 'active'
    /** Not on chain: never created, or closed. */
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
    const { authority, index, terms: t, consumed, window: counted } = found.data;
    const profile = await pda(findProfilePda({ authority }));
    const [profileData, from, clock] = await accounts(rpc, [profile, t.out.from, CLOCK]);
    // Clock: unix_timestamp at 32
    const [stale, now] = [profileData ? getProfileDecoder().decode(profileData).stale : 0n, view(clock!).getBigInt64(32, true)];
    const period = unwrapOption(t.out.period);
    const window = period === null ? 0n : (now - t.notBefore) / BigInt(period);
    const left = t.out.max - (window === counted ? consumed : 0n);
    const held = from ? seat(from) : { allowance: 0n, balance: 0n, enabled: false };
    const notAfter = unwrapOption(t.notAfter);
    const state: MandateState =
        index <= stale ? 'invalidated' : now < t.notBefore ? 'notStarted' : notAfter !== null && now >= notAfter ? 'expired' : !held.enabled ? 'notDelegate' : 'active';
    const least = [left, unwrapOption(t.out.maxPerPull) ?? left, held.allowance, held.balance].reduce((a, b) => (a < b ? a : b));
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
    // A mandate, by its tag, whose terms (after the header) start with this spender
    const filters = [
        { memcmp: { bytes: getBase58Decoder().decode(getMandateTagBytes()) as Base58EncodedBytes, encoding: 'base58' as const, offset: 0n } },
        { memcmp: { bytes: by.spender as string as Base58EncodedBytes, encoding: 'base58' as const, offset: MANDATE_HEADER_LENGTH } },
    ];
    const all = await rpc.getProgramAccounts(SOLANA_PULL_PROGRAM_ADDRESS, { encoding: 'base64', filters }).send();
    return all.map(({ account, pubkey }) => {
        const data = new Uint8Array(getBase64Encoder().encode(account.data[0]));
        return decodeMandate({ address: pubkey, data, executable: false, lamports: account.lamports, programAddress: SOLANA_PULL_PROGRAM_ADDRESS, space: BigInt(data.length) });
    });
}

const EVENTS = {
    [SolanaPullEvent.Closed]: ['closed', parseClosedEvent],
    [SolanaPullEvent.Created]: ['created', parseCreatedEvent],
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
            if (data[0] !== EVENT_DISCRIMINATOR) continue;
            const [type, parse] = EVENTS[identifySolanaPullEvent(data)];
            events.push({ type, ...parse(data) } as PullEvent);
        }
    }
    return events;
}
