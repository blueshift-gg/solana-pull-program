import type { SolanaSignOffchainMessageOutput } from '@solana/wallet-standard-features';
import type { WalletAccount, WalletWithFeatures } from '@wallet-standard/base';

import type {
    SolanaSignAuthorizationFeature,
    SolanaSignAuthorizationInput,
    SolanaSignAuthorizationOutput,
} from '../signAuthorization.js';
import { SolanaSignAuthorization } from '../signAuthorization.js';

const account = null as unknown as WalletAccount;

// [DESCRIBE] `SolanaSignAuthorizationInput`
{
    // The account and the canonical terms are all a dapp sends: no text.
    {
        ({ account, terms: new Uint8Array() }) satisfies SolanaSignAuthorizationInput;
    }

    // Text is not an input.
    {
        // @ts-expect-error The wallet renders the text itself.
        ({ account, terms: new Uint8Array(), message: '' }) satisfies SolanaSignAuthorizationInput;
    }
}

// [DESCRIBE] `SolanaSignAuthorizationOutput`
{
    // It is exactly the Offchain Message output, so verifiers need nothing new.
    {
        const output = null as unknown as SolanaSignAuthorizationOutput;
        output satisfies SolanaSignOffchainMessageOutput;
    }
}

// [DESCRIBE] Feature detection
{
    // A dapp narrows a wallet to one that supports the feature, then calls it.
    {
        const wallet = null as unknown as WalletWithFeatures<SolanaSignAuthorizationFeature>;
        const feature = wallet.features[SolanaSignAuthorization];
        feature.supportedTermsVersions satisfies readonly 1[];
        void feature.signAuthorization({ account, terms: new Uint8Array() });
    }
}
