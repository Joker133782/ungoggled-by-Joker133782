import { useEffect, useMemo, useState, type PointerEvent } from "react";
import type { History, Sample } from "../types";
import { api, withTimeout } from "../api";
import { agoLabel, number } from "../format";
import { GRAPH_GROUPS, useGraphGroups, type GraphGroup } from "../graph-groups";
import { PanelHead } from "./PanelHead";
import { IconChart } from "./icons";

/** Seconds shown: everything the Pi keeps (in RAM, so a restart clears it). */
const SPAN = 1800;
const POLL_MS = 2000;
/** Samples further apart than this are a gap in the line, not a slope. */
const GAP = 5;

type Series = { label: string; value: (s: Sample) => number | null };
type ChartSpec = {
  id: string;
  title: string;
  unit: string;
  digits: number;
  series: Series[];
  /** Smallest top of the y-axis, so a quiet half hour doesn't look dramatic. */
  floor: number;
  /** A fixed top instead (percentages). */
  top?: number;
  /** Warning threshold for the first series, with what it means. */
  warn?: number;
  warnText?: string;
  /** Shade the stretches without a live picture. */
  outages?: boolean;
};

const CHARTS: Record<GraphGroup, ChartSpec[]> = {
  video: [
    {
      id: "bitrate",
      title: "Bitrate",
      unit: "Mbps",
      digits: 1,
      floor: 10,
      outages: true,
      series: [{ label: "From the goggles", value: (s) => s.bitrate_mbps }],
    },
    {
      id: "fps",
      title: "Frame rate",
      unit: "fps",
      digits: 0,
      floor: 60,
      outages: true,
      series: [
        { label: "Camera", value: (s) => s.input_fps },
        { label: "TV", value: (s) => s.output_fps },
      ],
    },
  ],
  health: [
    {
      id: "temperature",
      title: "Temperature",
      unit: "°C",
      digits: 0,
      floor: 80,
      warn: 75,
      warnText: "The Pi is running hot",
      series: [{ label: "Processor", value: (s) => s.temperature_c }],
    },
    {
      id: "cpu",
      title: "CPU",
      unit: "%",
      digits: 0,
      floor: 100,
      top: 100,
      warn: 90,
      warnText: "The processor is close to its limit",
      series: [{ label: "Busy", value: (s) => s.cpu_percent }],
    },
    {
      id: "ram",
      title: "RAM",
      unit: "%",
      digits: 0,
      floor: 100,
      top: 100,
      warn: 90,
      warnText: "Memory is nearly full",
      series: [
        {
          label: "In use",
          value: (s) =>
            s.ram_total_mib > 0
              ? (s.ram_used_mib / s.ram_total_mib) * 100
              : null,
        },
      ],
    },
  ],
  network: [
    {
      id: "wifi",
      title: "Wi-Fi",
      unit: "Mbps",
      digits: 1,
      floor: 10,
      warn: 40,
      warnText: "Wi-Fi is sending close to what the Pi's radio can carry",
      series: [
        { label: "Out", value: (s) => s.wifi_tx_mbps },
        { label: "In", value: (s) => s.wifi_rx_mbps },
      ],
    },
    {
      id: "lan",
      title: "Ethernet",
      unit: "Mbps",
      digits: 1,
      floor: 10,
      series: [
        { label: "Out", value: (s) => s.lan_tx_mbps },
        { label: "In", value: (s) => s.lan_rx_mbps },
      ],
    },
  ],
  sd: [
    {
      id: "sd",
      title: "SD card",
      unit: "MB/s",
      digits: 1,
      floor: 5,
      series: [
        { label: "Write", value: (s) => s.disk_write_kib_s / 1024 },
        { label: "Read", value: (s) => s.disk_read_kib_s / 1024 },
      ],
    },
  ],
};

type Buffer = { instance: number; now: number; samples: Sample[] };
/**
 * The Pi's last 30 minutes: fetched whole once, then only the samples newer
 * than the last one held (`?since=`), so an open System tab costs a few
 * hundred bytes every couple of seconds instead of the whole history.
 */
function useHistory(): Buffer | null {
  const [buffer, setBuffer] = useState<Buffer | null>(null);
  useEffect(() => {
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let held: Buffer | null = null;
    const poll = async () => {
      const request = withTimeout(abort.signal, 8000);
      try {
        const since = held?.samples[held.samples.length - 1]?.t;
        const h = await api<History>(
          since == null ? "history" : `history?since=${since}`,
          { signal: request.signal },
        );
        if (abort.signal.aborted) return;
        if (held && h.instance !== held.instance) {
          // The service restarted, so `since` meant the previous run: start
          // over with everything from this one.
          held = null;
          timer = setTimeout(poll, 0);
          return;
        }
        const merged =
          held && since != null ? held.samples.concat(h.samples) : h.samples;
        const first = merged.findIndex((s) => s.t >= h.now - SPAN);
        held = {
          instance: h.instance,
          now: h.now,
          samples: first < 0 ? [] : merged.slice(first),
        };
        setBuffer(held);
      } catch {
        /* The next poll retries. */
      } finally {
        request.done();
      }
      if (!abort.signal.aborted) timer = setTimeout(poll, POLL_MS);
    };
    void poll();
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, []);
  return buffer;
}

/** The sample taken closest to `t`, if there is one within a few seconds. */
function nearest(samples: Sample[], t: number): Sample | null {
  if (samples.length === 0) return null;
  let lo = 0;
  let hi = samples.length - 1;
  while (lo < hi) {
    const mid = (lo + hi) >> 1;
    if (samples[mid].t < t) lo = mid + 1;
    else hi = mid;
  }
  const prev = samples[lo - 1];
  const best =
    prev && Math.abs(prev.t - t) < Math.abs(samples[lo].t - t)
      ? prev
      : samples[lo];
  return Math.abs(best.t - t) <= GAP ? best : null;
}
/** Stretches without a live picture, merged into [from, to) spans. */
function outageSpans(samples: Sample[]): [number, number][] {
  const spans: [number, number][] = [];
  for (const s of samples) {
    if (s.hdmi !== "fallback" && s.phase === "streaming") continue;
    const last = spans[spans.length - 1];
    if (last && s.t - last[1] <= 1.5) last[1] = s.t + 1;
    else spans.push([s.t, s.t + 1]);
  }
  return spans;
}
/** Rounds an axis top up to 1, 2 or 5 times a power of ten. */
function nice(v: number): number {
  const p = 10 ** Math.floor(Math.log10(v));
  const m = v / p;
  return (m <= 1 ? 1 : m <= 2 ? 2 : m <= 5 ? 5 : 10) * p;
}
const finite = (v: number | null): v is number =>
  v != null && Number.isFinite(v);

/**
 * The Pi's last half hour as small multiples: one compact graph per
 * measurement, grouped (Video, Pi health, Network, SD card; each group can be
 * switched off in Settings → Graphs), all on the same time axis. Touching or
 * hovering one graph reads the same moment off all of them. Values past a
 * warning threshold turn amber and are spelled out at the top.
 */
export function Graphs() {
  const history = useHistory();
  const [groups] = useGraphGroups();
  const [hover, setHover] = useState<number | null>(null);
  const samples = useMemo(() => history?.samples ?? [], [history]);
  const now = history?.now ?? 0;
  const begin = now - SPAN;
  const latest = samples[samples.length - 1] ?? null;
  const at = hover == null ? latest : nearest(samples, hover);
  const outages = useMemo(() => outageSpans(samples), [samples]);
  const shown = GRAPH_GROUPS.filter((g) => groups[g.id]);
  const warnings = shown.flatMap((g) =>
    CHARTS[g.id].flatMap((c) => {
      const v = latest ? c.series[0].value(latest) : null;
      return c.warn != null && finite(v) && v >= c.warn
        ? [
            `${c.warnText}: ${number(v, c.digits)} ${c.unit} (warning from ${c.warn} ${c.unit}).`,
          ]
        : [];
    }),
  );
  return (
    <section className="panel graphs" id="graphs">
      <PanelHead icon={<IconChart />} title="Last 30 minutes" wiki="graphs" />
      <div className="graph-caption">
        <span>
          {hover != null
            ? `${agoLabel(now - hover)}${at ? "" : " · no data from then"}`
            : !history
              ? "Loading…"
              : now < SPAN
                ? `Newest on the right; the Pi started ${Math.max(1, Math.round(now / 60))} min ago`
                : "Newest on the right. Touch a graph to read its values."}
        </span>
        {hover != null ? (
          <button className="link-button" onClick={() => setHover(null)}>
            Back to now
          </button>
        ) : (
          <span>
            <i className="outage-key" aria-hidden="true" /> no live picture
          </span>
        )}
      </div>
      {warnings.map((w) => (
        <p key={w} className="graph-warning" role="status">
          {w}
        </p>
      ))}
      {shown.length === 0 && (
        <p className="status-line">
          Every graph is switched off in{" "}
          <a href="#settings/graph-settings">Settings → Graphs</a>.
        </p>
      )}
      {shown.map((g) => (
        <div className="graph-group" key={g.id}>
          <h3>{g.title}</h3>
          <div className="graph-grid">
            {CHARTS[g.id].map((c) => (
              <Chart
                key={c.id}
                spec={c}
                samples={samples}
                begin={begin}
                at={at}
                cursor={hover}
                outages={c.outages ? outages : []}
                onHover={setHover}
              />
            ))}
          </div>
        </div>
      ))}
    </section>
  );
}

const W = 300;
const H = 60;
function Chart({
  spec,
  samples,
  begin,
  at,
  cursor,
  outages,
  onHover,
}: {
  spec: ChartSpec;
  samples: Sample[];
  begin: number;
  /** The sample whose values the header shows: hovered, else the latest. */
  at: Sample | null;
  cursor: number | null;
  outages: [number, number][];
  onHover: (t: number | null) => void;
}) {
  const values = useMemo(
    () => spec.series.map((s) => samples.map(s.value)),
    [spec, samples],
  );
  const readings = values.some((vs) => vs.some(finite));
  const peak = values.reduce(
    (m, vs) => vs.reduce<number>((n, v) => (finite(v) && v > n ? v : n), m),
    0,
  );
  // The floor itself while everything fits under it (60 fps, not 100),
  // a round number above the peak once something doesn't.
  const top =
    spec.top ?? (peak * 1.1 <= spec.floor ? spec.floor : nice(peak * 1.1));
  const x = (t: number) => ((t - begin) / SPAN) * W;
  const y = (v: number) =>
    H - 2 - (Math.min(Math.max(v, 0), top) / top) * (H - 4);
  // Built only when the data changes, not on every move of the cursor.
  const paths = useMemo(
    () =>
      values.map((vs) => {
        let d = "";
        let prev = -Infinity;
        samples.forEach((s, i) => {
          const v = vs[i];
          if (!finite(v)) {
            prev = -Infinity;
            return;
          }
          d += `${s.t - prev > GAP ? "M" : "L"}${x(s.t).toFixed(1)},${y(v).toFixed(1)}`;
          prev = s.t;
        });
        return d;
      }),
    // x and y only change with begin and top.
    [values, samples, begin, top],
  );
  const current = spec.series.map((s) => (at ? s.value(at) : null));
  const first = current[0];
  const over = spec.warn != null && finite(first) && first >= spec.warn;
  const showWarn = spec.warn != null && spec.warn < top;
  function pick(e: PointerEvent<HTMLDivElement>) {
    const r = e.currentTarget.getBoundingClientRect();
    const f = Math.min(1, Math.max(0, (e.clientX - r.left) / r.width));
    onHover(begin + f * SPAN);
  }
  const summary = readings
    ? `${spec.title}: ${spec.series
        .map(
          (s, i) =>
            `${spec.series.length > 1 ? `${s.label} ` : ""}${number(current[i], spec.digits)} ${spec.unit}`,
        )
        .join(
          ", ",
        )}; highest in the last 30 minutes ${number(peak, spec.digits)} ${spec.unit}`
    : `${spec.title}: no readings`;
  return (
    <figure className={`graph ${over ? "over" : ""}`}>
      <figcaption className="graph-head">
        <span className="graph-title">{spec.title}</span>
        <span className="graph-value">
          {spec.series.map((s, i) => (
            <span key={s.label} className={`series series-${i}`}>
              {spec.series.length > 1 && (
                <>
                  <i aria-hidden="true" />
                  {s.label}{" "}
                </>
              )}
              <b>{number(current[i], spec.digits)}</b>
            </span>
          ))}
          <span className="graph-unit">{spec.unit}</span>
        </span>
      </figcaption>
      <div
        className="graph-plot"
        role="img"
        aria-label={summary}
        onPointerDown={pick}
        onPointerMove={(e) => {
          if (e.pointerType === "mouse" || e.buttons) pick(e);
        }}
        onPointerLeave={(e) => {
          if (e.pointerType === "mouse") onHover(null);
        }}
      >
        <svg viewBox={`0 0 ${W} ${H}`} preserveAspectRatio="none">
          {outages.map(([a, b]) => (
            <rect
              key={a}
              x={x(a)}
              y={0}
              width={Math.max(0.5, x(b) - x(a))}
              height={H}
              className="outage"
            />
          ))}
          {showWarn && (
            <line
              x1={0}
              x2={W}
              y1={y(spec.warn!)}
              y2={y(spec.warn!)}
              className="threshold"
            />
          )}
          {paths.map((d, i) => (
            <path key={i} d={d} className={`line line-${i}`} />
          ))}
          {cursor != null && (
            <line
              x1={x(cursor)}
              x2={x(cursor)}
              y1={0}
              y2={H}
              className="cursor"
            />
          )}
        </svg>
        <span className="graph-top">
          {number(top, top < 10 ? 1 : 0)} {spec.unit}
        </span>
        {showWarn && (
          <span
            className="graph-threshold"
            style={{ top: `${(y(spec.warn!) / H) * 100}%` }}
          >
            {spec.warn} {spec.unit}
          </span>
        )}
      </div>
    </figure>
  );
}
