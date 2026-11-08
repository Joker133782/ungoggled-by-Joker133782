import qrcode from "qrcode-generator";

/**
 * The text of a Wi-Fi QR code (`WIFI:S:<ssid>;T:WPA;P:<password>;;`, the
 * format phone cameras offer to join), with the characters that format
 * reserves escaped. No password means an open network.
 */
export function wifiQrText(ssid: string, password: string | null): string {
  const esc = (s: string) => s.replace(/([\\;,:"])/g, "\\$1");
  return password
    ? `WIFI:S:${esc(ssid)};T:WPA;P:${esc(password)};;`
    : `WIFI:S:${esc(ssid)};T:nopass;;`;
}

/** UTF-8 bytes, which is what phone scanners expect in a QR code. */
export function utf8Bytes(s: string): number[] {
  return Array.from(new TextEncoder().encode(s));
}

// Encode as UTF-8 rather than the library's default of one byte per UTF-16
// unit, which garbles a network name like "Café".
qrcode.stringToBytes = utf8Bytes;

/**
 * `text` as a QR code: an SVG path of its dark modules, one unit each, inside
 * a `size` × `size` square that includes the standard 4-module quiet zone.
 * Medium error correction, smallest version that fits.
 */
export function qrSvgPath(text: string): { size: number; path: string } {
  const qr = qrcode(0, "M");
  qr.addData(text, "Byte");
  qr.make();
  const n = qr.getModuleCount();
  let path = "";
  for (let row = 0; row < n; row++)
    for (let col = 0; col < n; col++)
      if (qr.isDark(row, col)) path += `M${col + 4} ${row + 4}h1v1h-1z`;
  return { size: n + 8, path };
}
