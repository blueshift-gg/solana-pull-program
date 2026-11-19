// A swap order: the owner signs text and sends nothing; a solver fills it
// and pays what the owner must receive in the same instruction.
import { check, now, refused, rpc, send, tokenAccount, USDC, wallet, walletOf, WSOL } from './fork.ts';
import {
    cancelAuthorizationInstruction,
    decodeAuthorization,
    encodeAuthorization,
    enableInstruction,
    Envelope,
    fetchAuthorizationStatus,
    fillInstruction,
    getOpenInstructionAsync,
    parseAuthorization,
    requestAuthorization,
    SOLANA_PULL_ERROR__NONCE_USED,
} from '../src/index.ts';

export async function order() {
    const [owner, solver, poor] = await Promise.all([wallet({ [USDC]: 100_000_000, [WSOL]: 0 }), wallet({ [USDC]: 0, [WSOL]: 2_000_000_000 }), wallet({ [USDC]: 0, [WSOL]: 100 })]);
    const [usdc, sol] = [await tokenAccount(owner.address), await tokenAccount(owner.address, WSOL)];

    // Once per token: enable it and open the profile. Every order after is only a signature
    await send(owner, [enableInstruction({ account: usdc, owner }), await getOpenInstructionAsync({ authority: owner, payer: owner })]);

    // Sell 100 USDC for at least 0.52 SOL, to anyone, for an hour. The owner's
    // wallet only signs messages, as Phantom does: the client hands it the text
    const start = await now();
    const shown: string[] = [];
    const sell = () =>
        requestAuthorization(rpc, {
            ...walletOf(owner, 'message', shown),
            terms: {
                authority: owner.address,
                cluster: 3,
                limit: { from: usdc, max: 100_000_000n, mint: USDC, per: { __kind: 'Total' }, to: null },
                notAfter: start + 3_600n,
                notBefore: start,
                receive: { decay: null, min: 520_000_000n, mint: WSOL, to: sol },
                spender: null,
            },
        });
    const first = await sell();
    check(first.envelope === Envelope.Text && first.terms.salt === 0n, 'one call asks the wallet, picks how it signs, and takes the next salt');
    check(shown.includes('Can take: Up to 100 USDC in total') && shown.includes('Only if you receive: At least 0.52 SOL each time'), 'the wallet recognised the text and showed "At least 0.52 SOL each time"');
    check(parseAuthorization('gm') === null, 'a message that is not an authorization is not shown as one');

    // It travels to the solver as bytes. The solver asks before it pays for a transaction
    const received = decodeAuthorization(encodeAuthorization(first));
    let status = await fetchAuthorizationStatus(rpc, received);
    check(status.state === 'fillable' && status.available === 100_000_000n, 'status: fillable, 100 USDC');
    const forged = { ...received, terms: { ...received.terms, salt: 9n } };
    check((await fetchAuthorizationStatus(rpc, forged)).state === 'invalidSignature', 'status of a tampered copy: invalidSignature');

    // A solver without the SOL cannot fill it; one with it does, in one instruction
    const fill = async (who: typeof solver) =>
        send(who, [await fillInstruction({ amount: 100_000_000n, authorization: received, payFrom: await tokenAccount(who.address, WSOL), payer: who, spender: who, to: await tokenAccount(who.address) })]);
    check((await fill(poor)).error, 'a solver that cannot pay is refused');
    const filled = await fill(solver);
    const [event] = filled.events;
    check(event.type === 'filled' && event.amount === 100_000_000n && event.paid === 520_000_000n, `a solver fills it: 100 USDC out, 0.52 SOL in (${filled.units} CU)`);
    await refused(fill(solver), SOLANA_PULL_ERROR__NONCE_USED, 'a second fill');
    status = await fetchAuthorizationStatus(rpc, received);
    check(status.state === 'used', 'status: used');

    // The next order takes the next salt; the owner cancels it before anyone fills it
    const second = await sell();
    check(second.terms.salt === 1n, 'the next order takes salt 1: one page of nonces serves 1,024 orders');
    await send(owner, [await cancelAuthorizationInstruction({ authority: owner, nonceIndex: second.nonceIndex, payer: owner, salt: second.terms.salt })]);
    check((await fetchAuthorizationStatus(rpc, second)).state === 'used', 'status of a cancelled order: used');
}
