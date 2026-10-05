// Fathom's backend, as a publisher would run it: it holds the merchant key,
// keeps the member list, charges each member when their period is over, and
// serves reports to paying members only. Every charge is one Pull the
// Pull program decides; the server cannot take more than a member approved.
import fs from 'node:fs';
import type { IncomingMessage, ServerResponse } from 'node:http';
import path from 'node:path';

import { decode, fetchPolicy, getPullInstruction, loadWasm, programError } from '@solana-pull/sdk';
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
    pipe,
    setTransactionMessageFeePayerSigner,
    setTransactionMessageLifetimeUsingBlockhash,
    signTransactionMessageWithSigners,
    SOLANA_ERROR__INSTRUCTION_ERROR__CUSTOM,
} from '@solana/kit';
import { fetchSysvarClock } from '@solana/sysvars';
import { findAssociatedTokenPda, TOKEN_PROGRAM_ADDRESS } from '@solana-program/token';
import type { Plugin } from 'vite';

import { REPORTS } from './reports.ts';

const RPC = 'http://127.0.0.1:8899';
const rpc = createSolanaRpc(RPC);
const USDC = address('EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v');
const DAY = 86_400;

const PLANS = [
    { id: 'reader', name: 'Reader', period: 30 * DAY, price: 8_000_000n, perks: ['Every weekly report', 'The full archive'] },
    { id: 'desk', name: 'Desk', period: 30 * DAY, price: 20_000_000n, perks: ['Everything in Reader', 'Data behind every chart', 'Analyst office hours'] },
];

type Payment = { amount: string; at: number; signature: string };
type Member = {
    address: Address;
    plan: string;
    /** Canonical terms of the policy the member created on chain. */
    terms: Uint8Array;
    since: number;
    paidThrough: number;
    status: 'active' | 'past_due' | 'cancelled';
    reason?: string;
    payments: Payment[];
};

async function cheat(method: string, params: unknown[]) {
    const res = await fetch(RPC, { body: JSON.stringify({ id: 1, jsonrpc: '2.0', method, params }), headers: { 'content-type': 'application/json' }, method: 'POST' });
    const json = await res.json();
    if (json.error) throw new Error(json.error.message);
    return json.result;
}

const clock = async () => Number((await fetchSysvarClock(rpc)).unixTimestamp);
const bytes = (b64: string) => new Uint8Array(getBase64Encoder().encode(b64));
const b64 = (b: Uint8Array) => Buffer.from(b).toString('base64');

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

export function fathom(): Plugin {
    const members = new Map<Address, Member>();
    const ready = (async () => {
        await loadWasm(fs.readFileSync(path.resolve('../../packages/sdk/wasm/pull_bg.wasm')));
        const signer = await generateKeyPairSigner();
        await cheat('requestAirdrop', [signer.address, 5_000_000_000]);
        await cheat('surfnet_setTokenAccount', [signer.address, USDC, { amount: 0 }, TOKEN_PROGRAM_ADDRESS]);
        const [usdc] = await findAssociatedTokenPda({ mint: USDC, owner: signer.address, tokenProgram: TOKEN_PROGRAM_ADDRESS });
        return { signer, usdc };
    })();

    /** The approval must be exactly the plan: its price, its period, spendable only by Fathom. */
    async function check(member: Pick<Member, 'address' | 'plan' | 'terms'>) {
        const { signer } = await ready;
        const plan = PLANS.find((p) => p.id === member.plan);
        const t = decode(member.terms);
        const { limit } = t;
        const period = typeof limit.per === 'object' ? limit.per.every : 0;
        if (!plan || t.authority !== member.address || t.receive) return 'This approval is not for a Fathom plan';
        if (limit.mint !== USDC || BigInt(limit.max) !== plan.price || period !== plan.period) return 'This approval does not match the plan';
        if (t.spender !== signer.address) return 'This approval does not name Fathom as its spender';
        return null;
    }

    /** Collect one period's price. The program decides; a refusal comes back by name. */
    async function charge(member: Member) {
        const { signer, usdc } = await ready;
        const plan = PLANS.find((p) => p.id === member.plan)!;
        const instructions = [await getPullInstruction({ amount: plan.price, spender: signer, terms: member.terms, to: usdc })];
        const { value: blockhash } = await rpc.getLatestBlockhash().send();
        const tx = await signTransactionMessageWithSigners(
            pipe(
                createTransactionMessage({ version: 0 }),
                (m) => setTransactionMessageFeePayerSigner(signer, m),
                (m) => setTransactionMessageLifetimeUsingBlockhash(blockhash, m),
                (m) => appendTransactionMessageInstructions(instructions, m),
            ),
        );
        try {
            const signature = await rpc.sendTransaction(getBase64EncodedWireTransaction(tx), { encoding: 'base64' }).send();
            for (let i = 0; i < 60; i++) {
                const [status] = (await rpc.getSignatureStatuses([signature]).send()).value;
                if (status?.confirmationStatus) return status.err ? { ok: false as const, reason: 'Refused' } : { ok: true as const, signature };
                await new Promise((r) => setTimeout(r, 150));
            }
            return { ok: false as const, reason: 'NotConfirmed' };
        } catch (error) {
            for (let e: unknown = error; e; e = (e as { cause?: unknown }).cause) {
                if (isSolanaError(e, SOLANA_ERROR__INSTRUCTION_ERROR__CUSTOM)) return { ok: false as const, reason: programError(Number(e.context.code)) ?? 'InsufficientFunds' };
            }
            return { ok: false as const, reason: error instanceof Error ? error.message : String(error) };
        }
    }

    /** Whether the member withdrew the approval on chain: the chain, not our database, is the record. */
    async function withdrawn(member: Member) {
        // Cancelling closes the policy
        return (await fetchPolicy(rpc, member.terms, await clock())) === null;
    }

    /** Bring one member up to date: notice a cancellation, charge a period that is due. */
    async function settle(member: Member, now: number) {
        if (member.status === 'cancelled') return;
        if (await withdrawn(member)) {
            member.status = 'cancelled';
            return;
        }
        if (now < member.paidThrough) return;
        const result = await charge(member);
        if (result.ok) {
            const plan = PLANS.find((p) => p.id === member.plan)!;
            member.payments.unshift({ amount: plan.price.toString(), at: now, signature: result.signature });
            member.paidThrough = now + plan.period;
            member.status = 'active';
            delete member.reason;
        } else {
            member.status = 'past_due';
            member.reason = result.reason;
        }
    }

    // One chain operation at a time: the billing run and requests share the merchant key
    let queue: Promise<unknown> = Promise.resolve();
    const serial = <T>(run: () => Promise<T>) => {
        const next = queue.then(run);
        queue = next.catch(() => {});
        return next;
    };

    const view = (m: Member) => ({
        paidThrough: m.paidThrough,
        payments: m.payments,
        plan: m.plan,
        reason: m.reason,
        since: m.since,
        status: m.status,
        terms: b64(m.terms),
    });

    return {
        configureServer(server) {
            // The billing run: what a cron job does in production
            const billing = setInterval(() => serial(async () => {
                const now = await clock();
                for (const member of members.values()) await settle(member, now);
            }).catch(() => {}), 4000);
            server.httpServer?.on('close', () => clearInterval(billing));

            server.middlewares.use('/api', async (req, res) => {
                const { signer, usdc } = await ready;
                const url = new URL(req.url ?? '/', 'http://x');
                const who = url.searchParams.get('address') as Address | null;
                try {
                    switch (url.pathname) {
                        case '/config':
                            return reply(res, 200, { clock: await clock(), merchant: signer.address, merchantUsdc: usdc, plans: PLANS.map((p) => ({ ...p, price: p.price.toString() })) });
                        case '/subscribe': {
                            const input = await body(req);
                            const now = await clock();
                            const member: Member = {
                                address: input.address,
                                paidThrough: now,
                                payments: [],
                                plan: input.plan,
                                since: now,
                                status: 'active',
                                terms: bytes(input.terms),
                            };
                            const problem = await check(member);
                            if (problem) return reply(res, 400, { error: problem });
                            // The first period is charged now; membership starts only if it clears
                            await serial(() => settle(member, now));
                            if (member.status !== 'active') return reply(res, 402, { error: member.reason ?? 'The first payment did not go through' });
                            members.set(member.address, member);
                            return reply(res, 200, view(member));
                        }
                        case '/account': {
                            const member = who && members.get(who);
                            if (!member) return reply(res, 200, null);
                            await serial(async () => settle(member, await clock()));
                            return reply(res, 200, view(member));
                        }
                        case '/reports': {
                            const member = who && members.get(who);
                            const paid = !!member && member.paidThrough > (await clock());
                            return reply(res, 200, REPORTS.map((r) => (r.free || paid ? r : { ...r, body: null })));
                        }
                        // Local fork only: test funds and the calendar
                        case '/dev/fund': {
                            // Top up only: a forked wallet keeps whatever it already holds
                            const { address: to } = await body(req);
                            const [token] = await findAssociatedTokenPda({ mint: USDC, owner: to, tokenProgram: TOKEN_PROGRAM_ADDRESS });
                            const held = await rpc.getTokenAccountBalance(token).send().then((b) => BigInt(b.value.amount), () => 0n);
                            if ((await rpc.getBalance(to).send()).value < 100_000_000n) await cheat('requestAirdrop', [to, 1_000_000_000]);
                            if (held < 50_000_000n) await cheat('surfnet_setTokenAccount', [to, USDC, { amount: 50_000_000 }, TOKEN_PROGRAM_ADDRESS]);
                            return reply(res, 200, {});
                        }
                        case '/dev/skip': {
                            const { days } = await body(req);
                            await cheat('surfnet_timeTravel', [{ absoluteTimestamp: ((await clock()) + days * DAY) * 1000 }]);
                            return reply(res, 200, {});
                        }
                        default:
                            return reply(res, 404, { error: 'Not found' });
                    }
                } catch (error) {
                    return reply(res, 500, { error: error instanceof Error ? error.message : String(error) });
                }
            });
        },
        name: 'fathom',
    };
}
