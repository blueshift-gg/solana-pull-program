// `npm run e2e`: every journey, through this client, on a Surfpool fork of
// mainnet. Build the program first:
// `cargo build-sbf --manifest-path program/Cargo.toml --features localnet`.
import { fork } from './fork.ts';
import { order } from './order.ts';
import { payout } from './payout.ts';
import { subscription } from './subscription.ts';
import { undo } from './undo.ts';

const stop = await fork();
try {
    for (const journey of [subscription, order, payout, undo]) {
        console.log(`\n${journey.name}`);
        await journey();
    }
    console.log('\nEvery journey passed.');
} finally {
    stop();
}
