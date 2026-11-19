// A swap order: the owner creates a mandate that anyone may fill once they
// pay what the owner must receive, in the same instruction.
import { check, now, refused, rpc, send, tokenAccount, USDC, wallet, WSOL } from './fork.ts';
import { closeMandateInstruction, fetchMandateStatus, onboardInstructions, pullInstruction, SOLANA_PULL_ERROR__LIMIT_EXCEEDED } from '../src/index.ts';

export async function order() {
    const [owner, solver, poor] = await Promise.all([wallet({ [USDC]: 100_000_000, [WSOL]: 0 }), wallet({ [USDC]: 0, [WSOL]: 2_000_000_000 }), wallet({ [USDC]: 0, [WSOL]: 100 })]);
    const [usdc, sol] = [await tokenAccount(owner.address), await tokenAccount(owner.address, WSOL)];

    // Sell 100 USDC for at least 0.52 SOL, to anyone, for an hour: one transaction
    const start = await now();
    const sell = { out: { from: usdc, max: 100_000_000n, period: null, maxPerPull: null, to: null }, notAfter: start + 3_600n, notBefore: start, in: { min: 520_000_000n, to: sol }, spender: null };
    const { instructions, mandate } = await onboardInstructions(rpc, { authority: owner, payer: owner, terms: sell });
    await send(owner, instructions);
    const status = await fetchMandateStatus(rpc, mandate);
    check(status.state === 'active' && status.available === 100_000_000n, 'status: active, 100 USDC');

    // A solver without the SOL cannot fill it; one with it does, in one instruction
    const fill = async (who: typeof solver) =>
        send(who, [await pullInstruction(rpc, { amount: 100_000_000n, mandate: status.mandate!, payFrom: await tokenAccount(who.address, WSOL), spender: who, to: await tokenAccount(who.address) })]);
    check((await fill(poor)).error, 'a solver that cannot pay is refused');
    const filled = await fill(solver);
    const [event] = filled.events;
    check(event.type === 'pulled' && event.amount === 100_000_000n && event.paid === 520_000_000n, `a solver fills it: 100 USDC out, 0.52 SOL in (${filled.units} CU)`);
    await refused(fill(solver), SOLANA_PULL_ERROR__LIMIT_EXCEEDED, 'a second fill');

    // It is used up, so anyone may close it; the rent goes back to the owner, who paid it
    const closed = await send(solver, [await closeMandateInstruction({ closer: solver, mandate: status.mandate! })]);
    check(closed.events[0].type === 'closed' && (await fetchMandateStatus(rpc, mandate)).state === 'missing', 'the solver closes the filled order');
}
