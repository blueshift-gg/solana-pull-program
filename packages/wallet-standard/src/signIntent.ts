import type { SolanaSignOffchainMessageOutput } from '@solana/wallet-standard-features';
import type { ReadonlyUint8Array, WalletAccount } from '@wallet-standard/base';

/** Name of the feature. */
export const SolanaSignIntent = 'solana:signIntent';

/**
 * `solana:signIntent` is a feature that may be implemented by a {@link "@wallet-standard/base".Wallet}
 * to allow a dapp to request the wallet to sign intents: one-time spending permissions the Pull
 * program enforces on chain.
 *
 * The dapp sends the canonical terms, never text. The wallet decodes and validates them, reads each
 * mint's decimals and the account's profile from its own RPC, renders the canonical text itself
 * under the profile's nonce index and shows it. For terms with
 * no expiry (`until revoked`) it asks for a separate, explicit confirmation.
 * It then signs that text as an Offchain Message v1, so the output is exactly what
 * `solana:signOffchainMessage` would return for the same text, and what the program verifies.
 */
export type SolanaSignIntentFeature = {
    /** Name of the feature. */
    readonly [SolanaSignIntent]: {
        /** Version of the feature API. */
        readonly version: SolanaSignIntentVersion;

        /** intent terms format versions this wallet can decode, render and sign. */
        readonly supportedTermsVersions: readonly SolanaIntentTermsVersion[];

        /**
         * Sign intents using the account's secret key.
         *
         * @param inputs Intents to sign.
         *
         * @return Results of signing. For each intent this includes the full Offchain Message v1
         * bytes the wallet rendered and signed, along with the resulting signature.
         */
        readonly signIntent: SolanaSignIntentMethod;
    };
};

/** Version of the feature. */
export type SolanaSignIntentVersion = '1.0.0';

/** intent terms format version (the first byte of the canonical terms). */
export type SolanaIntentTermsVersion = 1;

/**
 * Signs one or more intents, returning for each the full bytes the wallet constructed and signed
 * along with the resulting signature.
 *
 * @param inputs Intents to sign.
 *
 * @return Results of signing intents.
 */
export type SolanaSignIntentMethod = (
    ...inputs: readonly SolanaSignIntentInput[]
) => Promise<readonly SolanaSignIntentOutput[]>;

/** Input for signing an intent. */
export interface SolanaSignIntentInput {
    /**
     * Account to use.
     * Must be the authority named in `terms`; the wallet rejects the request otherwise.
     */
    readonly account: WalletAccount;

    /**
     * Canonical intent terms. The wallet must reject bytes that are not a canonical encoding of
     * valid terms for the cluster the account is on.
     */
    readonly terms: ReadonlyUint8Array;
}

/**
 * Output of signing an intent: the Offchain Message v1 the wallet signed, with the canonical text
 * as its body, and the Ed25519 signature. `terms` plus `signature` is the portable approval
 * any executor can verify and fill.
 */
export type SolanaSignIntentOutput = SolanaSignOffchainMessageOutput;
