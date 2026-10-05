// Dev only: a Wallet Standard wallet that lives in this page, so the site runs
// without an extension. It is what a wallet looks like once it implements
// `solana:signIntent` (packages/wallet-standard): the site sends terms, and
// the wallet decodes them, renders the canonical text itself and asks.
// The key is in localStorage: never use it for real funds.
import { decode, PULL_PROGRAM_ADDRESS, message, text } from '@solana-pull/sdk';
import {
    type Address,
    createKeyPairSignerFromPrivateKeyBytes,
    createSolanaRpc,
    getAddressEncoder,
    getCompiledTransactionMessageDecoder,
    getTransactionDecoder,
    getTransactionEncoder,
    partiallySignTransaction,
    signBytes,
} from '@solana/kit';
import { fetchMint, TOKEN_PROGRAM_ADDRESS } from '@solana-program/token';

const rpc = createSolanaRpc(`${location.origin}/rpc`);
const stored = localStorage.getItem('demo-wallet');
const seed = stored ? Uint8Array.from(stored.match(/../g)!, (h) => parseInt(h, 16)) : crypto.getRandomValues(new Uint8Array(32));
localStorage.setItem('demo-wallet', Array.from(seed, (b) => b.toString(16).padStart(2, '0')).join(''));
const signer = await createKeyPairSignerFromPrivateKeyBytes(seed);

/** The wallet's own token list: names come from the wallet, never from the site. */
const TOKENS: Record<string, string> = { EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v: 'USDC' };
const short = (a: string) => `${a.slice(0, 4)}…${a.slice(-4)}`;
const amount = (raw: string, decimals: number) => (Number(raw) / 10 ** decimals).toLocaleString('en-US', { maximumFractionDigits: decimals });
const span = (seconds: number) => (seconds % 86_400 === 0 ? `${seconds / 86_400} days` : `${seconds} seconds`);
const date = (unix: number) => new Date(unix * 1000).toLocaleDateString('en-US', { day: 'numeric', month: 'short', year: 'numeric' });
const esc = (s: string) => s.replace(/[&<>]/g, (c) => ({ '&': '&amp;', '<': '&lt;', '>': '&gt;' })[c]!);

const STYLE = `
:host { all: initial; }
.veil { position: fixed; inset: 0; background: rgba(8, 10, 14, .55); display: grid; place-items: end center; z-index: 9999; font: 14px/1.5 system-ui, sans-serif; color: #e9ecf1; }
@media (min-width: 640px) { .veil { place-items: center end; padding-right: 28px; } }
.sheet { width: min(380px, 100vw); background: #15181e; border: 1px solid #2a2f38; border-radius: 18px; padding: 18px; display: grid; gap: 14px; box-shadow: 0 24px 60px rgba(0, 0, 0, .5); }
.from { display: flex; align-items: center; gap: 8px; font-size: 12px; color: #98a1ae; }
.from b { color: #e9ecf1; font-weight: 500; }
.dot { width: 18px; height: 18px; border-radius: 6px; background: #7c5cff; }
h3 { margin: 0; font-size: 17px; font-weight: 600; }
dl { margin: 0; display: grid; gap: 1px; border-radius: 12px; overflow: hidden; }
dl div { background: #1c2028; padding: 10px 12px; display: grid; gap: 2px; }
dt { font-size: 11px; letter-spacing: .06em; text-transform: uppercase; color: #98a1ae; }
dd { margin: 0; }
.note { font-size: 12.5px; color: #98a1ae; margin: 0; }
details { font-size: 12px; color: #98a1ae; }
pre { margin: 8px 0 0; max-height: 180px; overflow: auto; background: #0e1014; border-radius: 10px; padding: 10px; font: 10.5px/1.5 ui-monospace, monospace; white-space: pre-wrap; overflow-wrap: anywhere; color: #c5cbd5; }
label { display: flex; gap: 8px; font-size: 12.5px; color: #f0b56a; }
.row { display: grid; grid-template-columns: 1fr 1fr; gap: 8px; }
button { font: 500 14px system-ui, sans-serif; border-radius: 12px; padding: 11px; border: 1px solid #2a2f38; background: #1c2028; color: #e9ecf1; cursor: pointer; }
button.go { background: #7c5cff; border-color: #7c5cff; color: #fff; }
button:disabled { opacity: .4; cursor: default; }
`;

/** Show an approval sheet; resolves once the user approves and rejects if they decline. */
function ask(title: string, rows: [string, string][], details: string, mustAcknowledge?: string) {
    return new Promise<void>((resolve, reject) => {
        const host = document.createElement('div');
        const root = host.attachShadow({ mode: 'open' });
        root.innerHTML = `<style>${STYLE}</style>
<div class="veil"><div class="sheet" role="dialog" aria-label="Demo Wallet">
  <div class="from"><span class="dot"></span><span>Demo Wallet · <b>${esc(location.host)}</b> asks</span></div>
  <h3>${esc(title)}</h3>
  <dl>${rows.map(([k, v]) => `<div><dt>${esc(k)}</dt><dd>${esc(v)}</dd></div>`).join('')}</dl>
  <p class="note">Signing as ${esc(short(signer.address))}. The network enforces exactly what is shown.</p>
  <details><summary>Exactly what you sign</summary><pre>${esc(details)}</pre></details>
  ${mustAcknowledge ? `<label><input type="checkbox" id="ack"> ${esc(mustAcknowledge)}</label>` : ''}
  <div class="row"><button id="no">Reject</button><button id="yes" class="go" ${mustAcknowledge ? 'disabled' : ''}>Approve</button></div>
</div></div>`;
        const done = (ok: boolean) => {
            host.remove();
            if (ok) resolve();
            else reject(new Error('You rejected the request'));
        };
        const yes = root.getElementById('yes') as HTMLButtonElement;
        root.getElementById('ack')?.addEventListener('change', (e) => (yes.disabled = !(e.target as HTMLInputElement).checked));
        yes.addEventListener('click', () => done(true));
        root.getElementById('no')!.addEventListener('click', () => done(false));
        document.body.append(host);
    });
}

/** The approval a policy grants, in the wallet's words, and its canonical text. */
async function describe(terms: Uint8Array) {
    const t = decode(terms);
    const decimals: Record<string, number> = {};
    for (const mint of [t.limit.mint, ...(t.receive ? [t.receive.mint] : [])]) decimals[mint] ??= (await fetchMint(rpc, mint)).data.decimals;
    const token = (mint: Address) => TOKENS[mint] ?? short(mint);
    const rows: [string, string][] = [];
    const per = typeof t.limit.per === 'object' ? ` every ${span(t.limit.per.every)}` : ' in total';
    rows.push(['Can take', `Up to ${amount(t.limit.max, decimals[t.limit.mint])} ${token(t.limit.mint)}${per}`]);
    if (t.receive) rows.push(['Only if', `You receive at least ${amount(t.receive.min, decimals[t.receive.mint])} ${token(t.receive.mint)} each time`]);
    rows.push(['Who can take it', t.spender ? short(t.spender) : 'Anyone who delivers that']);
    rows.push(['Ends', t.notAfter ? date(t.notAfter) : 'Never, until you revoke it']);
    return { decimals, never: !t.notAfter, rows, text: text(terms, decimals) };
}

/** What a transaction does, by instruction. A policy created by transaction is shown like a signed one. */
async function summarize(transaction: Uint8Array) {
    const compiled = getCompiledTransactionMessageDecoder().decode(getTransactionDecoder().decode(transaction).messageBytes);
    const rows: [string, string][] = [];
    let details = '';
    // The site sends version 0; anything else is shown without a summary
    if (compiled.version === 1) return { details: 'A version 1 transaction.', rows: [['Run', 'A transaction this wallet cannot summarize']] as [string, string][] };
    for (const ix of compiled.instructions) {
        const program = compiled.staticAccounts[ix.programAddressIndex];
        const data = ix.data ?? new Uint8Array();
        if (program === TOKEN_PROGRAM_ADDRESS && data[0] === 4) {
            rows.push(['Turn on', 'Pull payments for this token. Each approval still sets its own limit']);
        } else if (program === TOKEN_PROGRAM_ADDRESS && data[0] === 5) {
            rows.push(['Turn off', 'Pull payments for this token: every approval stops']);
        } else if (program === PULL_PROGRAM_ADDRESS && data[0] === 0) {
            // discriminator, terms_len: u16, terms
            const approval = await describe(new Uint8Array(data.slice(3, 3 + data[1] + data[2] * 256)));
            rows.push(['Approve', 'On chain, with this transaction'], ...approval.rows);
            details += approval.text;
        } else if (program === PULL_PROGRAM_ADDRESS && data[0] === 2) {
            rows.push(['Cancel', 'One approval. It can never be used again, and its rent goes back to whoever paid it']);
        } else {
            rows.push(['Run', `Program ${short(program)}`]);
        }
    }
    return { details: details || 'A transaction with the instructions listed above.', rows };
}

// Wallets that have not adopted the feature yet, to see the site's fallbacks:
// `?offchain` signs offchain messages as raw text, `?basic` only signs transactions.
const query = new URLSearchParams(location.search);
const mode = query.has('basic') ? 'basic' : query.has('offchain') ? 'offchain' : 'intent';
const SIGNING = { basic: [], intent: ['solana:signIntent'], offchain: ['solana:signOffchainMessage'] } as const;
const chains = ['solana:localnet', 'solana:mainnet'] as const;
const account = {
    address: signer.address,
    chains,
    features: ['solana:signTransaction', ...SIGNING[mode]],
    label: 'Demo',
    publicKey: getAddressEncoder().encode(signer.address),
};

const wallet = {
    accounts: [account],
    chains,
    features: {
        'solana:signIntent': {
            signIntent: (...inputs: { terms: Uint8Array }[]) =>
                Promise.all(
                    inputs.map(async ({ terms }) => {
                        const bytes = new Uint8Array(terms);
                        const approval = await describe(bytes);
                        if (decode(bytes).authority !== signer.address) throw new Error('This approval is for another account');
                        // An intent with no expiry stays usable until it runs or is cancelled
                        await ask('Approve a payment', approval.rows, approval.text, approval.never ? 'I understand this approval does not expire' : undefined);
                        const signedOffchainMessage = message(bytes, approval.decimals);
                        return { signature: await signBytes(signer.keyPair.privateKey, signedOffchainMessage), signatureType: 'ed25519', signedOffchainMessage };
                    }),
                ),
            supportedTermsVersions: [1],
            version: '1.0.0',
        },
        'solana:signOffchainMessage': {
            // A wallet that knows nothing about intents: it can only show the text it was given
            signOffchainMessage: (...inputs: { message: string }[]) =>
                Promise.all(
                    inputs.map(async ({ message: body }) => {
                        await ask('Sign a message', [['Message', 'This wallet cannot read this message for you. Check the text below']], body);
                        const signedOffchainMessage = Uint8Array.from([0xff, ...new TextEncoder().encode('solana offchain'), 1, 1, ...account.publicKey, ...new TextEncoder().encode(body)]);
                        return { signature: await signBytes(signer.keyPair.privateKey, signedOffchainMessage), signatureType: 'ed25519', signedOffchainMessage };
                    }),
                ),
            supportedMessageVersions: [1],
            version: '1.0.0',
        },
        'solana:signTransaction': {
            signTransaction: (...inputs: { transaction: Uint8Array }[]) =>
                Promise.all(
                    inputs.map(async ({ transaction }) => {
                        const { rows, details } = await summarize(transaction);
                        await ask('Approve a transaction', rows, details);
                        const signed = await partiallySignTransaction([signer.keyPair], getTransactionDecoder().decode(transaction));
                        return { signedTransaction: getTransactionEncoder().encode(signed) };
                    }),
                ),
            supportedTransactionVersions: ['legacy', 0],
            version: '1.0.0',
        },
        'standard:connect': { connect: async () => ({ accounts: [account] }), version: '1.0.0' },
        'standard:events': { on: () => () => {}, version: '1.0.0' },
    },
    icon: 'data:image/svg+xml;base64,' + btoa('<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32"><rect width="32" height="32" rx="9" fill="#7c5cff"/><path d="M9 12h14v10H9z" fill="none" stroke="#fff" stroke-width="2" stroke-linejoin="round"/><circle cx="19.5" cy="17" r="1.5" fill="#fff"/></svg>'),
    name: mode === 'intent' ? 'Demo Wallet' : `Demo Wallet (${mode})`,
    version: '1.0.0',
};

for (const feature of ['solana:signIntent', 'solana:signOffchainMessage'] as const) {
    if (!account.features.includes(feature)) delete (wallet.features as Partial<typeof wallet.features>)[feature];
}

const register = ({ register }: { register: (w: typeof wallet) => void }) => register(wallet);
window.dispatchEvent(new CustomEvent('wallet-standard:register-wallet', { detail: register }));
window.addEventListener('wallet-standard:app-ready', (e) => register((e as CustomEvent).detail));
