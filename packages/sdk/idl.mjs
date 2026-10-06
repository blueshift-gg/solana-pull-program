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
const field = (name, type, docs) => c.structFieldTypeNode({ docs, name, type });
const option = (type) => c.optionTypeNode(type);
const link = (name) => c.definedTypeLinkNode(name);
const bytes = (size) => c.fixedSizeTypeNode(c.bytesTypeNode(), size);

const types = [
    c.definedTypeNode({
        docs: ['What comes before the intent text in the bytes the authority signed.'],
        name: 'envelope',
        type: c.enumTypeNode([
            c.enumEmptyVariantTypeNode('text', ['Nothing: the text alone, as `solana:signMessage` signs it.']),
            c.enumEmptyVariantTypeNode('offchainMessage', ['The Offchain Message v1 preamble, as `solana:signOffchainMessage` signs it.']),
        ]),
    }),
    c.definedTypeNode({
        docs: ['What the limit counts over.'],
        name: 'per',
        type: c.enumTypeNode([
            c.enumEmptyVariantTypeNode('total'),
            c.enumStructVariantTypeNode('every', c.structTypeNode([field('seconds', u32, ['Fixed windows of this length, counted from `notBefore`. Nothing carries over.'])])),
        ]),
    }),
    c.definedTypeNode({
        docs: ['What may go out: at most `max` of `mint` may leave `from`, a token account of the authority.'],
        name: 'limit',
        type: c.structTypeNode([
            field('from', key),
            field('mint', key),
            field('max', u64),
            field('per', link('per')),
            field('to', option(key), ['The only token account the tokens may go to. None leaves it to the spender.']),
        ]),
    }),
    c.definedTypeNode({
        docs: ['The minimum moves in a straight line to `min` between `t0` and `t1`.'],
        name: 'decay',
        type: c.structTypeNode([field('t0', i64), field('t1', i64), field('min', u64)]),
    }),
    c.definedTypeNode({
        docs: ['What must come in: each use, at least `min` of `mint` arrives in `to`, a token account of the authority.'],
        name: 'receive',
        type: c.structTypeNode([field('to', key), field('mint', key), field('min', u64), field('decay', option(link('decay')))]),
    }),
    c.definedTypeNode({
        docs: ['What an authority permits: the same for a policy and for a signed intent.'],
        name: 'terms',
        type: c.structTypeNode([
            c.structFieldTypeNode({ defaultValue: c.numberValueNode(1), defaultValueStrategy: 'omitted', name: 'version', type: u8 }),
            field('cluster', u8, ['0 mainnet, 1 devnet, 2 testnet, 3 localnet: the cluster the program was built for.']),
            field('authority', key),
            field('spender', option(key), ['Who may pull. None is anyone.']),
            field('notBefore', i64),
            field('notAfter', option(i64)),
            field('salt', u64, ["A signed intent's nonce. A policy does not use it."]),
            field('limit', link('limit')),
            field('receive', option(link('receive'))),
        ]),
    }),
];

const tag = (value) => c.structFieldTypeNode({ defaultValue: c.numberValueNode(value), defaultValueStrategy: 'omitted', name: 'tag', type: u8 });
const account = (name, value, docs, fields, pda) =>
    c.accountNode({ data: c.structTypeNode([tag(value), ...fields]), discriminators: [c.fieldDiscriminatorNode('tag')], docs, name, pda: c.pdaLinkNode(pda ?? name) });
const accounts = [
    account('policy', 1, ['A standing permission on chain: this header, then its terms.'], [
        field('rolled', i64, ['When `consumed` was last written.']),
        field('consumed', u64),
        field('payer', key, ['Paid the rent; it goes back there.']),
        field('index', u64, ["Its number among the authority's policies."]),
        field('terms', c.sizePrefixTypeNode(link('terms'), c.numberTypeNode('u16'))),
    ]),
    account('nonces', 2, ["One page of the nonces an authority's signed intents have used or cancelled."], [
        field('authority', key),
        field('index', u64, ['The nonce index its intents were signed under.']),
        field('page', u64),
        field('payer', key, ['Paid the rent; it goes back there.']),
        field('bits', bytes(128)),
    ]),
    account('profile', 3, ["An authority's profile: it numbers its policies and holds what invalidates them and its intents."], [
        field('authority', key),
        field('policies', u64, ['How many policies it has created; they are numbered from 1.']),
        field('stale', u64, ['Every policy numbered this or less is invalid.']),
        field('nonceIndex', u64, ['What its intents are signed under.']),
    ]),
];

const seed = (name, type) => c.variablePdaSeedNode(name, type);
const word = (text) => c.constantPdaSeedNodeFromString('utf8', text);
const pdas = [
    c.pdaNode({ name: 'engine', seeds: [word('engine')] }),
    c.pdaNode({ name: 'profile', seeds: [word('profile'), seed('authority', key)] }),
    c.pdaNode({ name: 'policy', seeds: [word('policy'), seed('authority', key), seed('index', u64)] }),
    c.pdaNode({ name: 'nonces', seeds: [word('nonces'), seed('authority', key), seed('nonceIndex', u64), seed('page', u64)] }),
];

const acc = (name, flags = '', extra = {}) =>
    c.instructionAccountNode({ isSigner: flags.includes('s') ? true : flags.includes('e') ? 'either' : false, isWritable: flags.includes('w'), name, ...extra });
const fixed = (name, address) => acc(name, '', { defaultValue: c.publicKeyValueNode(address) });
const ownProfile = acc('profile', '', { defaultValue: c.pdaValueNode('profile', [c.pdaSeedValueNode('authority', c.accountValueNode('authority'))]) });
const tail = [fixed('engine', ENGINE), fixed('program', PROGRAM)];
const legs = [
    acc('from', 'w', { docs: ["The authority's token account."] }),
    acc('mint'),
    acc('to', 'w', { docs: ['Where the tokens go: the one the terms name, or the spender\'s choice.'] }),
    ...tail,
    fixed('tokenProgram', TOKEN),
    acc('payFrom', 'w', { docs: ["The spender's token account, if the authority receives something."], isOptional: true }),
    acc('payMint', '', { isOptional: true }),
    acc('payTo', 'w', { isOptional: true }),
    acc('payProgram', '', { isOptional: true }),
];
const arg = (name, type, docs) => c.instructionArgumentNode({ docs, name, type });
const instruction = (name, code, docs, accountNodes, args = []) =>
    c.instructionNode({
        accounts: accountNodes,
        arguments: [c.instructionArgumentNode({ defaultValue: c.numberValueNode(code), defaultValueStrategy: 'omitted', name: 'discriminator', type: u8 }), ...args],
        discriminators: [c.fieldDiscriminatorNode('discriminator')],
        docs,
        name,
        optionalAccountStrategy: 'omitted',
    });
const instructions = [
    instruction('create', 0, ['Put a policy on chain, under the next number of the profile.'], [
        acc('authority', 's'),
        acc('payer', 'sw', { docs: ['Funds the rent; it goes back there.'] }),
        { ...ownProfile, isWritable: true },
        acc('policy', 'w'),
        fixed('systemProgram', SYSTEM),
        ...tail,
    ], [arg('terms', link('terms'))]),
    instruction('pull', 1, ['Take tokens under a policy, and deliver what the authority must receive.'], [acc('spender', 's'), acc('policy', 'w'), acc('profile'), ...legs], [
        arg('amount', u64),
        arg('reference', c.remainderOptionTypeNode(bytes(32)), ["The spender's own id for this pull. Emitted, not stored."]),
    ]),
    instruction('close', 2, ['Close what can never be used again, and return its rent. Anyone may.'], [
        acc('closer', 's'),
        acc('account', 'w', { docs: ['A policy that expired or was invalidated, or a page of nonces under an old nonce index.'] }),
        acc('payer', 'w', { docs: ['The recorded payer; receives the rent.'] }),
        acc('profile'),
        ...tail,
    ]),
    instruction('open', 3, ["Create the authority's profile, once."], [acc('authority', 's'), acc('payer', 'sw'), { ...ownProfile, isWritable: true }, fixed('systemProgram', SYSTEM), ...tail]),
    instruction('invalidate', 4, ['End every policy so far, every intent so far, or both.'], [acc('authority', 's'), { ...ownProfile, isWritable: true }, ...tail], [
        arg('what', u8, ['1 for policies, 2 for intents, 3 for both.']),
    ]),
    instruction('fill', 10, ['Run a signed intent, once.'], [
        acc('spender', 's'),
        acc('payer', 'sw', { docs: ['Funds the page of nonces if it is new; it goes back there.'] }),
        acc('profile'),
        acc('nonces', 'w'),
        fixed('systemProgram', SYSTEM),
        ...legs,
    ], [arg('amount', u64), arg('envelope', link('envelope')), arg('signature', bytes(64)), arg('terms', link('terms'))]),
    instruction('cancel', 11, ['The authority ends one thing: a policy, or with a salt one signed intent.'], [
        acc('authority', 's'),
        acc('payer', 'ew', { docs: ["A policy: its recorded payer. An intent: a signer that funds the page of nonces."] }),
        ownProfile,
        acc('account', 'w', { docs: ['The policy, or the page of nonces of the intent.'] }),
        fixed('systemProgram', SYSTEM),
        ...tail,
    ], [arg('salt', c.remainderOptionTypeNode(u64), ["The intent's salt. None for a policy."])]),
];

// Events are the data of an inner instruction to the program itself: 255, the instruction, the fields
const event = (name, code, fields) => {
    const prefix = c.constantValueNodeFromBytes('base16', `ff${code.toString(16).padStart(2, '0')}`);
    return c.eventNode({ data: c.hiddenPrefixTypeNode(c.structTypeNode(fields), [prefix]), discriminators: [c.constantDiscriminatorNode(prefix)], name });
};
const events = [
    event('created', 0, [field('policy', key), field('authority', key)]),
    event('pulled', 1, [field('policy', key), field('spender', key), field('amount', u64), field('paid', u64), field('reference', bytes(32))]),
    event('closed', 2, [field('account', key), field('closer', key)]),
    event('opened', 3, [field('authority', key), field('nonceIndex', u64)]),
    event('invalidated', 4, [field('authority', key), field('stale', u64), field('nonceIndex', u64)]),
    event('filled', 10, [field('authority', key), field('spender', key), field('salt', u64), field('amount', u64), field('paid', u64)]),
    event('cancelled', 11, [field('authority', key), field('account', key), field('salt', u64)]),
];

// The SPL delegate of every enabled token account, and the signer of every event
const constants = [c.constantNode('engineAddress', key, c.publicKeyValueNode(ENGINE))];
const root = c.rootNode(c.programNode({ accounts, constants, definedTypes: types, errors, events, instructions, name: 'solanaPull', pdas, publicKey: PROGRAM, version: '0.0.1' }));
fs.writeFileSync(`${here}idl.json`, `${JSON.stringify(root, null, 2)}\n`);
await createFromRoot(root).accept(renderVisitor(here.replace(/\/$/, ''), { erasableSyntax: true, importExtension: 'ts', kitImportStrategy: 'rootOnly', syncPackageJson: false }));
console.log(`idl.json and src/generated: ${instructions.length} instructions, ${accounts.length} accounts, ${events.length} events, ${errors.length} errors`);
