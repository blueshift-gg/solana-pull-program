// Ending everything at once: one transaction ends every policy, another
// every signed intent, and anyone can then close what was ended, with the
// rent going back to whoever paid it.
import { signBytes } from '@solana/kit';

import { check, now, refused, rpc, send, tokenAccount, USDC, wallet } from './fork.ts';
import {
    closePolicyInstruction,
    Envelope,
    fetchPolicies,
    fetchPolicyStatus,
    fetchProfile,
    fillInstruction,
    findNoncesPda,
    findProfilePda,
    getCloseInstruction,
    getInvalidateInstructionAsync,
    intentMessage,
    onboardInstructions,
    pullInstruction,
    SOLANA_PULL_ERROR__INVALID_SIGNATURE,
    SOLANA_PULL_ERROR__NOT_CLOSABLE,
    SOLANA_PULL_ERROR__STALE,
    terms,
} from '../src/index.ts';

export async function undo() {
    const [owner, merchant, sponsor, stranger] = await Promise.all([wallet({ [USDC]: 100_000_000 }), wallet({ [USDC]: 0 }), wallet(), wallet()]);
    const [account, treasury] = [await tokenAccount(owner.address), await tokenAccount(merchant.address)];
    const start = await now();
    const allow = (max: bigint) => ({ authority: owner.address, cluster: 3, limit: { from: account, max, mint: USDC, per: { __kind: 'Total' as const }, to: null }, notAfter: null, notBefore: start, receive: null, salt: 0n, spender: merchant.address });

    // Three policies, numbered as they are created, and one signed intent that has run
    for (const max of [1_000_000n, 2_000_000n, 3_000_000n]) await send(sponsor, (await onboardInstructions(rpc, { authority: owner, payer: sponsor, terms: allow(max) })).instructions);
    const [profile] = await findProfilePda({ authority: owner.address });
    const before = (await fetchProfile(rpc, profile)).data;
    check(before.policies === 3n && before.stale === 0n, 'the profile counts three policies, none stale');
    const hold = terms({ ...allow(5_000_000n), salt: 0n });
    const later = terms({ ...allow(5_000_000n), salt: 1n });
    const sign = async (t: typeof hold) => new Uint8Array(await signBytes(owner.keyPair.privateKey, intentMessage(t, before.nonceIndex, { [USDC]: 6 }, Envelope.OffchainMessage)));
    const fill = async (t: typeof hold, nonceIndex: bigint, signature: Uint8Array) => send(merchant, [await fillInstruction({ envelope: Envelope.OffchainMessage, amount: 1_000_000n, nonceIndex, payer: merchant, signature, spender: merchant, terms: t, to: treasury })]);
    await fill(hold, before.nonceIndex, await sign(hold));
    const [page] = await findNoncesPda({ authority: owner.address, nonceIndex: before.nonceIndex, page: 0n });
    const reap = (who: typeof stranger) => getCloseInstruction({ account: page, closer: who, payer: merchant.address, profile });
    await refused(send(stranger, [reap(stranger)]), SOLANA_PULL_ERROR__NOT_CLOSABLE, 'closing a page of nonces that is in use');

    // One transaction ends every policy
    const policies = await fetchPolicies(rpc, { authority: owner.address });
    await refused(send(stranger, [await closePolicyInstruction({ closer: stranger, policy: policies[0] })]), SOLANA_PULL_ERROR__NOT_CLOSABLE, 'closing a policy that is in force');
    const ended = await send(owner, [await getInvalidateInstructionAsync({ authority: owner, what: 1 })]);
    check(ended.events[0].type === 'invalidated' && ended.events[0].stale === 3n, 'the owner invalidates every policy');
    check((await fetchPolicyStatus(rpc, policies[1].address)).state === 'invalidated', 'status: invalidated');
    await refused(send(merchant, [await pullInstruction({ amount: 1n, policy: policies[1], spender: merchant, to: treasury })]), SOLANA_PULL_ERROR__STALE, 'a pull under any of them');
    const rent = (await rpc.getBalance(sponsor.address).send()).value;
    for (const policy of policies) await send(stranger, [await closePolicyInstruction({ closer: stranger, policy })]);
    check((await fetchPolicies(rpc, { authority: owner.address })).length === 0 && (await rpc.getBalance(sponsor.address).send()).value > rent, 'a stranger closes all three; the sponsor has the rent back');

    // Another ends every signed intent, without touching policies created since
    const fourth = await onboardInstructions(rpc, { authority: owner, payer: owner, terms: allow(4_000_000n) });
    await send(owner, fourth.instructions);
    const signature = await sign(later);
    await send(owner, [await getInvalidateInstructionAsync({ authority: owner, what: 2 })]);
    const after = (await fetchProfile(rpc, profile)).data;
    check(after.nonceIndex !== before.nonceIndex && after.nonceIndex !== before.nonceIndex + 1n, 'the nonce index moves, and not by one');
    await refused(fill(later, after.nonceIndex, signature), SOLANA_PULL_ERROR__INVALID_SIGNATURE, 'an intent signed before');
    check((await fetchPolicyStatus(rpc, fourth.policy)).state === 'active', 'the fourth policy is still active');
    const paid = (await rpc.getBalance(merchant.address).send()).value;
    await send(stranger, [reap(stranger)]);
    check((await rpc.getBalance(merchant.address).send()).value > paid, 'a stranger closes the old page of nonces; the merchant, who paid for it, has the rent back');
}
