import {
    AccountRole,
    address,
    type AccountMeta,
    type Address,
    getAddressEncoder,
    getProgramDerivedAddress,
    getU64Encoder,
    type Instruction,
    type TransactionSigner,
} from '@solana/kit';

export const PULL_PROGRAM_ADDRESS = address('PULLrgDYqK1yFKVTSbWieX3ARP7U2XUyrjxWXqKgVzA');
/** The SPL delegate of every enabled token account, and the event signer. */
export const ENGINE_ADDRESS = address('58eSE1WJDvzrwz7BZ23sbsDiqa75pxetzkyRyUcPwb6E');
import { decode, termsId } from './terms.ts';

const SYSTEM = address('11111111111111111111111111111111');
const TOKEN_PROGRAM = address('TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA');

const key = (a: Address) => getAddressEncoder().encode(a);

export const findPolicyPda = async (authority: Address, id: Uint8Array) =>
    (await getProgramDerivedAddress({ programAddress: PULL_PROGRAM_ADDRESS, seeds: ['policy', key(authority), id] }))[0];

/** Nonces in one page. */
export const NONCE_BITS = 1024n;

/**
 * The page that holds the nonce of an intent: one per authority per 1,024
 * salts, kept forever. An intent's bit in it is its salt modulo 1,024.
 */
export const findNoncesPda = async (authority: Address, salt: bigint) =>
    (await getProgramDerivedAddress({ programAddress: PULL_PROGRAM_ADDRESS, seeds: ['nonces', key(authority), getU64Encoder().encode(salt / NONCE_BITS)] }))[0];

const signer = (s: TransactionSigner, role = AccountRole.READONLY_SIGNER): AccountMeta => ({ address: s.address, role, signer: s }) as AccountMeta;
const writable = (a: Address): AccountMeta => ({ address: a, role: AccountRole.WRITABLE });
const readonly = (a: Address): AccountMeta => ({ address: a, role: AccountRole.READONLY });
const tail = [readonly(ENGINE_ADDRESS), readonly(PULL_PROGRAM_ADDRESS)];

/** The address of the policy for canonical `terms`. */
export const policyAddress = async (terms: Uint8Array) => findPolicyPda(decode(terms).authority, await termsId(terms));

/**
 * Enable a token account: SPL `Approve` the engine as its delegate. `amount`
 * caps what every policy and intent on the account can pull in total; leave
 * it out for no cap beyond their own limits.
 */
export const getEnableInstruction = (p: { owner: TransactionSigner; account: Address; amount?: bigint; tokenProgram?: Address }): Instruction => ({
    accounts: [writable(p.account), readonly(ENGINE_ADDRESS), signer(p.owner)],
    data: Uint8Array.of(4, ...getU64Encoder().encode(p.amount ?? 2n ** 64n - 1n)),
    programAddress: p.tokenProgram ?? TOKEN_PROGRAM,
});

/**
 * Put a policy on chain: a standing permission its spender uses with pulls.
 * The authority signs; `payer` funds the rent and gets it back when the
 * policy closes.
 */
export const getCreateInstruction = async (p: { authority: TransactionSigner; payer: TransactionSigner; terms: Uint8Array }): Promise<Instruction> => ({
    accounts: [signer(p.authority), signer(p.payer, AccountRole.WRITABLE_SIGNER), writable(await policyAddress(p.terms)), readonly(SYSTEM), ...tail],
    data: Uint8Array.of(0, ...p.terms),
    programAddress: PULL_PROGRAM_ADDRESS,
});

type Legs = { terms: Uint8Array; to: Address; payFrom?: Address; tokenProgram?: Address; payTokenProgram?: Address };

/** The accounts a pull moves tokens between, shared by `Pull` and `Fill`. */
function legs(p: Legs): AccountMeta[] {
    const t = decode(p.terms);
    if (t.receive && !p.payFrom) throw new Error('the authority must receive something: pass payFrom');
    const payment = t.receive ? [writable(p.payFrom!), readonly(t.receive.mint), writable(t.receive.to), readonly(p.payTokenProgram ?? TOKEN_PROGRAM)] : [];
    return [writable(t.limit.from), readonly(t.limit.mint), writable(p.to), ...tail, readonly(p.tokenProgram ?? TOKEN_PROGRAM), ...payment];
}

/**
 * Take `amount` under a policy, from the token account its terms limit into
 * `to`. If the terms say what the authority receives, `payFrom` is the
 * spender's token account that pays it; the program moves the payment itself
 * and checks what arrives.
 */
export const getPullInstruction = async (p: Legs & { spender: TransactionSigner; amount: bigint; reference?: Uint8Array }): Promise<Instruction> => {
    if (p.reference && p.reference.length !== 32) throw new Error('a reference is 32 bytes');
    return {
        accounts: [signer(p.spender), writable(await policyAddress(p.terms)), ...legs(p)],
        data: Uint8Array.of(1, ...getU64Encoder().encode(p.amount), ...(p.reference ?? [])),
        programAddress: PULL_PROGRAM_ADDRESS,
    };
};

/**
 * Run a signed intent, once: `terms` the authority signed as text
 * (`message()`). Nothing goes on chain first;
 * `payer` funds the page of nonces if this intent is the first in it.
 */
export const getFillInstruction = async (
    p: Legs & { spender: TransactionSigner; payer: TransactionSigner; signature: Uint8Array; amount: bigint },
): Promise<Instruction> => {
    const t = decode(p.terms);
    return {
        accounts: [
            signer(p.spender),
            signer(p.payer, AccountRole.WRITABLE_SIGNER),
            writable(await findNoncesPda(t.authority, BigInt(t.salt))),
            readonly(SYSTEM),
            ...legs(p),
        ],
        data: Uint8Array.of(10, ...getU64Encoder().encode(p.amount), ...p.signature, ...p.terms),
        programAddress: PULL_PROGRAM_ADDRESS,
    };
};

/**
 * Use up the nonce of a signed intent with this `salt`, so it can never be
 * filled. With `page`, every nonce of its page: up to 1,024 intents at once.
 */
export const getCancelInstruction = async (p: { authority: TransactionSigner; payer: TransactionSigner; salt: bigint; page?: boolean }): Promise<Instruction> => ({
    accounts: [
        signer(p.authority),
        signer(p.payer, AccountRole.WRITABLE_SIGNER),
        writable(await findNoncesPda(p.authority.address, p.salt)),
        readonly(SYSTEM),
        ...tail,
    ],
    data: Uint8Array.of(11, ...getU64Encoder().encode(p.salt), p.page ? 1 : 0),
    programAddress: PULL_PROGRAM_ADDRESS,
});

/**
 * Close a policy (`policyAddress`) and return its rent to `payer`, the
 * account that paid it (`fetchPolicy`): its authority or spender at any
 * time, anyone after its expiry.
 */
export const getCloseInstruction = (p: { closer: TransactionSigner; account: Address; payer: Address }): Instruction => ({
    accounts: [signer(p.closer), writable(p.account), writable(p.payer), ...tail],
    data: Uint8Array.of(2),
    programAddress: PULL_PROGRAM_ADDRESS,
});
