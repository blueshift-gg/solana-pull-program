// Ending everything at once: one transaction ends every mandate, another
// every signed authorization, and anyone can then close what was ended, with the
// rent going back to whoever paid it.
import { check, now, refused, rpc, send, tokenAccount, USDC, wallet, walletOf } from './fork.ts';
import {
    closeMandateInstruction,
    fetchAuthorizationStatus,
    fetchMandates,
    fetchMandateStatus,
    fetchProfile,
    fillInstruction,
    findNoncesPda,
    findProfilePda,
    getCloseInstruction,
    getInvalidateInstructionAsync,
    onboardInstructions,
    pullInstruction,
    requestAuthorization,
    SOLANA_PULL_ERROR__INVALID_SIGNATURE,
    SOLANA_PULL_ERROR__NOT_CLOSABLE,
    SOLANA_PULL_ERROR__STALE,
} from '../src/index.ts';

export async function undo() {
    const [owner, merchant, sponsor, stranger] = await Promise.all([wallet({ [USDC]: 100_000_000 }), wallet({ [USDC]: 0 }), wallet(), wallet()]);
    const [account, treasury] = [await tokenAccount(owner.address), await tokenAccount(merchant.address)];
    const start = await now();
    const allow = (max: bigint) => ({ authority: owner.address, cluster: 3, limit: { from: account, max, mint: USDC, per: { __kind: 'Total' as const }, to: null }, notAfter: null, notBefore: start, receive: null, salt: 0n, spender: merchant.address });

    // Three mandates, numbered as they are created, and one signed authorization that has run
    for (const max of [1_000_000n, 2_000_000n, 3_000_000n]) await send(sponsor, (await onboardInstructions(rpc, { authority: owner, payer: sponsor, terms: allow(max) })).instructions);
    const [profile] = await findProfilePda({ authority: owner.address });
    const before = (await fetchProfile(rpc, profile)).data;
    check(before.mandates === 3n && before.stale === 0n, 'the profile counts three mandates, none stale');
    const ask = (salt: bigint) => requestAuthorization(rpc, { ...walletOf(owner, 'offchain'), terms: { ...allow(5_000_000n), salt } });
    const fill = async (authorization: Awaited<ReturnType<typeof ask>>) => send(merchant, [await fillInstruction({ amount: 1_000_000n, authorization, payer: merchant, spender: merchant, to: treasury })]);
    await fill(await ask(0n));
    const [page] = await findNoncesPda({ authority: owner.address, nonceIndex: before.nonceIndex, page: 0n });
    const reap = (who: typeof stranger) => getCloseInstruction({ account: page, closer: who, payer: merchant.address, profile });
    await refused(send(stranger, [reap(stranger)]), SOLANA_PULL_ERROR__NOT_CLOSABLE, 'closing a page of nonces that is in use');

    // One transaction ends every mandate
    const mandates = await fetchMandates(rpc, { authority: owner.address });
    await refused(send(stranger, [await closeMandateInstruction({ closer: stranger, mandate: mandates[0] })]), SOLANA_PULL_ERROR__NOT_CLOSABLE, 'closing a mandate that is in force');
    const ended = await send(owner, [await getInvalidateInstructionAsync({ authority: owner, what: 1 })]);
    check(ended.events[0].type === 'invalidated' && ended.events[0].stale === 4n, 'the owner invalidates every mandate');
    check((await fetchMandateStatus(rpc, mandates[1].address)).state === 'invalidated', 'status: invalidated');
    await refused(send(merchant, [await pullInstruction({ amount: 1n, mandate: mandates[1], spender: merchant, to: treasury })]), SOLANA_PULL_ERROR__STALE, 'a pull under any of them');
    const rent = (await rpc.getBalance(sponsor.address).send()).value;
    for (const mandate of mandates) await send(stranger, [await closeMandateInstruction({ closer: stranger, mandate })]);
    check((await fetchMandates(rpc, { authority: owner.address })).length === 0 && (await rpc.getBalance(sponsor.address).send()).value > rent, 'a stranger closes all three; the sponsor has the rent back');

    // Another ends every signed authorization, without touching mandates created since
    const fourth = await onboardInstructions(rpc, { authority: owner, payer: owner, terms: allow(4_000_000n) });
    await send(owner, fourth.instructions);
    const later = await ask(1n);
    await send(owner, [await getInvalidateInstructionAsync({ authority: owner, what: 2 })]);
    const after = (await fetchProfile(rpc, profile)).data;
    check(after.nonceIndex !== before.nonceIndex && after.nonceIndex !== before.nonceIndex + 1n, 'the nonce index moves, and not by one');
    check((await fetchAuthorizationStatus(rpc, later)).state === 'invalidated', 'status of an authorization signed before: invalidated');
    await refused(fill({ ...later, nonceIndex: after.nonceIndex }), SOLANA_PULL_ERROR__INVALID_SIGNATURE, 'filling it anyway');
    check((await fetchMandateStatus(rpc, fourth.mandate)).state === 'active', 'the fourth mandate is still active');
    const paid = (await rpc.getBalance(merchant.address).send()).value;
    await send(stranger, [reap(stranger)]);
    check((await rpc.getBalance(merchant.address).send()).value > paid, 'a stranger closes the old page of nonces; the merchant, who paid for it, has the rent back');
}
