import type { SolanaSignOffchainMessageOutput } from '@solana/wallet-standard-features';
import type { ReadonlyUint8Array, WalletAccount } from '@wallet-standard/base';

/** Name of the feature. */
export const SolanaSignAuthorization = 'solana:signAuthorization';

/**
 * `solana:signAuthorization` is a feature that may be implemented by a {@link "@wallet-standard/base".Wallet}
 * to allow a dapp to request the wallet to sign authorizations: one-time spending permissions the Pull
 * program enforces on chain.
 *
 * The dapp sends the canonical terms, never text. The wallet decodes and validates them, reads each
 * mint's decimals and the account's profile from its own RPC, renders the canonical text itself
 * under the profile's nonce index and shows it. For terms with
 * no expiry (`until revoked`) it asks for a separate, explicit confirmation.
 * It then signs that text as an Offchain Message v1, so the output is exactly what
 * `solana:signOffchainMessage` would return for the same text, and what the program verifies.
 */
export type SolanaSignAuthorizationFeature = {
    /** Name of the feature. */
    readonly [SolanaSignAuthorization]: {
        /** Version of the feature API. */
        readonly version: SolanaSignAuthorizationVersion;

        /** authorization terms format versions this wallet can decode, render and sign. */
        readonly supportedTermsVersions: readonly SolanaAuthorizationTermsVersion[];

        /**
         * Sign authorizations using the account's secret key.
         *
         * @param inputs Authorizations to sign.
         *
         * @return Results of signing. For each authorization this includes the full Offchain Message v1
         * bytes the wallet rendered and signed, along with the resulting signature.
         */
        readonly signAuthorization: SolanaSignAuthorizationMethod;
    };
};

/** Version of the feature. */
export type SolanaSignAuthorizationVersion = '1.0.0';

/** authorization terms format version (the first byte of the canonical terms). */
export type SolanaAuthorizationTermsVersion = 1;

/**
 * Signs one or more authorizations, returning for each the full bytes the wallet constructed and signed
 * along with the resulting signature.
 *
 * @param inputs Authorizations to sign.
 *
 * @return Results of signing authorizations.
 */
export type SolanaSignAuthorizationMethod = (
    ...inputs: readonly SolanaSignAuthorizationInput[]
) => Promise<readonly SolanaSignAuthorizationOutput[]>;

/** Input for signing an authorization. */
export interface SolanaSignAuthorizationInput {
    /**
     * Account to use.
     * Must be the authority named in `terms`; the wallet rejects the request otherwise.
     */
    readonly account: WalletAccount;

    /**
     * Canonical authorization terms. The wallet must reject bytes that are not a canonical encoding of
     * valid terms for the cluster the account is on.
     */
    readonly terms: ReadonlyUint8Array;
}

/**
 * Output of signing an authorization: the Offchain Message v1 the wallet signed, with the canonical text
 * as its body, and the Ed25519 signature. `terms` plus `signature` is the portable approval
 * any executor can verify and fill.
 */
export type SolanaSignAuthorizationOutput = SolanaSignOffchainMessageOutput;
