// The intent text: what an authority reads and signs, and the validity rules
// of terms. This is the same text and the same rules as the program's
// (`pull-core`), checked line for line against it by `npm test`.
import { type Address, getAddressEncoder, type Option } from '@solana/kit';

import { Envelope, SOLANA_PULL_PROGRAM_ADDRESS, type Terms } from './generated/index.ts';

/** The cluster byte of terms, and the name the text shows for it. */
export const CLUSTERS = ['mainnet', 'devnet', 'testnet', 'localnet'] as const;
/** 9999-12-31T23:59:59Z: the last instant the text can show. */
const MAX_TIME = 253_402_300_799n;

const some = <T>(o: Option<T>): T | null => (o.__option === 'Some' ? o.value : null);
const renderable = (t: bigint) => t >= 0n && t <= MAX_TIME;

/**
 * Whether the program accepts these terms. Something must be bound: who
 * takes, where it goes, or what the authority receives.
 */
export function isValid(terms: Terms): boolean {
    const [notAfter, receive] = [some(terms.notAfter), some(terms.receive)];
    const window = renderable(terms.notBefore) && (notAfter === null || (notAfter > terms.notBefore && renderable(notAfter)));
    const { per } = terms.limit;
    const limit = terms.limit.max > 0n && !(per.__kind === 'Every' && per.seconds === 0);
    const decay = receive && some(receive.decay);
    const received = !receive || (receive.min > 0n && (!decay || (renderable(decay.t0) && renderable(decay.t1) && decay.t0 < decay.t1 && decay.min > 0n)));
    const bound = some(terms.spender) !== null || some(terms.limit.to) !== null || receive !== null;
    return terms.version === 1 && terms.cluster < CLUSTERS.length && window && limit && received && bound;
}

/** `raw` with exactly `decimals` fractional digits. */
function amount(raw: bigint, decimals: number): string {
    if (decimals === 0) return raw.toString();
    const digits = raw.toString().padStart(decimals + 1, '0');
    return `${digits.slice(0, -decimals)}.${digits.slice(-decimals)}`;
}

/** `YYYY-MM-DDTHH:MM:SSZ`. */
const time = (t: bigint) => new Date(Number(t) * 1000).toISOString().replace('.000Z', 'Z');

function duration(seconds: number): string {
    if (seconds === 0) return '0s';
    const parts: [number, string][] = [[Math.floor(seconds / 86_400), 'd'], [Math.floor(seconds / 3_600) % 24, 'h'], [Math.floor(seconds / 60) % 60, 'm'], [seconds % 60, 's']];
    return parts.filter(([n]) => n > 0).map(([n, unit]) => `${n}${unit}`).join('');
}

/**
 * The text of an intent: `terms` signed under the authority's `nonceIndex`
 * (its profile's). `decimals` gives the decimals of each mint the terms name.
 * Throws on terms the program would refuse.
 */
export function renderIntent(terms: Terms, nonceIndex: bigint, decimals: Record<string, number>): string {
    if (!isValid(terms)) throw new Error('these terms are not valid');
    const of = (mint: Address) => {
        if (decimals[mint] === undefined) throw new Error(`decimals of mint ${mint} are missing`);
        return decimals[mint];
    };
    const { limit } = terms;
    const [spender, to, receive, notAfter] = [some(terms.spender), some(limit.to), some(terms.receive), some(terms.notAfter)];
    const lines = [
        'Solana Pull v1',
        `cluster: ${CLUSTERS[terms.cluster]}`,
        `engine: ${SOLANA_PULL_PROGRAM_ADDRESS}`,
        `authority: ${terms.authority}`,
        `SPENDER: ${spender ?? 'anyone'}`,
        `MAY TAKE: at most ${amount(limit.max, of(limit.mint))} of mint ${limit.mint} from ${limit.from}${to ? ` to ${to}` : ''}${limit.per.__kind === 'Total' ? ' in total' : ` every ${duration(limit.per.seconds)}`}`,
    ];
    if (receive) {
        const decay = some(receive.decay);
        const moving = decay ? `, moving to ${amount(decay.min, of(receive.mint))} from ${time(decay.t0)} to ${time(decay.t1)}` : '';
        lines.push(`MUST RECEIVE: at least ${amount(receive.min, of(receive.mint))} of mint ${receive.mint} in ${receive.to} for each use${moving}`);
    }
    lines.push(`VALID: from ${time(terms.notBefore)} until ${notAfter === null ? 'revoked' : time(notAfter)}`, `INDEX: ${nonceIndex}`, `SALT: ${terms.salt}`);
    return lines.join('\n');
}

const KEY = '([1-9A-HJ-NP-Za-km-z]{32,44})';
const AMOUNT = '(\\d+(?:\\.\\d+)?)';
const TIME = '(\\d{4}-\\d\\d-\\d\\dT\\d\\d:\\d\\d:\\d\\dZ)';
const TEXT = new RegExp(
    [
        '^Solana Pull v1',
        'cluster: (\\w+)',
        `engine: ${KEY}`,
        `authority: ${KEY}`,
        `SPENDER: (?:anyone|${KEY})`,
        `MAY TAKE: at most ${AMOUNT} of mint ${KEY} from ${KEY}(?: to ${KEY})? (?:in total|every ((?:\\d+[dhms])+))`,
        `(?:MUST RECEIVE: at least ${AMOUNT} of mint ${KEY} in ${KEY} for each use(?:, moving to ${AMOUNT} from ${TIME} to ${TIME})?\n)?VALID: from ${TIME} until (?:revoked|${TIME})`,
        'INDEX: (\\d+)',
        'SALT: (\\d+)$',
    ].join('\n'),
);

const raw = (shown: string) => BigInt(shown.replace('.', ''));
const places = (shown: string) => (shown.includes('.') ? shown.length - shown.indexOf('.') - 1 : 0);
const seconds = (iso: string) => BigInt(Date.parse(iso) / 1000);
const option = <T>(value: T | undefined): Option<T> => (value === undefined ? { __option: 'None' } : { __option: 'Some', value });
const UNITS = { d: 86_400, h: 3_600, m: 60, s: 1 };

/**
 * Read an intent text back into what it says: how a wallet recognises one in
 * a message it is asked to sign, and shows it as an approval. Returns null
 * for any text that is not exactly what `renderIntent` writes for valid
 * terms, so one text means one thing. `decimals` are the ones the text
 * shows; a wallet compares them with each mint's own before it trusts the
 * amounts.
 */
export function parseIntent(text: string): { terms: Terms; nonceIndex: bigint; decimals: Record<string, number> } | null {
    const m = TEXT.exec(text);
    if (!m) return null;
    const [, cluster, engine, authority, spender, max, mint, from, to, every, min, payMint, payTo, decayMin, t0, t1, notBefore, notAfter, index, salt] = m;
    if (engine !== SOLANA_PULL_PROGRAM_ADDRESS) return null;
    try {
        const per = every ? { __kind: 'Every' as const, seconds: [...every.matchAll(/(\d+)([dhms])/g)].reduce((sum, [, n, unit]) => sum + Number(n) * UNITS[unit as keyof typeof UNITS], 0) } : { __kind: 'Total' as const };
        const decay = decayMin === undefined ? undefined : { min: raw(decayMin), t0: seconds(t0), t1: seconds(t1) };
        const terms: Terms = {
            authority: authority as Address,
            cluster: CLUSTERS.indexOf(cluster as (typeof CLUSTERS)[number]),
            limit: { from: from as Address, max: raw(max), mint: mint as Address, per, to: option(to as Address | undefined) },
            notAfter: option(notAfter === undefined ? undefined : seconds(notAfter)),
            notBefore: seconds(notBefore),
            receive: option(min === undefined ? undefined : { decay: option(decay), min: raw(min), mint: payMint as Address, to: payTo as Address }),
            salt: BigInt(salt),
            spender: option(spender as Address | undefined),
            version: 1,
        };
        const decimals = { [mint]: places(max), ...(min === undefined ? {} : { [payMint]: places(min) }) };
        const nonceIndex = BigInt(index);
        // Only the canonical text counts: the one the program itself would render
        return renderIntent(terms, nonceIndex, decimals) === text ? { decimals, nonceIndex, terms } : null;
    } catch {
        return null;
    }
}

/**
 * The exact bytes the authority signs for an intent, and the program
 * verifies. Wallets do not agree on one way to sign a message, so there are
 * two, and `Fill` is told which with the same `Envelope`:
 *
 * - `Envelope.Text`: the text alone. Pass these bytes to
 *   `solana:signMessage`; every wallet with software keys signs them.
 * - `Envelope.OffchainMessage`: the text in an Offchain Message v1 with the
 *   authority as its one signer. A wallet with `solana:signOffchainMessage`
 *   builds the same bytes from `renderIntent`'s text and returns them.
 */
export function intentMessage(terms: Terms, nonceIndex: bigint, decimals: Record<string, number>, envelope: Envelope): Uint8Array {
    const text = new TextEncoder().encode(renderIntent(terms, nonceIndex, decimals));
    if (envelope === Envelope.Text) return text;
    return Uint8Array.from([0xff, ...new TextEncoder().encode('solana offchain'), 1, 1, ...getAddressEncoder().encode(terms.authority), ...text]);
}
