// A subscription: the reader approves once, the merchant collects every
// period, and either the reader ends it or the merchant finds out why not.
import { check, now, refused, rpc, send, TOKENS, tokenAccount, USDC, wallet } from './fork.ts';
import {
    closeMandateInstruction,
    describeInstruction,
    enableInstruction,
    fetchMandates,
    fetchMandateStatus,
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

    // The site builds one transaction: enable USDC, open the profile, create the mandate.
    // The merchant sponsors the fee and the rent; the reader only signs
    const start = await now();
    const plan = { out: { from: account, max: 8_000_000n, period: 2_592_000, to: treasury }, notAfter: null, notBefore: start, in: null, spender: merchant.address };
    const { instructions, mandate } = await onboardInstructions(rpc, { authority: reader, payer: sponsor, terms: plan });
    // What the reader's wallet shows for it: it knows its own USDC account
    const shown = instructions.map((ix) => describeInstruction(ix, { [account]: TOKENS[USDC] }));
    check(shown.map((a) => a?.title).join(' | ') === 'Turn on pull payments for this token | Open your profile with the Pull program | Approve a standing permission', 'the wallet names all three steps');
    check(shown[2]!.rows.some(([label, value]) => label === 'Can take' && value === 'Up to 8 USDC every 30 days') && shown[2]!.caution, 'and shows "Up to 8 USDC every 30 days", with a caution that it does not expire');
    const created = await send(sponsor, instructions);
    check(created.events.map((e) => e.type).join() === 'opened,created', `one transaction onboards the reader (${created.units} CU)`);

    // The merchant asks before it charges, then charges with its invoice number
    let status = await fetchMandateStatus(rpc, mandate);
    check(status.state === 'active' && status.available === 8_000_000n, 'status: active, 8 USDC available');
    const invoice = new Uint8Array(32).fill(7);
    const charge = (amount: bigint, who = merchant) => pullInstruction(rpc, { amount, mandate: status.mandate!, reference: invoice, spender: who }).then((ix) => send(who, [ix]));
    const charged = await charge(5_000_000n);
    const [pulled] = charged.events;
    check(pulled.type === 'pulled' && pulled.amount === 5_000_000n && pulled.reference.every((b) => b === 7), `5 USDC is collected, and the event carries the invoice (${charged.units} CU)`);
    await refused(charge(4_000_000n), SOLANA_PULL_ERROR__LIMIT_EXCEEDED, 'a charge past what is left this month');
    await refused(charge(1n, stranger), SOLANA_PULL_ERROR__INVALID_SPENDER, 'a charge by anyone else');
    status = await fetchMandateStatus(rpc, mandate);
    check(status.state === 'active' && status.available === 3_000_000n, 'status: active, 3 USDC left this month');

    // A stolen merchant key can collect, but only into the merchant's own treasury
    const redirect = await pullInstruction(rpc, { amount: 1n, mandate: status.mandate!, spender: merchant });
    const elsewhere = await tokenAccount(stranger.address);
    const forged = { ...redirect, accounts: redirect.accounts.map((a) => (a.address === treasury ? { ...a, address: elsewhere } : a)) };
    await refused(send(merchant, [forged]), SOLANA_PULL_ERROR__INVALID_DESTINATION, 'a charge sent anywhere but the treasury');

    // The reader moves its delegate elsewhere: the merchant learns why, and one Approve restores it
    const approve = enableInstruction({ account, owner: reader });
    await send(reader, [{ ...approve, accounts: approve.accounts!.map((a, i) => (i === 1 ? { ...a, address: stranger.address } : a)) }]);
    check((await fetchMandateStatus(rpc, mandate)).state === 'notDelegate', 'status: notDelegate after another Approve');
    await refused(charge(1n), SOLANA_PULL_ERROR__NOT_DELEGATE, 'a charge then');
    await send(reader, [enableInstruction({ account, owner: reader })]);
    check((await fetchMandateStatus(rpc, mandate)).state === 'active', 'one Approve restores it: nothing is created or signed anew');

    // Both sides can list it; the reader ends it, and the sponsor has its rent back
    const [mine, theirs] = await Promise.all([fetchMandates(rpc, { authority: reader.address }), fetchMandates(rpc, { spender: merchant.address })]);
    check(mine.length === 1 && theirs.length === 1 && mine[0].address === mandate && theirs[0].address === mandate, 'the reader and the merchant each list the mandate');
    const before = (await rpc.getBalance(sponsor.address).send()).value;
    const closed = await send(reader, [await closeMandateInstruction({ closer: reader, mandate: mine[0] })]);
    check(closed.events[0].type === 'closed', 'the reader ends it');
    check((await rpc.getBalance(sponsor.address).send()).value > before, 'the rent is back with the sponsor');
    check((await fetchMandateStatus(rpc, mandate)).state === 'missing', 'status: missing');
}
