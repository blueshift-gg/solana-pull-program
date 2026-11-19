// The client against the Rust core, on the random terms `vectors` prints
// (`npm test` writes them to `test/vectors.jsonl` first): the same bytes,
// and the same verdict on validity.
import assert from 'node:assert/strict';
import fs from 'node:fs';
import test from 'node:test';

import { getTermsCodec, isValid } from '../src/index.ts';

type Vector = { bytes: string; valid: boolean };
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
