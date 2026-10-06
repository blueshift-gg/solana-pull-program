// A subscription: the reader approves once, the merchant collects every
// period, and either the reader cancels or the merchant finds out why not.
import { check, now, refused, rpc, send, TOKENS, tokenAccount, USDC, wallet } from './fork.ts';
import {
    cancelPolicyInstruction,
    describeInstruction,
    enableInstruction,
    fetchPolicies,
    fetchPolicyStatus,
    onboardInstructions,
    pullInstruction,
    SOLANA_PULL_ERROR__INVALID_DESTINATION,
    SOLANA_PULL_ERROR__INVALID_SPENDER,
    SOLANA_PULL_ERROR__LIMIT_EXCEEDED,
    SOLANA_PULL_ERROR__NOT_DELEGATE,
} from '../src/index.ts';

export async function subscription() {
    const [reader, merchant, sponsor, stranger] = await Promise.all([wallet({ [USDC]: 20_000_000 }), wallet({ [USDC]: 0 }), wallet(), wallet({ [USDC]: 0 })]);
    const [account, treasury] = [await tokenAccount(reader.address), await tokenAccount(merchant.address)];

    // The site builds one transaction: enable USDC, open the profile, create the policy.
    // The merchant sponsors the fee and the rent; the reader only signs
    const start = await now();
    const plan = { authority: reader.address, cluster: 3, limit: { from: account, max: 8_000_000n, mint: USDC, per: { __kind: 'Every' as const, seconds: 2_592_000 }, to: treasury }, notAfter: null, notBefore: start, receive: null, salt: 0n, spender: merchant.address };
    const { instructions, policy } = await onboardInstructions(rpc, { authority: reader, payer: sponsor, terms: plan });
    // What the reader's wallet shows for it
    const shown = instructions.map((ix) => describeInstruction(ix, TOKENS));
    check(shown.map((a) => a?.title).join(' | ') === 'Turn on pull payments for this token | Open your profile with the Pull program | Approve a standing permission', 'the wallet names all three steps');
    check(shown[2]!.rows.some(([label, value]) => label === 'Can take' && value === 'Up to 8 USDC every 30 days') && shown[2]!.caution, 'and shows "Up to 8 USDC every 30 days", with a caution that it does not expire');
    const created = await send(sponsor, instructions);
    check(created.events.map((e) => e.type).join() === 'opened,created', `one transaction onboards the reader (${created.units} CU)`);

    // The merchant asks before it charges, then charges with its invoice number
    let status = await fetchPolicyStatus(rpc, policy);
    check(status.state === 'active' && status.available === 8_000_000n, 'status: active, 8 USDC available');
    const invoice = new Uint8Array(32).fill(7);
    const charge = (amount: bigint, who = merchant) => pullInstruction({ amount, policy: status.policy!, reference: invoice, spender: who }).then((ix) => send(who, [ix]));
    const charged = await charge(5_000_000n);
    const [pulled] = charged.events;
    check(pulled.type === 'pulled' && pulled.amount === 5_000_000n && pulled.reference.every((b) => b === 7), `5 USDC is collected, and the event carries the invoice (${charged.units} CU)`);
    await refused(charge(4_000_000n), SOLANA_PULL_ERROR__LIMIT_EXCEEDED, 'a charge past what is left this month');
    await refused(charge(1n, stranger), SOLANA_PULL_ERROR__INVALID_SPENDER, 'a charge by anyone else');
    status = await fetchPolicyStatus(rpc, policy);
    check(status.state === 'active' && status.available === 3_000_000n, 'status: active, 3 USDC left this month');

    // A stolen merchant key can collect, but only into the merchant's own treasury
    const redirect = await pullInstruction({ amount: 1n, policy: status.policy!, spender: merchant });
    const elsewhere = await tokenAccount(stranger.address);
    const forged = { ...redirect, accounts: redirect.accounts.map((a) => (a.address === treasury ? { ...a, address: elsewhere } : a)) };
    await refused(send(merchant, [forged]), SOLANA_PULL_ERROR__INVALID_DESTINATION, 'a charge sent anywhere but the treasury');

    // The reader moves its delegate elsewhere: the merchant learns why, and one Approve restores it
    const approve = enableInstruction({ account, owner: reader });
    await send(reader, [{ ...approve, accounts: approve.accounts!.map((a, i) => (i === 1 ? { ...a, address: stranger.address } : a)) }]);
    check((await fetchPolicyStatus(rpc, policy)).state === 'notDelegate', 'status: notDelegate after another Approve');
    await refused(charge(1n), SOLANA_PULL_ERROR__NOT_DELEGATE, 'a charge then');
    await send(reader, [enableInstruction({ account, owner: reader })]);
    check((await fetchPolicyStatus(rpc, policy)).state === 'active', 'one Approve restores it: nothing is created or signed anew');

    // Both sides can list it; the reader cancels, and the sponsor has its rent back
    const [mine, theirs] = await Promise.all([fetchPolicies(rpc, { authority: reader.address }), fetchPolicies(rpc, { spender: merchant.address })]);
    check(mine.length === 1 && theirs.length === 1 && mine[0].address === policy && theirs[0].address === policy, 'the reader and the merchant each list the policy');
    const before = (await rpc.getBalance(sponsor.address).send()).value;
    const cancelled = await send(reader, [await cancelPolicyInstruction({ authority: reader, policy: mine[0] })]);
    check(cancelled.events[0].type === 'cancelled', 'the reader cancels');
    check((await rpc.getBalance(sponsor.address).send()).value > before, 'the rent is back with the sponsor');
    check((await fetchPolicyStatus(rpc, policy)).state === 'missing', 'status: missing');
}
