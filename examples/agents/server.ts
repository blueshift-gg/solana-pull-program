// The paid API, as its provider would run it: "Inference API" sells on-chain
// analytics per request over HTTP 402. An agent pays with the budget its
// owner approved on chain; the server checks the budget pays this API, settles the price
// on chain with the Pull program, then serves the data. Real mainnet data,
// read from the Surfpool fork.
//
// Mallory runs a second server that tries to collect with the same budget.
import fs from 'node:fs';
import os from 'node:os';
import type { IncomingMessage, ServerResponse } from 'node:http';
import path from 'node:path';

import { decode, getPullInstruction, loadWasm, programError } from '@solana-pull/sdk';
import {
    address,
    type Address,
    appendTransactionMessageInstructions,
    createSolanaRpc,
    createTransactionMessage,
    generateKeyPairSigner,
    getBase64EncodedWireTransaction,
    getBase64Encoder,
    isSolanaError,
    type KeyPairSigner,
    pipe,
    setTransactionMessageFeePayerSigner,
    setTransactionMessageLifetimeUsingBlockhash,
    signTransactionMessageWithSigners,
    SOLANA_ERROR__INSTRUCTION_ERROR__CUSTOM,
} from '@solana/kit';
import { fetchSysvarClock } from '@solana/sysvars';
import { findAssociatedTokenPda, TOKEN_PROGRAM_ADDRESS } from '@solana-program/token';
import type { Plugin } from 'vite';

const RPC = 'http://127.0.0.1:8899';
const rpc = createSolanaRpc(RPC);
export const USDC = address('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');
export const PRICE = 50_000n; // $0.05 a request
export const PER_DAY = 1_000_000n; // the budget the agent asks for: $1 a day

/** Tokens the agent researches: their real mainnet supply comes from the fork. */
const TOKENS: Record<string, string> = {
    BONK: 'DezXAZ8z7PnrnRJjz3wXBoRgixCa6xjnB7YaB1pPB263',
    JUP: 'JUPyiwrYJFskUPiHa7hkeR8VUtAeFoSYbKedZNsDvCN',
    JTO: 'jtojtomepa8beP8AuQc6eXt5FriJwfFMwQx2v2f9mCL',
    PYTH: 'HZ1JovNiVvGrGNiiYvEozEVgZ58xaU3RKwX8eACQBCt3',
    USDC: 'EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v',
    WIF: 'EKpQGSJtjMFqKZ9KQanSqYXRcF8fBopzLHYxdM65zcjm',
};

async function cheat(method: string, params: unknown[]) {
    const res = await fetch(RPC, { body: JSON.stringify({ id: 1, jsonrpc: '2.0', method, params }), headers: { 'content-type': 'application/json' }, method: 'POST' });
    const json = await res.json();
    if (json.error) throw new Error(json.error.message);
    return json.result;
}

const ata = async (owner: Address) => (await findAssociatedTokenPda({ mint: USDC, owner, tokenProgram: TOKEN_PROGRAM_ADDRESS }))[0];

async function party(name: string) {
    const signer = await generateKeyPairSigner();
    await cheat('requestAirdrop', [signer.address, 2_000_000_000]);
    await cheat('surfnet_setTokenAccount', [signer.address, USDC, { amount: 0 }, TOKEN_PROGRAM_ADDRESS]);
    return { name, signer, usdc: await ata(signer.address) };
}
type Party = Awaited<ReturnType<typeof party>>;


let lastBlockhash = '';

/** Settle: pull `amount` from the payer to `to` under the budget Alice put on chain. The program decides. */
async function settle(executor: Party, budget: Budget, to: Address, amount: bigint) {
    const instructions = [await getPullInstruction({ amount, spender: executor.signer, terms: budget.terms, to })];
    // Each settlement must be a new transaction: wait for a fresh blockhash
    let { value: blockhash } = await rpc.getLatestBlockhash().send();
    while (blockhash.blockhash === lastBlockhash) {
        await new Promise((r) => setTimeout(r, 80));
        ({ value: blockhash } = await rpc.getLatestBlockhash().send());
    }
    lastBlockhash = blockhash.blockhash;
    const tx = await signTransactionMessageWithSigners(
        pipe(
            createTransactionMessage({ version: 0 }),
            (m) => setTransactionMessageFeePayerSigner(executor.signer as KeyPairSigner, m),
            (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
            (m) => appendTransactionMessageInstructions(instructions, m),
        ),
    );
    try {
        const signature = await rpc.sendTransaction(getBase64EncodedWireTransaction(tx), { encoding: 'base64' }).send();
        for (let i = 0; i < 50; i++) {
            const found = await rpc.getTransaction(signature, { commitment: 'confirmed', encoding: 'json', maxSupportedTransactionVersion: 0 }).send();
            if (found) return { computeUnits: Number(found.meta?.computeUnitsConsumed ?? 0), ok: true as const, signature };
            await new Promise((r) => setTimeout(r, 150));
        }
        return { ok: false as const, reason: 'NotConfirmed' };
    } catch (error) {
        for (let e: unknown = error; e; e = (e as { cause?: unknown }).cause) {
            if (isSolanaError(e, SOLANA_ERROR__INSTRUCTION_ERROR__CUSTOM)) return { ok: false as const, reason: programError(Number(e.context.code)) ?? 'Refused' };
        }
        return { ok: false as const, reason: error instanceof Error ? error.message : String(error) };
    }
}

type Budget = { terms: Uint8Array };
const bytes = (b64: string) => new Uint8Array(getBase64Encoder().encode(b64));

async function body(req: IncomingMessage) {
    let text = '';
    for await (const chunk of req) text += chunk;
    return text ? JSON.parse(text) : {};
}

function reply(res: ServerResponse, status: number, value: unknown) {
    res.statusCode = status;
    res.setHeader('content-type', 'application/json');
    res.end(JSON.stringify(value));
}

/** Where a phone on the same network reaches this server. */
const phoneUrl = () => {
    const ip = Object.values(os.networkInterfaces()).flat().find((i) => i?.family === 'IPv4' && !i.internal)?.address ?? 'localhost';
    return `http://${ip}:5173/phone`;
};

export function paidApi(): Plugin {
    const ready = (async () => {
        await loadWasm(fs.readFileSync(path.resolve('../../packages/sdk/wasm/pull_bg.wasm')));
        const [provider, mallory] = await Promise.all([party('Inference API'), party('Mallory')]);
        return { mallory, provider };
    })();
    // What the owner's phone has approved, shared with the screen
    const shared: { alice?: Address; budget?: { terms: string } } = {};

    return {
        configureServer(server) {
            server.middlewares.use('/api', async (req, res) => {
                const { provider, mallory } = await ready;
                const url = new URL(req.url ?? '/', 'http://x');
                try {
                    switch (url.pathname) {
                        case '/state':
                            return reply(res, 200, { ...shared, perDay: PER_DAY.toString(), price: PRICE.toString(), provider: provider.signer.address, providerUsdc: provider.usdc, mallory: mallory.signer.address, phoneUrl: phoneUrl(), clock: Number((await fetchSysvarClock(rpc)).unixTimestamp) });
                        case '/fund': {
                            // A new owner: SOL for fees and 25 real USDC on the fork
                            const { address: who } = await body(req);
                            await cheat('requestAirdrop', [who, 1_000_000_000]);
                            await cheat('surfnet_setTokenAccount', [who, USDC, { amount: 25_000_000 }, TOKEN_PROGRAM_ADDRESS]);
                            shared.alice = who;
                            delete shared.budget;
                            return reply(res, 200, {});
                        }
                        case '/budget':
                            shared.budget = await body(req);
                            return reply(res, 200, {});
                        case '/tomorrow':
                            await cheat('surfnet_timeTravel', [{ absoluteTimestamp: (Number((await fetchSysvarClock(rpc)).unixTimestamp) + 86_400) * 1000 }]);
                            return reply(res, 200, {});
                        case '/analytics': {
                            const symbol = url.searchParams.get('token') ?? 'JUP';
                            const requirement = { accepts: [{ amount: PRICE.toString(), asset: USDC, payTo: provider.usdc, resource: `/api/analytics?token=${symbol}`, scheme: 'pull' }], x402Version: 1 };
                            const header = req.headers['x-payment'];
                            if (typeof header !== 'string') return reply(res, 402, { ...requirement, error: 'Payment required' });

                            // Verify: the budget must pay this API, then let the program settle it
                            const payment = JSON.parse(Buffer.from(header, 'base64').toString());
                            const budget = { terms: bytes(payment.terms) };
                            if (decode(budget.terms).spender !== provider.signer.address) return reply(res, 402, { ...requirement, error: 'This budget does not name this API as its spender' });
                            const settled = await settle(provider, budget, provider.usdc, PRICE);
                            if (!settled.ok) return reply(res, 402, { ...requirement, error: settled.reason });

                            const supply = await rpc.getTokenSupply(address(TOKENS[symbol])).send();
                            return reply(res, 200, { payment: settled, symbol, supply: supply.value.uiAmountString });
                        }
                        case '/mallory': {
                            // A prompt-injected agent sends its payment header to Mallory, who tries to collect with it
                            const payment = await body(req);
                            const budget = { terms: bytes(payment.terms) };
                            const attempt = await settle(mallory, budget, mallory.usdc, PRICE * 2n);
                            return reply(res, attempt.ok ? 200 : 402, attempt);
                        }
                        default:
                            return reply(res, 404, { error: 'Not found' });
                    }
                } catch (error) {
                    return reply(res, 500, { error: error instanceof Error ? error.message : String(error) });
                }
            });
        },
        name: 'paid-api',
    };
}

