export const number = (v: number | null | undefined, digits = 1) =>
  v == null ? "—" : v.toFixed(digits);
export const size = (w?: number, h?: number) => (w && h ? `${w} × ${h}` : "—");
export const scan = (interlaced?: boolean) => (interlaced ? " interlaced" : "");
// "1920x1080@60i" → "1920 × 1080 · 60 Hz interlaced"
export function modeLabel(mode: string): string {
  const m = /^(\d+)x(\d+)@(\d+)(i?)$/.exec(mode);
  if (!m) return mode;
  return `${m[1]} × ${m[2]} · ${m[3]} Hz${scan(m[4] === "i")}`;
}
// A mode of 3840 pixels or more across at 50 Hz or more ("3840x2160@60"),
// which only the first HDMI port can do and which needs a good cable.
export function is4k60(mode: string): boolean {
  const m = /^(\d+)x(\d+)@(\d+)(i?)$/.exec(mode);
  return !!m && Number(m[1]) >= 3840 && Number(m[3]) >= 50 && m[4] === "";
}
export function agoLabel(seconds: number): string {
  const s = Math.max(0, Math.round(seconds));
  if (s < 60) return `${s}s ago`;
  const m = Math.floor(s / 60);
  const rem = s % 60;
  return rem === 0 ? `${m} min ago` : `${m}m ${rem}s ago`;
}
export function skewLabel(deltaMs: number): string {
  const behind = deltaMs > 0;
  const sec = Math.max(1, Math.round(Math.abs(deltaMs) / 1000));
  let amount: string;
  if (sec < 60) amount = `${sec} second${sec === 1 ? "" : "s"}`;
  else if (sec < 3600) {
    const m = Math.round(sec / 60);
    amount = `${m} minute${m === 1 ? "" : "s"}`;
  } else if (sec < 86400) {
    const h = Math.round(sec / 3600);
    amount = `${h} hour${h === 1 ? "" : "s"}`;
  } else {
    const d = Math.round(sec / 86400);
    amount = `${d} day${d === 1 ? "" : "s"}`;
  }
  return behind ? `${amount} behind` : `${amount} ahead`;
}
