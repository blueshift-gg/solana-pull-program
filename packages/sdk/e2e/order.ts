// A swap order: the owner signs text and sends nothing; a solver fills it
// and pays what the owner must receive in the same instruction.
import { signBytes } from '@solana/kit';

import { check, now, refused, rpc, send, TOKENS, tokenAccount, USDC, wallet, WSOL } from './fork.ts';
import {
    cancelIntentInstruction,
    describeIntent,
    enableInstruction,
    Envelope,
    fetchIntentUsed,
    fetchProfile,
    fillInstruction,
    findProfilePda,
    getOpenInstructionAsync,
    intentMessage,
    nextSalt,
    renderIntent,
    SOLANA_PULL_ERROR__NONCE_USED,
    terms,
} from '../src/index.ts';

export async function order() {
    const [owner, solver, poor] = await Promise.all([wallet({ [USDC]: 100_000_000, [WSOL]: 0 }), wallet({ [USDC]: 0, [WSOL]: 2_000_000_000 }), wallet({ [USDC]: 0, [WSOL]: 100 })]);
    const [usdc, sol] = [await tokenAccount(owner.address), await tokenAccount(owner.address, WSOL)];
    const decimals = { [USDC]: 6, [WSOL]: 9 };

    // Once per token: enable it and open the profile. Every order after is only a signature
    await send(owner, [enableInstruction({ account: usdc, owner }), await getOpenInstructionAsync({ authority: owner, payer: owner })]);
    const [profile] = await findProfilePda({ authority: owner.address });
    const { nonceIndex } = (await fetchProfile(rpc, profile)).data;

    // Sell 100 USDC for at least 0.52 SOL, to anyone, for an hour
    const start = await now();
    const sell = async () =>
        terms({
            authority: owner.address,
            cluster: 3,
            limit: { from: usdc, max: 100_000_000n, mint: USDC, per: { __kind: 'Total' }, to: null },
            notAfter: start + 3_600n,
            notBefore: start,
            receive: { decay: null, min: 520_000_000n, mint: WSOL, to: sol },
            salt: await nextSalt(rpc, owner.address, nonceIndex),
            spender: null,
        });
    const first = await sell();

    // The wallet is handed text to sign as a message, as every wallet can.
    // It recognises an intent and shows what it approves
    const text = renderIntent(first, nonceIndex, decimals);
    const approval = describeIntent(text, TOKENS)!;
    check(approval.title === 'Approve one payment' && approval.nonceIndex === nonceIndex, 'the wallet reads the text back as an intent');
    check(approval.rows.some(([label, value]) => label === 'Only if you receive' && value === 'At least 0.52 SOL each time'), 'and shows "At least 0.52 SOL each time"');
    check(describeIntent(text.replace('0.520000000', '0.52'), TOKENS) === null && describeIntent('gm', TOKENS) === null, 'a text that is not exactly an intent is not shown as one');
    const signature = new Uint8Array(await signBytes(owner.keyPair.privateKey, intentMessage(first, nonceIndex, decimals, Envelope.Text)));

    // A solver without the SOL cannot fill it; one with it does, in one instruction
    const fill = async (who: typeof solver) =>
        send(who, [await fillInstruction({ envelope: Envelope.Text, amount: 100_000_000n, nonceIndex, payFrom: await tokenAccount(who.address, WSOL), payer: who, signature, spender: who, terms: first, to: await tokenAccount(who.address) })]);
    check((await fill(poor)).error, 'a solver that cannot pay is refused');
    const filled = await fill(solver);
    const [event] = filled.events;
    check(event.type === 'filled' && event.amount === 100_000_000n && event.paid === 520_000_000n, `a solver fills it: 100 USDC out, 0.52 SOL in (${filled.units} CU)`);
    await refused(fill(solver), SOLANA_PULL_ERROR__NONCE_USED, 'a second fill');
    check(await fetchIntentUsed(rpc, first, nonceIndex), 'the intent reads as used');

    // The next order takes the next salt; the owner cancels it before anyone fills it
    const second = await sell();
    check(first.salt === 0n && second.salt === 1n, 'salts go 0, 1: one page of nonces serves 1,024 orders');
    await send(owner, [await cancelIntentInstruction({ authority: owner, nonceIndex, payer: owner, salt: second.salt })]);
    check(await fetchIntentUsed(rpc, second, nonceIndex), 'a cancelled order reads as used');
}
