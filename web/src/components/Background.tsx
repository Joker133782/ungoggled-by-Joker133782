/**
 * Decorative, fully transparent background: concentric radio-wave rings
 * "broadcasting" from the top-right corner, a second, smaller glow left of
 * center (so something is still visible once a phone's fullscreen crop
 * discards most of the top-right one), a faint dot grid fading toward the
 * middle, and a small secondary ring cluster bottom-left. Fixed behind all
 * content, never intercepts clicks.
 */
export function Background() {
  const rings = [
    [110, 0.172],
    [192, 0.158],
    [274, 0.145],
    [356, 0.132, "2 7"],
    [438, 0.119],
    [520, 0.107],
    [602, 0.095],
    [684, 0.083, "2 7"],
    [766, 0.073],
    [848, 0.062],
    [930, 0.053],
    [1012, 0.043, "2 7"],
    [1094, 0.035],
    [1176, 0.027],
    [1258, 0.021],
    [1340, 0.015, "2 7"],
  ] as const;
  const midRings = [
    [70, 0.15],
    [130, 0.13],
    [190, 0.11, "2 7"],
    [250, 0.09],
    [310, 0.07],
    [370, 0.05, "2 7"],
    [430, 0.035],
  ] as const;
  const cornerRings = [
    [90, 0.08],
    [160, 0.068],
    [230, 0.057],
    [300, 0.045],
    [370, 0.033],
    [440, 0.022],
  ] as const;
  return (
    <svg
      viewBox="0 0 1600 1000"
      preserveAspectRatio="xMaxYMin slice"
      fill="none"
      aria-hidden="true"
      className="site-background"
    >
      <defs>
        <pattern
          id="ug-dots"
          width="24"
          height="24"
          patternUnits="userSpaceOnUse"
        >
          <circle cx="12" cy="12" r="1" fill="#8a93a3" />
        </pattern>
        <radialGradient
          id="ug-fade"
          cx="1470"
          cy="70"
          r="1100"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stopColor="#fff" stopOpacity=".9" />
          <stop offset=".55" stopColor="#fff" stopOpacity=".25" />
          <stop offset="1" stopColor="#fff" stopOpacity="0" />
        </radialGradient>
        <mask id="ug-mask">
          <rect width="1600" height="1000" fill="url(#ug-fade)" />
        </mask>
        <radialGradient
          id="ug-glow"
          cx="1470"
          cy="70"
          r="520"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stopColor="var(--accent)" stopOpacity=".1" />
          <stop offset="1" stopColor="var(--accent)" stopOpacity="0" />
        </radialGradient>
        <radialGradient
          id="ug-glow-mid"
          cx="1250"
          cy="480"
          r="380"
          gradientUnits="userSpaceOnUse"
        >
          <stop offset="0" stopColor="var(--accent)" stopOpacity=".09" />
          <stop offset="1" stopColor="var(--accent)" stopOpacity="0" />
        </radialGradient>
      </defs>
      <rect
        width="1600"
        height="1000"
        fill="url(#ug-dots)"
        opacity=".16"
        mask="url(#ug-mask)"
      />
      <rect width="1600" height="1000" fill="url(#ug-glow)" />
      <rect width="1600" height="1000" fill="url(#ug-glow-mid)" />
      {rings.map(([r, o, dash]) => (
        <circle
          key={`tr-${r}`}
          cx="1470"
          cy="70"
          r={r}
          stroke="var(--accent)"
          strokeOpacity={o}
          strokeWidth="1"
          strokeDasharray={dash}
        />
      ))}
      {midRings.map(([r, o, dash]) => (
        <circle
          key={`mid-${r}`}
          cx="1250"
          cy="480"
          r={r}
          stroke="var(--accent)"
          strokeOpacity={o}
          strokeWidth="1"
          strokeDasharray={dash}
        />
      ))}
      {cornerRings.map(([r, o]) => (
        <circle
          key={`bl-${r}`}
          cx="60"
          cy="1040"
          r={r}
          stroke="var(--accent)"
          strokeOpacity={o}
          strokeWidth="1"
        />
      ))}
    </svg>
  );
}
