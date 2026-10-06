// For wallets: say what a transaction or a message does with the Pull
// program, in rows a wallet can show before the owner signs.
import { type Address, type Instruction, type Option, type ReadonlyUint8Array } from '@solana/kit';

import { ENGINE_ADDRESS, parseSolanaPullInstruction, SOLANA_PULL_PROGRAM_ADDRESS, SolanaPullInstruction, type Terms } from './generated/index.ts';
import { parseIntent } from './text.ts';

const TOKEN_PROGRAMS: string[] = ['TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA', 'TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb'];
const some = <T>(o: Option<T>): T | null => (o.__option === 'Some' ? o.value : null);

/** One line of an approval: a label and what it says. */
export type Row = [label: string, value: string];
/** What the owner is asked to approve. `caution` is for what deserves a second look. */
export type Approval = { title: string; rows: Row[]; caution?: string };
/** What a wallet knows about tokens: decimals and a symbol per mint. Amounts of a mint it does not know are shown raw. */
export type Tokens = Record<string, { decimals: number; symbol?: string }>;

const short = (a: Address) => `${a.slice(0, 4)}…${a.slice(-4)}`;
const date = (t: bigint) => new Date(Number(t) * 1000).toISOString().replace('.000Z', 'Z');

function span(seconds: number): string {
    const units: [number, string][] = [[86_400, 'day'], [3_600, 'hour'], [60, 'minute'], [1, 'second']];
    const [size, name] = units.find(([size]) => seconds % size === 0)!;
    const n = seconds / size;
    return n === 1 ? name : `${n} ${name}s`;
}

function amount(raw: bigint, mint: Address, tokens: Tokens): string {
    const token = tokens[mint];
    if (!token) return `${raw} base units of ${short(mint)}`;
    const digits = raw.toString().padStart(token.decimals + 1, '0');
    const [whole, fraction] = [digits.slice(0, digits.length - token.decimals), digits.slice(digits.length - token.decimals).replace(/0+$/, '')];
    return `${whole}${fraction ? `.${fraction}` : ''} ${token.symbol ?? short(mint)}`;
}

/** Terms in a wallet's words: who can take what, where it goes, what comes back, and until when. */
export function describeTerms(terms: Terms, tokens: Tokens = {}): Row[] {
    const { limit } = terms;
    const [spender, to, receive, notAfter] = [some(terms.spender), some(limit.to), some(terms.receive), some(terms.notAfter)];
    const rows: Row[] = [
        ['Who can take it', spender ? short(spender) : 'Anyone'],
        ['Can take', `Up to ${amount(limit.max, limit.mint, tokens)}${limit.per.__kind === 'Every' ? ` every ${span(limit.per.seconds)}` : ' in total'}`],
        ['From', short(limit.from)],
        ['Goes to', to ? `Only ${short(to)}` : 'Wherever the taker chooses'],
    ];
    if (receive) {
        const decay = some(receive.decay);
        const moving = decay ? `, moving to ${amount(decay.min, receive.mint, tokens)} between ${date(decay.t0)} and ${date(decay.t1)}` : '';
        rows.push(['Only if you receive', `At least ${amount(receive.min, receive.mint, tokens)} each time${moving}`]);
    }
    rows.push(['Starts', date(terms.notBefore)], ['Ends', notAfter === null ? 'Never, until you cancel it' : date(notAfter)]);
    return rows;
}

const NO_EXPIRY = 'This does not expire. It stays usable until it runs or you cancel it.';

/**
 * What one instruction of a transaction does, if it concerns the Pull
 * program: enabling a token account, or any of the program's own
 * instructions. Null for anything else.
 */
export function describeInstruction(instruction: Instruction, tokens: Tokens = {}): Approval | null {
    const data = (instruction.data ?? new Uint8Array()) as ReadonlyUint8Array;
    if (TOKEN_PROGRAMS.includes(instruction.programAddress)) {
        // SPL `Approve` (4) and `ApproveChecked` (13): source, [mint,] delegate
        const delegate = data[0] === 4 ? instruction.accounts?.[1] : data[0] === 13 ? instruction.accounts?.[2] : undefined;
        if (delegate?.address !== ENGINE_ADDRESS) return null;
        const cap = new DataView(data.buffer, data.byteOffset).getBigUint64(1, true);
        const rows: Row[] = [['Token account', short(instruction.accounts![0].address)], ['Cap', cap === 2n ** 64n - 1n ? 'None: each approval sets its own limit' : `${cap} base units across every approval`]];
        return { rows, title: 'Turn on pull payments for this token' };
    }
    if (instruction.programAddress !== SOLANA_PULL_PROGRAM_ADDRESS || !instruction.accounts) return null;
    const parsed = parseSolanaPullInstruction(instruction as Parameters<typeof parseSolanaPullInstruction>[0]);
    switch (parsed.instructionType) {
        case SolanaPullInstruction.Open:
            return { rows: [], title: 'Open your profile with the Pull program' };
        case SolanaPullInstruction.Create: {
            const never = some(parsed.data.terms.notAfter) === null;
            return { rows: describeTerms(parsed.data.terms, tokens), title: 'Approve a standing permission', ...(never ? { caution: NO_EXPIRY } : {}) };
        }
        case SolanaPullInstruction.Cancel:
            return some(parsed.data.salt) === null
                ? { rows: [['Permission', short(parsed.accounts.account.address)]], title: 'Cancel one standing permission' }
                : { rows: [['Number', `${some(parsed.data.salt)}`]], title: 'Cancel one payment you signed' };
        case SolanaPullInstruction.Invalidate: {
            const what = [parsed.data.what & 1 ? 'every standing permission' : '', parsed.data.what & 2 ? 'every payment you signed' : ''].filter(Boolean);
            return { caution: 'This cannot be undone. Subscriptions you still want must be approved again.', rows: [], title: `End ${what.join(' and ')}, all at once` };
        }
        case SolanaPullInstruction.Close:
            return { rows: [['Account', short(parsed.accounts.account.address)]], title: 'Close an ended permission and return its rent' };
        case SolanaPullInstruction.Pull:
            return { rows: [['Permission', short(parsed.accounts.policy.address)], ['Amount', amount(parsed.data.amount, parsed.accounts.mint.address, tokens)]], title: 'Take a payment under a standing permission' };
        case SolanaPullInstruction.Fill:
            return { rows: [['Amount', amount(parsed.data.amount, parsed.accounts.mint.address, tokens)], ...describeTerms(parsed.data.terms, tokens)], title: 'Run a signed payment' };
        default:
            return null;
    }
}

/**
 * What a message the owner is asked to sign approves, if it is an intent of
 * the Pull program: null for any other message, and for any text that is not
 * exactly an intent's. `nonceIndex` is the one the text was written for; it
 * only works while it is the owner's profile's. A wallet that knows a mint's
 * decimals should refuse a text that shows other ones.
 */
export function describeIntent(text: string, tokens: Tokens = {}): (Approval & { terms: Terms; nonceIndex: bigint }) | null {
    const intent = parseIntent(text);
    if (!intent) return null;
    for (const [mint, decimals] of Object.entries(intent.decimals)) {
        if (tokens[mint] && tokens[mint].decimals !== decimals) return null;
    }
    const never = some(intent.terms.notAfter) === null;
    return { nonceIndex: intent.nonceIndex, rows: describeTerms(intent.terms, tokens), terms: intent.terms, title: 'Approve one payment', ...(never ? { caution: NO_EXPIRY } : {}) };
}
