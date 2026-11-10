import { test } from 'node:test';
import assert from 'node:assert/strict';
import { is4k60, modeLabel } from '../src/format.ts';
test('only progressive 4K at 50 Hz or more counts as 4K60', () => {
  assert.equal(is4k60('3840x2160@60'), true);
  assert.equal(is4k60('3840x2160@50'), true);
  assert.equal(is4k60('4096x2160@60'), true);
  assert.equal(is4k60('3840x2160@30'), false);
  assert.equal(is4k60('2560x1440@144'), false);
  assert.equal(is4k60('1920x1080@60'), false);
  assert.equal(is4k60('3840x2160@60i'), false);
  assert.equal(is4k60('auto'), false);
});
test('mode labels still read "W × H · N Hz"', () => {
  assert.equal(modeLabel('3840x2160@60'), '3840 × 2160 · 60 Hz');
});
