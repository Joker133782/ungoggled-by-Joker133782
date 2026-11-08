import { test } from 'node:test';
import assert from 'node:assert/strict';
import { qrSvgPath, utf8Bytes, wifiQrText } from '../src/wifi-qr.ts';
test('Wi-Fi QR text follows the WIFI: format', () => {
  assert.equal(wifiQrText('ungoggled', 'ungoggled'), 'WIFI:S:ungoggled;T:WPA;P:ungoggled;;');
  assert.equal(wifiQrText('Open Field', null), 'WIFI:S:Open Field;T:nopass;;');
});
test('reserved characters are escaped, everything else is kept', () => {
  assert.equal(
    wifiQrText('a;b,c:d"e\\f', 'p;w,d:"\\ x'),
    'WIFI:S:a\\;b\\,c\\:d\\"e\\\\f;T:WPA;P:p\\;w\\,d\\:\\"\\\\ x;;',
  );
});
test('QR bytes are UTF-8', () => {
  assert.deepEqual(utf8Bytes('Café'), [0x43, 0x61, 0x66, 0xc3, 0xa9]);
});
test('QR codes are drawn with a quiet zone, in whole modules', () => {
  const { size, path } = qrSvgPath(wifiQrText('ungoggled', 'ungoggled'));
  // Version 2 or 3 (25 or 29 modules) plus 4 modules of quiet zone per side.
  assert.ok([33, 37].includes(size), `size ${size}`);
  const cells = [...path.matchAll(/M(\d+) (\d+)h1v1h-1z/g)];
  assert.equal(cells.map((m) => m[0]).join(''), path, 'only whole-module squares');
  assert.ok(cells.every(([, x, y]) => x >= 4 && y >= 4 && x < size - 4 && y < size - 4));
  // The three finder patterns' corners are dark.
  for (const [x, y] of [[4, 4], [size - 5, 4], [4, size - 5]])
    assert.ok(path.includes(`M${x} ${y}h`), `finder at ${x},${y}`);
});
