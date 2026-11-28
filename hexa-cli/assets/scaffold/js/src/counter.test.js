/**
 * The gate. `npm test` must exit 0 on a freshly scaffolded project with no
 * install step: Node's own test runner is the whole toolchain.
 *
 * Gate-first development: this file is the spec. If you change what the
 * project should do, change this first.
 */
import assert from 'node:assert/strict';
import { test } from 'node:test';

import { Count } from './core/domain/count.js';
import { increment } from './core/usecases/increment.js';
import { counter, incrementOnce } from './composition-root.js';

test('a new count starts at zero', () => {
  assert.equal(Count.zero().value(), 0);
});

test('incrementing twice gives two', () => {
  const store = counter();
  increment(store);
  assert.equal(increment(store).value(), 2);
});

test('the wired application increments', () => {
  assert.equal(incrementOnce().value(), 1);
});

test('a count saturates rather than wrapping', () => {
  const top = new Count(Number.MAX_SAFE_INTEGER);
  assert.equal(top.next().value(), Number.MAX_SAFE_INTEGER);
});
