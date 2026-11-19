// Ending everything at once: one transaction ends every mandate, and anyone
// can then close what was ended, with the rent going back to whoever paid it.
import { check, now, refused, rpc, send, tokenAccount, USDC, wallet } from './fork.ts';
import {
    closeMandateInstruction,
    fetchMandates,
    fetchMandateStatus,
    fetchProfile,
    findProfilePda,
    getInvalidateInstructionAsync,
    onboardInstructions,
    pullInstruction,
    SOLANA_PULL_ERROR__NOT_CLOSABLE,
    SOLANA_PULL_ERROR__STALE,
} from '../src/index.ts';

export async function undo() {
    const [owner, merchant, sponsor, stranger] = await Promise.all([wallet({ [USDC]: 100_000_000 }), wallet({ [USDC]: 0 }), wallet(), wallet()]);
    const [account, treasury] = [await tokenAccount(owner.address), await tokenAccount(merchant.address)];
    const start = await now();
    const allow = (max: bigint) => ({ out: { from: account, max, period: null, to: null }, notAfter: null, notBefore: start, in: null, spender: merchant.address });

    // Three mandates, numbered as they are created
    for (const max of [1_000_000n, 2_000_000n, 3_000_000n]) await send(sponsor, (await onboardInstructions(rpc, { authority: owner, payer: sponsor, terms: allow(max) })).instructions);
    const [profile] = await findProfilePda({ authority: owner.address });
    const before = (await fetchProfile(rpc, profile)).data;
    check(before.mandates === 3n && before.stale === 0n, 'the profile counts three mandates, none stale');

    // One transaction ends every mandate
    const mandates = await fetchMandates(rpc, { authority: owner.address });
    await refused(send(stranger, [await closeMandateInstruction({ closer: stranger, mandate: mandates[0] })]), SOLANA_PULL_ERROR__NOT_CLOSABLE, 'a stranger closing a mandate that is in force');
    const ended = await send(owner, [await getInvalidateInstructionAsync({ authority: owner })]);
    check(ended.events[0].type === 'invalidated' && ended.events[0].stale === 4n, 'the owner invalidates every mandate');
    check((await fetchMandateStatus(rpc, mandates[1].address)).state === 'invalidated', 'status: invalidated');
    await refused(send(merchant, [await pullInstruction(rpc, { amount: 1n, mandate: mandates[1], spender: merchant, to: treasury })]), SOLANA_PULL_ERROR__STALE, 'a pull under any of them');
    const rent = (await rpc.getBalance(sponsor.address).send()).value;
    for (const mandate of mandates) await send(stranger, [await closeMandateInstruction({ closer: stranger, mandate })]);
    check((await fetchMandates(rpc, { authority: owner.address })).length === 0 && (await rpc.getBalance(sponsor.address).send()).value > rent, 'a stranger closes all three; the sponsor has the rent back');

    // A mandate created afterwards works as usual
    const fourth = await onboardInstructions(rpc, { authority: owner, payer: owner, terms: allow(4_000_000n) });
    await send(owner, fourth.instructions);
    check((await fetchMandateStatus(rpc, fourth.mandate)).state === 'active', 'a fourth mandate, created after, is active');
}
