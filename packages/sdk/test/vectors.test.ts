// The client against the Rust core, on the random terms `vectors` prints
// (`npm test` writes them to `test/vectors.jsonl` first): the same bytes,
// the same verdict on validity, and the same intent text, both ways.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';

import { Envelope, getTermsCodec, intentMessage, isValid, parseIntent, renderIntent } from '../src/index.ts';

type Vector = { bytes: string; valid: boolean; index: string; decimals: [number, number]; text?: string };
const vectors: Vector[] = fs.readFileSync(new URL('./vectors.jsonl', import.meta.url), 'utf8').trim().split('\n').map((line) => JSON.parse(line));
const hex = (s: string) => Uint8Array.from(s.match(/../g) ?? [], (b) => parseInt(b, 16));
const codec = getTermsCodec();

test('terms encode to the bytes the program reads', () => {
    for (const v of vectors) assert.deepEqual(new Uint8Array(codec.encode(codec.decode(hex(v.bytes)))), hex(v.bytes));
});

test('terms are valid exactly when the program says so', () => {
    for (const v of vectors) assert.equal(isValid(codec.decode(hex(v.bytes))), v.valid, v.bytes);
    assert.ok(vectors.filter((v) => v.valid).length > 500 && vectors.filter((v) => !v.valid).length > 500);
});

test('the intent text is the one the program verifies, and reads back to the same terms', () => {
    for (const v of vectors.filter((v) => v.text)) {
        const terms = codec.decode(hex(v.bytes));
        const receive = terms.receive.__option === 'Some' ? terms.receive.value : null;
        // The received mint's decimals win when both legs name one mint, as in the program
        const decimals = { [terms.limit.mint]: v.decimals[0], ...(receive ? { [receive.mint]: v.decimals[1] } : {}) };
        const text = new TextDecoder().decode(hex(v.text!));
        assert.equal(renderIntent(terms, BigInt(v.index), decimals), text);

        const parsed = parseIntent(text);
        assert.ok(parsed, text);
        assert.deepEqual(new Uint8Array(codec.encode(parsed.terms)), hex(v.bytes));
        assert.deepEqual([parsed.nonceIndex, parsed.decimals], [BigInt(v.index), decimals]);
        const [raw, wrapped] = [Envelope.Text, Envelope.OffchainMessage].map((envelope) => intentMessage(terms, BigInt(v.index), decimals, envelope));
        assert.equal(new TextDecoder().decode(raw), text);
        assert.equal(new TextDecoder().decode(wrapped.subarray(50)), text);
    }
});

test('a text that is not canonical is not an intent', () => {
    const v = vectors.find((v) => v.text)!;
    const text = new TextDecoder().decode(hex(v.text!));
    assert.ok(parseIntent(text));
    for (const forged of [text + '\n', ` ${text}`, text.replace('SALT: ', 'SALT: 0'), text.replace('Solana Pull v1', 'Solana Pull v2'), text.replace(/engine: \w+/, 'engine: 11111111111111111111111111111111')]) {
        assert.equal(parseIntent(forged), null, forged);
    }
});
