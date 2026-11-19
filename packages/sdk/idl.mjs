// The program, described once for every client. `npm run generate` writes
// `idl.json` and the TypeScript in `src/generated` from it. The address, the
// engine and the errors are read from the Rust source, not repeated here.
import fs from 'node:fs';
import { createFromRoot } from 'codama';
import { renderVisitor } from '@codama/renderers-js';
import * as c from 'codama';

const here = new URL('.', import.meta.url).pathname;
const rust = (file) => fs.readFileSync(`${here}../pull-core/src/${file}`, 'utf8');
const constant = (source, name) => source.match(new RegExp(`${name}: Pubkey =\\s*(?:five8_const::)?decode_32_const\\("(\\w+)"\\)`))[1];
const PROGRAM = constant(rust('lib.rs'), 'ID');
const ENGINE = constant(rust('constants.rs'), 'ENGINE');
const SYSTEM = '11111111111111111111111111111111';
const TOKEN = 'TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA';

// `/// doc` lines, then `Variant,` — in code order
const errors = [...rust('errors.rs').split('pub enum PullError {')[1].split('\n}')[0].matchAll(/((?:\s*\/\/\/.*\n)+)\s*(\w+),/g)].map(([, docs, name], code) =>
    c.errorNode({ code, message: docs.replace(/\s*\/\/\/ ?/g, ' ').trim(), name }),
);

const [u8, u32, u64, i64, key] = [c.numberTypeNode('u8'), c.numberTypeNode('u32'), c.numberTypeNode('u64'), c.numberTypeNode('i64'), c.publicKeyTypeNode()];
// Clear signing (sRFC 39): how a wallet that reads this IDL shows a value
const time = c.numberTypeNode('i64', 'le', { display: c.dateTimeNumberDisplayNode({}) });
const span = c.numberTypeNode('u32', 'le', { display: c.durationNumberDisplayNode({}) });
const hidden = c.structFieldDisplayNode({ skip: 'always' });
const field = (name, type, docs) => c.structFieldTypeNode({ docs, name, type });
// Absent is all zeroes, which is never a valid present value: the encoding has a fixed size
const option = (type) => c.zeroableOptionTypeNode(type);
const link = (name) => c.definedTypeLinkNode(name);
const bytes = (size) => c.fixedSizeTypeNode(c.bytesTypeNode(), size);

const types = [
    c.definedTypeNode({
        docs: ['What may go out: at most `max` may leave `from`, a token account of the authority, in its mint.'],
        name: 'out',
        type: c.structTypeNode([
            field('from', key),
            field('to', option(key), ['The only token account the tokens may go to. None leaves it to the spender.']),
            field('max', u64),
            field('period', option(span), ['Fixed windows of this length, counted from `notBefore`, with nothing carried over. None counts over the life of the mandate.']),
        ]),
    }),
    c.definedTypeNode({
        docs: ['What must come back: each pull, at least `min` arrives in `to`, a token account of the authority, in its mint.'],
        name: 'in',
        type: c.structTypeNode([field('to', key), field('min', u64)]),
    }),
    c.definedTypeNode({
        docs: ['What a mandate permits. Its authority is whoever signs `Create`.'],
        name: 'terms',
        type: c.structTypeNode([
            field('spender', option(key), ['Who may pull. None is anyone.']),
            field('notBefore', time),
            field('notAfter', option(time)),
            field('out', link('out')),
            // A suffix: present only when the terms have one
            field('in', c.remainderOptionTypeNode(link('in'))),
        ]),
    }),
];

const tag = (value) => c.structFieldTypeNode({ defaultValue: c.numberValueNode(value), defaultValueStrategy: 'omitted', name: 'tag', type: u8 });
const account = (name, value, docs, fields, pda) =>
    c.accountNode({ data: c.structTypeNode([tag(value), ...fields]), discriminators: [c.fieldDiscriminatorNode('tag')], docs, name, pda: c.pdaLinkNode(pda ?? name) });
const accounts = [
    account('mandate', 1, ['A standing permission on chain: who it is from, how much of the limit is used, then its terms.'], [
        field('authority', key, ['Signed `Create`; must own the account the terms take from.']),
        field('payer', key, ['Paid the rent; it goes back there.']),
        field('index', u64, ["Its number among the authority's mandates."]),
        field('consumed', u64, ['What the limit has used in `window`.']),
        field('window', u64, ['The window `consumed` counts: windows of the period from `notBefore`, or always 0 for a limit in total.']),
        field('terms', link('terms')),
    ]),
    account('profile', 3, ["An authority's profile: it numbers its mandates and holds what invalidates them."], [
        field('authority', key),
        field('mandates', u64, ['How many mandates it has created; they are numbered from 1.']),
        field('stale', u64, ['Every mandate numbered this or less is invalid.']),
    ]),
];

const seed = (name, type) => c.variablePdaSeedNode(name, type);
const word = (text) => c.constantPdaSeedNodeFromString('utf8', text);
const pdas = [
    c.pdaNode({ name: 'engine', seeds: [word('engine')] }),
    c.pdaNode({ name: 'profile', seeds: [word('profile'), seed('authority', key)] }),
    c.pdaNode({ name: 'mandate', seeds: [word('mandate'), seed('authority', key), seed('index', u64)] }),
];

const acc = (name, flags = '', extra = {}) =>
    c.instructionAccountNode({ isSigner: flags.includes('s') ? true : flags.includes('e') ? 'either' : false, isWritable: flags.includes('w'), name, ...extra });
// The program's own plumbing is not shown
const fixed = (name, address) => acc(name, '', { defaultValue: c.publicKeyValueNode(address), display: c.instructionAccountDisplayNode({ skip: 'always' }) });
const ownProfile = acc('profile', '', { defaultValue: c.pdaValueNode('profile', [c.pdaSeedValueNode('authority', c.accountValueNode('authority'))]) });
const tail = [fixed('engine', ENGINE), fixed('program', PROGRAM)];
const legs = [
    acc('from', 'w', { docs: ["The authority's token account."] }),
    acc('mint', '', { docs: ['The mint of `from`.'] }),
    acc('to', 'w', { docs: ['Where the tokens go: the one the terms name, or the spender\'s choice.'] }),
    ...tail,
    fixed('tokenProgram', TOKEN),
    acc('payFrom', 'w', { docs: ["The spender's token account, if the authority receives something."], isOptional: true }),
    acc('payMint', '', { docs: ['The mint of `payTo`.'], isOptional: true }),
    acc('payTo', 'w', { isOptional: true }),
    acc('payProgram', '', { isOptional: true }),
];
const arg = (name, type, docs, display) => c.instructionArgumentNode({ display, docs, name, type });
const instruction = (name, code, docs, [intent, interpolatedIntent], accountNodes, args = []) =>
    c.instructionNode({
        accounts: accountNodes,
        arguments: [c.instructionArgumentNode({ defaultValue: c.numberValueNode(code), defaultValueStrategy: 'omitted', display: hidden, name: 'discriminator', type: u8 }), ...args],
        discriminators: [c.fieldDiscriminatorNode('discriminator')],
        display: c.instructionDisplayNode({ intent, interpolatedIntent }),
        docs,
        name,
        optionalAccountStrategy: 'omitted',
    });
const instructions = [
    instruction('create', 0, ['Put a mandate on chain, under the next number of the profile.'], ['Approve a mandate', 'Let a spender collect from your account, within these terms: ${data.terms}'], [
        acc('authority', 's'),
        acc('payer', 'sw', { docs: ['Funds the rent; it goes back there.'] }),
        { ...ownProfile, isWritable: true },
        acc('mandate', 'w'),
        fixed('systemProgram', SYSTEM),
        ...tail,
    ], [arg('terms', link('terms'))]),
    instruction('pull', 1, ['Take tokens under a mandate, and deliver what the authority must receive.'], ['Collect under a mandate', 'Collect ${data.amount} from ${accounts.from} to ${accounts.to}, under mandate ${accounts.mandate}'], [acc('spender', 's'), acc('mandate', 'w'), acc('profile'), ...legs], [
        arg('amount', u64),
        arg('reference', c.remainderOptionTypeNode(bytes(32)), ["The spender's own id for this pull. Emitted, not stored."]),
    ]),
    instruction('close', 2, ['End a mandate and return its rent: its authority at any time, anyone once it expired or was invalidated.'], ['End a mandate', 'End mandate ${accounts.mandate} and return its rent to ${accounts.payer}. It can never be used again'], [
        acc('closer', 's', { docs: ['The authority, or anyone once the mandate can never be used again.'] }),
        acc('mandate', 'w'),
        acc('payer', 'w', { docs: ['The recorded payer; receives the rent.'] }),
        acc('profile'),
        ...tail,
    ]),
    instruction('open', 3, ["Create the authority's profile, once."], ['Open a profile', 'Open the profile of ${accounts.authority} with the Pull program'], [acc('authority', 's'), acc('payer', 'sw'), { ...ownProfile, isWritable: true }, fixed('systemProgram', SYSTEM), ...tail]),
    instruction('invalidate', 4, ['End every mandate so far.'], ['End every mandate', 'End every mandate of ${accounts.authority} so far. This cannot be undone'], [acc('authority', 's'), { ...ownProfile, isWritable: true }, ...tail]),
];

// Events are the data of an inner instruction to the program itself: 255, the instruction, the fields
const event = (name, code, fields) => {
    const prefix = c.constantValueNodeFromBytes('base16', `ff${code.toString(16).padStart(2, '0')}`);
    return c.eventNode({ data: c.hiddenPrefixTypeNode(c.structTypeNode(fields), [prefix]), discriminators: [c.constantDiscriminatorNode(prefix)], name });
};
const events = [
    event('created', 0, [field('mandate', key), field('authority', key)]),
    event('pulled', 1, [field('mandate', key), field('spender', key), field('amount', u64), field('paid', u64), field('reference', bytes(32))]),
    event('closed', 2, [field('mandate', key), field('closer', key)]),
    event('opened', 3, [field('authority', key)]),
    event('invalidated', 4, [field('authority', key), field('stale', u64)]),
];

// The SPL delegate of every enabled token account, and the signer of every event
const constants = [c.constantNode('engineAddress', key, c.publicKeyValueNode(ENGINE))];
const root = c.rootNode(c.programNode({ accounts, constants, definedTypes: types, errors, events, instructions, name: 'solanaPull', pdas, publicKey: PROGRAM, version: '0.1.0-rc.1' }));
fs.writeFileSync(`${here}idl.json`, `${JSON.stringify(root, null, 2)}\n`);
await createFromRoot(root).accept(renderVisitor(here.replace(/\/$/, ''), { erasableSyntax: true, importExtension: 'ts', kitImportStrategy: 'rootOnly', syncPackageJson: false }));
console.log(`idl.json and src/generated: ${instructions.length} instructions, ${accounts.length} accounts, ${events.length} events, ${errors.length} errors`);
