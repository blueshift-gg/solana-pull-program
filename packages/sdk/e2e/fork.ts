// What the journeys share: a Surfpool fork of mainnet with the program at
// its address, funded wallets, and a way to send and to check.
import { spawn } from 'node:child_process';

import {
    type Address,
    address,
    appendTransactionMessageInstructions,
    createSolanaRpc,
    createTransactionMessage,
    generateKeyPairSigner,
    getAddressEncoder,
    getBase64EncodedWireTransaction,
    getProgramDerivedAddress,
    getSignatureFromTransaction,
    type Instruction,
    type KeyPairSigner,
    pipe,
    setTransactionMessageFeePayerSigner,
    setTransactionMessageLifetimeUsingBlockhash,
    signTransactionMessageWithSigners,
    type TransactionSigner,
} from '@solana/kit';

import { getSolanaPullErrorMessage, parseEvents, SOLANA_PULL_PROGRAM_ADDRESS, type SolanaPullError } from '../src/index.ts';

const RPC_URL = 'http://127.0.0.1:8899';
export const USDC = address('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');
export const WSOL = address('So11111111111111111111111111111111111111112');
export const TOKENS = { [USDC]: { decimals: 6, symbol: 'USDC' }, [WSOL]: { decimals: 9, symbol: 'SOL' } };
const TOKEN_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');
export const rpc = createSolanaRpc(RPC_URL);

async function call(method: string, params: unknown[]) {
    const res = await fetch(RPC_URL, { body: JSON.stringify({ id: 1, jsonrpc: '2.0', method, params }), headers: { 'content-type': 'application/json' }, method: 'POST' });
    const json = await res.json();
    if (json.error) throw new Error(json.error.message);
    return json.result;
}

/** Start Surfpool and wait until the program is installed. Returns how to stop it. */
export async function fork() {
    const surfpool = spawn('surfpool', ['start', '--no-tui', '--yes', '--runbook', 'surfnet-setup'], { cwd: new URL('../../..', import.meta.url).pathname, stdio: 'ignore' });
    for (let i = 0; ; i++) {
        const installed = await call('getAccountInfo', [SOLANA_PULL_PROGRAM_ADDRESS, { encoding: 'base64' }]).then((r) => r?.value?.executable === true, () => false);
        if (installed) return () => surfpool.kill();
        if (i === 120) throw new Error('Surfpool did not install the program within 60 s; see .surfpool/logs');
        await new Promise((r) => setTimeout(r, 500));
    }
}

export const tokenAccount = async (owner: Address, mint: Address = USDC) =>
    (await getProgramDerivedAddress({ programAddress: address('ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJA8knL'), seeds: [owner, TOKEN_PROGRAM, mint].map((a) => getAddressEncoder().encode(a)) }))[0];

/** A new wallet with SOL for fees and the token balances given, in base units. */
export async function wallet(balances: Partial<Record<Address, number>> = {}): Promise<KeyPairSigner> {
    const signer = await generateKeyPairSigner();
    await call('requestAirdrop', [signer.address, 5_000_000_000]);
    for (const [mint, amount] of Object.entries(balances)) await call('surfnet_setTokenAccount', [signer.address, mint, { amount }, TOKEN_PROGRAM]);
    return signer;
}

export const now = async () => {
    const { value } = await rpc.getAccountInfo(address('SysvarC1ock11111111111111111111111111111111'), { encoding: 'base64' }).send();
    return Buffer.from(value!.data[0], 'base64').readBigInt64LE(32);
};

/**
 * Send one transaction. A refusal by the program comes back as its error; a
 * success as the events the program emitted.
 */
export async function send(payer: TransactionSigner, instructions: Instruction[]) {
    const { value: blockhash } = await rpc.getLatestBlockhash().send();
    const message = pipe(
        createTransactionMessage({ version: 0 }),
        (m) => setTransactionMessageFeePayerSigner(payer, m),
        (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
        (m) => appendTransactionMessageInstructions(instructions, m),
    );
    const wire = getBase64EncodedWireTransaction(await signTransactionMessageWithSigners(message));
    const simulated = await rpc.simulateTransaction(wire, { encoding: 'base64', sigVerify: false }).send();
    if (simulated.value.err) {
        const text = JSON.stringify(simulated.value.err, (_, v) => (typeof v === 'bigint' ? Number(v) : v));
        const code = text.match(/"Custom":(\d+)/)?.[1];
        return { code: code ? Number(code) : undefined, error: code ? getSolanaPullErrorMessage(Number(code) as SolanaPullError) : text, events: [] };
    }
    const signature = getSignatureFromTransaction(await signTransactionMessageWithSigners(message));
    await rpc.sendTransaction(wire, { encoding: 'base64' }).send();
    for (let i = 0; i < 60; i++) {
        const transaction = await rpc.getTransaction(signature, { encoding: 'json', maxSupportedTransactionVersion: 0 }).send();
        if (transaction) return { events: parseEvents(transaction as unknown as Parameters<typeof parseEvents>[0]), units: Number(simulated.value.unitsConsumed) };
        await new Promise((r) => setTimeout(r, 250));
    }
    throw new Error(`transaction ${signature} did not confirm`);
}

export function check(ok: unknown, what: string) {
    if (!ok) throw new Error(`FAILED: ${what}`);
    console.log(`  ok  ${what}`);
}

/** `send` must be refused, with this error of the program. */
export async function refused(sending: ReturnType<typeof send>, error: SolanaPullError, what: string) {
    const result = (await sending) as { code?: number; error?: string };
    check(result.code === error, `${what}: refused${result.code === error ? '' : ` — but ${result.error ?? 'it went through'}`}`);
}
