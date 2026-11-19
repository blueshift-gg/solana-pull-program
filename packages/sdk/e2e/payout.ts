// A payout set up in advance: "50 USDC, only to the payee". Anyone can land
// it, whenever; nobody can redirect it.
import { check, now, refused, rpc, send, tokenAccount, USDC, wallet } from './fork.ts';
import { fetchMandateStatus, fetchSeat, onboardInstructions, pullInstruction, SOLANA_PULL_ERROR__INVALID_DESTINATION } from '../src/index.ts';

export async function payout() {
    const [treasury, payee, relayer] = await Promise.all([wallet({ [USDC]: 100_000_000 }), wallet({ [USDC]: 0 }), wallet({ [USDC]: 0 })]);
    const [vault, destination, relayerUsdc] = await Promise.all([tokenAccount(treasury.address), tokenAccount(payee.address), tokenAccount(relayer.address)]);
    const terms = { authority: treasury.address, limit: { from: vault, max: 50_000_000n, per: { __kind: 'Total' as const }, to: destination }, notAfter: null, notBefore: await now(), receive: null, spender: null };
    const { instructions, mandate } = await onboardInstructions(rpc, { authority: treasury, payer: treasury, terms });
    await send(treasury, instructions);
    const { mandate: account } = await fetchMandateStatus(rpc, mandate);
    const honest = await pullInstruction(rpc, { amount: 50_000_000n, mandate: account!, spender: relayer });
    const stolen = { ...honest, accounts: honest.accounts.map((a) => (a.address === destination ? { ...a, address: relayerUsdc } : a)) };

    await refused(send(relayer, [stolen]), SOLANA_PULL_ERROR__INVALID_DESTINATION, "landing it in the relayer's own account");
    check((await send(relayer, [honest])).events[0].type === 'pulled', 'the relayer lands it');
    const [paid, kept] = await Promise.all([fetchSeat(rpc, destination), fetchSeat(rpc, relayerUsdc)]);
    check(paid!.balance === 50_000_000n && kept!.balance === 0n, 'the payee has 50 USDC, the relayer none');
}
