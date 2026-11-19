// The program, described once for every client. `npm run generate` writes
// `idl.json` and the TypeScript in `src/generated` from it. Every address,
// seed, tag, discriminator, length and error is read from the Rust source,
// not repeated here.
import fs from 'node:fs';
import { createFromRoot } from 'codama';
import { renderVisitor } from '@codama/renderers-js';
import * as c from 'codama';

const here = new URL('.', import.meta.url).pathname;
const read = (path) => fs.readFileSync(`${here}../../${path}`, 'utf8');
const rust = (file) => read(`packages/pull-core/src/${file}`);
const find = (source, pattern) => source.match(pattern)?.[1] ?? (() => { throw new Error(`not in the Rust source: ${pattern}`); })();
const constants_rs = rust('constants.rs');
const address = (source, name) => find(source, new RegExp(`${name}: Pubkey =\\s*(?:five8_const::)?decode_32_const\\("(\\w+)"\\)`));
// A `pub const` whose value is a number, or a sum of numbers
const number = (name) => find(constants_rs, new RegExp(`pub const ${name}: \\w+ = ([\\d_ +]+);`)).split('+').reduce((sum, n) => sum + Number(n.replaceAll('_', '')), 0);
const seedOf = (name) => find(constants_rs, new RegExp(`pub const ${name}: &\\[u8\\] = b"(\\w+)";`));
const discriminator = (name) => Number(find(read(`program/src/${name}.rs`), /pub const DISCRIMINATOR: u8 = (\d+);/));
const PROGRAM = address(rust('lib.rs'), 'ID');
const ENGINE = address(constants_rs, 'ENGINE');
const TOKEN = address(constants_rs, 'TOKEN_PROGRAM');
const TOKEN_2022 = address(constants_rs, 'TOKEN_2022_PROGRAM');
const SYSTEM = '11111111111111111111111111111111';
const VERSION = find(read('program/Cargo.toml'), /^version = "([^"]+)"/m);

// `/// doc` lines, then `Variant,` — in code order
const errors = [...rust('errors.rs').split('pub enum PullError {')[1].split('\n}')[0].matchAll(/((?:\s*\/\/\/.*\n)+)\s*(\w+),/g)].map(([, docs, name], code) =>
    c.errorNode({ code, message: docs.replace(/\s*\/\/\/ ?/g, ' ').trim(), name }),
);

const [u8, u64, key] = [c.numberTypeNode('u8'), c.numberTypeNode('u64'), c.publicKeyTypeNode()];
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
            field('maxPerPull', option(u64), ['The most one pull may take, at most `max`. None lets one pull take all that is left.']),
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
    account('mandate', number('MANDATE_TAG'), ['A standing permission on chain: who it is from, how much of the limit is used, then its terms.'], [
        field('authority', key, ['Signed `Create`; must own the account the terms take from.']),
        field('payer', key, ['Paid the rent; it goes back there.']),
        field('index', u64, ["Its number among the authority's mandates."]),
        field('consumed', u64, ['What the limit has used in `window`.']),
        field('window', u64, ['The window `consumed` counts: windows of the period from `notBefore`, or always 0 for a limit in total.']),
        field('terms', link('terms')),
    ]),
    account('profile', number('PROFILE_TAG'), ["An authority's profile: it numbers its mandates and holds what invalidates them."], [
        field('authority', key),
        field('mandates', u64, ['How many mandates it has created; they are numbered from 1.']),
        field('stale', u64, ['Every mandate numbered this or less is invalid.']),
    ]),
];

const seed = (name, type) => c.variablePdaSeedNode(name, type);
const word = (text) => c.constantPdaSeedNodeFromString('utf8', text);
const pdas = [
    c.pdaNode({ name: 'engine', seeds: [word(seedOf('ENGINE_SEED'))] }),
    c.pdaNode({ name: 'profile', seeds: [word(seedOf('PROFILE_SEED')), seed('authority', key)] }),
    c.pdaNode({ name: 'mandate', seeds: [word(seedOf('MANDATE_SEED')), seed('authority', key), seed('index', u64)] }),
];

const acc = (name, flags = '', extra = {}) =>
    c.instructionAccountNode({ isSigner: flags.includes('s'), isWritable: flags.includes('w'), name, ...extra });
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
const instruction = (name, docs, [intent, interpolatedIntent], accountNodes, args = []) =>
    c.instructionNode({
        accounts: accountNodes,
        arguments: [c.instructionArgumentNode({ defaultValue: c.numberValueNode(discriminator(name)), defaultValueStrategy: 'omitted', display: hidden, name: 'discriminator', type: u8 }), ...args],
        discriminators: [c.fieldDiscriminatorNode('discriminator')],
        display: c.instructionDisplayNode({ intent, interpolatedIntent }),
        docs,
        name,
        optionalAccountStrategy: 'omitted',
    });
const instructions = [
    instruction('create', ['Put a mandate on chain, under the next number of the profile.'], ['Approve a mandate', 'Let a spender collect from your account, within these terms: ${data.terms}'], [
        acc('authority', 's'),
        acc('payer', 'sw', { docs: ['Funds the rent; it goes back there.'] }),
        { ...ownProfile, isWritable: true },
        acc('mandate', 'w'),
        fixed('systemProgram', SYSTEM),
        ...tail,
    ], [arg('terms', link('terms'))]),
    instruction('pull', ['Take tokens under a mandate, and deliver what the authority must receive.'], ['Collect under a mandate', 'Collect ${data.amount} from ${accounts.from} to ${accounts.to}, under mandate ${accounts.mandate}'], [acc('spender', 's'), acc('mandate', 'w'), acc('profile'), ...legs], [
        arg('amount', u64),
        arg('pay', u64, ['What the spender sends to `in.to`: at least `in.min`, more if the mint takes a transfer fee. 0 when the terms have no `in`.']),
        arg('reference', c.remainderOptionTypeNode(bytes(32)), ["The spender's own id for this pull. Emitted, not stored."]),
    ]),
    instruction('close', ['End a mandate and return its rent: its authority at any time, anyone once it can never pull again: expired, invalidated or used up.'], ['End a mandate', 'End mandate ${accounts.mandate} and return its rent to ${accounts.payer}. It can never be used again'], [
        acc('closer', 's', { docs: ['The authority, or anyone once the mandate can never be used again.'] }),
        acc('mandate', 'w'),
        acc('payer', 'w', { docs: ['The recorded payer; receives the rent.'] }),
        acc('profile'),
        ...tail,
    ]),
    instruction('open', ["Create the authority's profile, once."], ['Open a profile', 'Open the profile of ${accounts.authority} with the Pull program'], [acc('authority', 's'), acc('payer', 'sw'), { ...ownProfile, isWritable: true }, fixed('systemProgram', SYSTEM), ...tail]),
    instruction('invalidate', ['End every mandate so far.'], ['End every mandate', 'End every mandate of ${accounts.authority} so far. This cannot be undone'], [acc('authority', 's'), { ...ownProfile, isWritable: true }, ...tail]),
];

// Events are the data of an inner instruction to the program itself: 255, the instruction, the fields
// Each event is emitted by one instruction, and carries its discriminator
const event = (name, instruction, fields) => {
    const code = discriminator(instruction);
    const prefix = c.constantValueNodeFromBytes('base16', [number('EVENT_DISCRIMINATOR'), code].map((b) => b.toString(16).padStart(2, '0')).join(''));
    return c.eventNode({ data: c.hiddenPrefixTypeNode(c.structTypeNode(fields), [prefix]), discriminators: [c.constantDiscriminatorNode(prefix)], name });
};
const events = [
    event('created', 'create', [field('mandate', key), field('authority', key)]),
    event('pulled', 'pull', [field('mandate', key), field('spender', key), field('amount', u64), field('paid', u64), field('reference', bytes(32))]),
    event('closed', 'close', [field('mandate', key), field('closer', key)]),
    event('opened', 'open', [field('authority', key)]),
    event('invalidated', 'invalidate', [field('authority', key), field('stale', u64)]),
];

const constants = [
    // The SPL delegate of every enabled token account, and the signer of every event
    c.constantNode('engineAddress', key, c.publicKeyValueNode(ENGINE)),
    c.constantNode('tokenProgramAddress', key, c.publicKeyValueNode(TOKEN)),
    c.constantNode('token2022ProgramAddress', key, c.publicKeyValueNode(TOKEN_2022)),
    // The first byte of every event: the self-CPI that carries it
    c.constantNode('eventDiscriminator', u8, c.numberValueNode(number('EVENT_DISCRIMINATOR'))),
    // The latest timestamp terms may hold
    c.constantNode('maxTime', c.numberTypeNode('i64'), c.numberValueNode(number('MAX_TIME'))),
    // Where a mandate's terms start: its spender is the first field
    c.constantNode('mandateHeaderLength', u64, c.numberValueNode(number('MANDATE_LEN'))),
];
const root = c.rootNode(c.programNode({ accounts, constants, definedTypes: types, errors, events, instructions, name: 'solanaPull', pdas, publicKey: PROGRAM, version: VERSION }));
fs.writeFileSync(`${here}idl.json`, `${JSON.stringify(root, null, 2)}\n`);
await createFromRoot(root).accept(renderVisitor(here.replace(/\/$/, ''), { erasableSyntax: true, importExtension: 'ts', kitImportStrategy: 'rootOnly', syncPackageJson: false }));
console.log(`idl.json and src/generated: ${instructions.length} instructions, ${accounts.length} accounts, ${events.length} events, ${errors.length} errors`);
