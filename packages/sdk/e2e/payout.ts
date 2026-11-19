// A payout signed in advance: the owner signs "50 USDC, only to the payee".
// Anyone can land it, whenever; nobody can redirect it or use it up for less.
import { check, now, refused, rpc, send, tokenAccount, USDC, wallet, walletOf } from './fork.ts';
import {
    enableInstruction,
    Envelope,
    fetchSeat,
    fillInstruction,
    getOpenInstructionAsync,
    requestAuthorization,
    SOLANA_PULL_ERROR__INEXACT_AMOUNT,
    SOLANA_PULL_ERROR__INVALID_DESTINATION,
} from '../src/index.ts';

export async function payout() {
    const [treasury, payee, relayer] = await Promise.all([wallet({ [USDC]: 100_000_000 }), wallet({ [USDC]: 0 }), wallet({ [USDC]: 0 })]);
    const [vault, destination, relayerUsdc] = await Promise.all([tokenAccount(treasury.address), tokenAccount(payee.address), tokenAccount(relayer.address)]);
    await send(treasury, [enableInstruction({ account: vault, owner: treasury }), await getOpenInstructionAsync({ authority: treasury, payer: treasury })]);

    // Signed now, with no expiry; landed whenever it is due, with nothing set up per payout.
    // This wallet has `solana:signOffchainMessage`, so the client uses it
    const shown: string[] = [];
    const authorization = await requestAuthorization(rpc, {
        ...walletOf(treasury, 'offchain', shown),
        terms: {
            authority: treasury.address,
            cluster: 3,
            limit: { from: vault, max: 50_000_000n, mint: USDC, per: { __kind: 'Total' }, to: destination },
            notAfter: null,
            notBefore: await now(),
            receive: null,
            spender: null,
        },
    });
    check(authorization.envelope === Envelope.OffchainMessage, 'the wallet signed an Offchain Message, built by the wallet from the text');
    check(shown.some((row) => row.startsWith('Goes to: Only ')), 'and showed that the money can only go to the payee');
    const land = (amount: bigint) => fillInstruction({ amount, authorization, payer: relayer, spender: relayer });

    await refused(send(relayer, [await land(0n)]), SOLANA_PULL_ERROR__INEXACT_AMOUNT, 'landing it for nothing, to use it up');
    const honest = await land(50_000_000n);
    const stolen = { ...honest, accounts: honest.accounts.map((a) => (a.address === destination ? { ...a, address: relayerUsdc } : a)) };
    await refused(send(relayer, [stolen]), SOLANA_PULL_ERROR__INVALID_DESTINATION, "landing it in the relayer's own account");
    check((await send(relayer, [honest])).events[0].type === 'filled', 'the relayer lands it');
    const [paid, kept] = await Promise.all([fetchSeat(rpc, destination), fetchSeat(rpc, relayerUsdc)]);
    check(paid!.balance === 50_000_000n && kept!.balance === 0n, 'the payee has 50 USDC, the relayer none');
}
