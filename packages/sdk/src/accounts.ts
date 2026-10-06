import { type Address, type Base58EncodedBytes, getAddressDecoder, getBase64Encoder, type Rpc, type SolanaRpcApi } from '@solana/kit';

import { ENGINE_ADDRESS, findNoncesPda, findPolicyPda, NONCE_BITS, PULL_PROGRAM_ADDRESS } from './program.ts';
import { decode, termsId } from './terms.ts';

const read = async (rpc: Rpc<SolanaRpcApi>, account: Parameters<Rpc<SolanaRpcApi>['getAccountInfo']>[0]) => {
    const { value } = await rpc.getAccountInfo(account, { encoding: 'base64' }).send();
    return value ? new Uint8Array(getBase64Encoder().encode(value.data[0])) : null;
};

/**
 * A policy's account: null if it is not on chain (never created, or closed).
 * `spent` is what its limit has consumed at `now` (pass the Clock sysvar's
 * time): a periodic limit starts every window at zero, as in the program.
 */
export async function fetchPolicy(rpc: Rpc<SolanaRpcApi>, terms: Uint8Array, now: number) {
    const t = decode(terms);
    const data = await read(rpc, await findPolicyPda(t.authority, await termsId(terms)));
    if (!data) return null;
    const view = new DataView(data.buffer, data.byteOffset);
    // tag, rolled: i64, consumed: u64, payer
    const [rolled, consumed] = [Number(view.getBigInt64(1, true)), view.getBigUint64(9, true)];
    const { per } = t.limit;
    const window = (at: number) => (per === 'total' ? 0 : Math.floor((at - t.notBefore) / per.every));
    return { payer: getAddressDecoder().decode(data.subarray(17, 49)), spent: window(now) === window(rolled) ? consumed : 0n };
}

/**
 * Whether policies and intents can pull from token account `account`, and
 * how much. Not `enabled` means the engine is not its delegate: it was never
 * enabled, another `Approve` took its place, or a capped approval ran out.
 * `getEnableInstruction` restores it, and every policy works again as it was.
 */
export async function fetchSeat(rpc: Rpc<SolanaRpcApi>, account: Address) {
    const data = await read(rpc, account);
    if (!data) return null;
    const view = new DataView(data.buffer, data.byteOffset);
    // mint, owner, amount: u64, delegate: COption<Pubkey> at 72, ..., delegated_amount: u64 at 121
    const enabled = view.getUint32(72, true) === 1 && getAddressDecoder().decode(data.subarray(76, 108)) === ENGINE_ADDRESS;
    return { allowance: enabled ? view.getBigUint64(121, true) : 0n, balance: view.getBigUint64(64, true), enabled };
}

/**
 * Every policy on chain that `authority` created, or that names `spender`:
 * what a wallet lists for its owner, and a merchant for itself. Each comes
 * with its address, its canonical terms (`decode`) and who paid its rent.
 */
export async function fetchPolicies(rpc: Rpc<SolanaRpcApi>, by: { authority: Address } | { spender: Address }) {
    // header (51 bytes), version, cluster, authority at 53, spender: option tag at 85, key at 86
    const [offset, key] = 'authority' in by ? [53n, by.authority] : [86n, by.spender];
    const filters = [{ memcmp: { bytes: key as string as Base58EncodedBytes, encoding: 'base58' as const, offset } }];
    const accounts = await rpc.getProgramAccounts(PULL_PROGRAM_ADDRESS, { encoding: 'base64', filters }).send();
    return accounts.flatMap(({ account, pubkey }) => {
        const data = new Uint8Array(getBase64Encoder().encode(account.data[0]));
        // tag 1 is a policy; a spender's key follows only a set option tag
        if (data[0] !== 1 || ('spender' in by && data[85] !== 1)) return [];
        return [{ address: pubkey, payer: getAddressDecoder().decode(data.subarray(17, 49)), terms: data.subarray(51) }];
    });
}

/** The used-nonce bits of one page, or null if the page is not on chain. */
async function fetchNonces(rpc: Rpc<SolanaRpcApi>, authority: Address, salt: bigint) {
    // tag, authority, page: u64, bits
    return (await read(rpc, await findNoncesPda(authority, salt)))?.subarray(41) ?? null;
}

const used = (bits: Uint8Array | null, salt: bigint) => {
    const bit = Number(salt % NONCE_BITS);
    return !!bits && (bits[bit >> 3] & (1 << (bit & 7))) !== 0;
};

/** Whether a signed intent's nonce is used: it ran, or its authority cancelled it. */
export async function fetchIntentUsed(rpc: Rpc<SolanaRpcApi>, terms: Uint8Array): Promise<boolean> {
    const t = decode(terms);
    return used(await fetchNonces(rpc, t.authority, BigInt(t.salt)), BigInt(t.salt));
}

/**
 * The lowest salt whose nonce is free for a new intent of `authority`, from
 * `from` on. Salts handed out in sequence share pages, so an owner pays for
 * one page per 1,024 intents. `skip` are salts already given to intents that
 * are signed but not yet on chain.
 */
export async function nextSalt(rpc: Rpc<SolanaRpcApi>, authority: Address, skip: string[] = [], from = 0n): Promise<string> {
    for (let page = from / NONCE_BITS; ; page++) {
        const bits = await fetchNonces(rpc, authority, page * NONCE_BITS);
        for (let salt = page * NONCE_BITS; salt < (page + 1n) * NONCE_BITS; salt++) {
            if (salt >= from && !used(bits, salt) && !skip.includes(salt.toString())) return salt.toString();
        }
    }
}
