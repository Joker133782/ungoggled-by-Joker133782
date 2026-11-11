import { test } from 'node:test';
import assert from 'node:assert/strict';
import { withTimeout } from '../src/api.ts';
const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms));
test('aborts after the timeout', async () => {
  const { signal, done } = withTimeout(null, 10);
  assert.equal(signal.aborted, false);
  await sleep(30);
  assert.equal(signal.aborted, true);
  done();
});
test('follows the parent signal, also one aborted already', () => {
  const parent = new AbortController();
  const request = withTimeout(parent.signal, 10000);
  parent.abort();
  assert.equal(request.signal.aborted, true);
  request.done();
  const late = withTimeout(parent.signal, 10000);
  assert.equal(late.signal.aborted, true);
  late.done();
});
test('done() stops the timer and detaches from the parent', async () => {
  const parent = new AbortController();
  const { signal, done } = withTimeout(parent.signal, 10);
  done();
  await sleep(30);
  parent.abort();
  assert.equal(signal.aborted, false);
});
