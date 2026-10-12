import { useEffect, useRef, useState } from "react";
import type { Status, Settings, ImageEntry, History } from "./types";
import { api, headers, usePolling } from "./api";
import { modeLabel, number, scan, size, skewLabel } from "./format";
import { Preview } from "./components/Preview";
import { Timeline } from "./components/Timeline";
import { SoftwareUpdate } from "./components/SoftwareUpdate";
import { useReleases } from "./releases";
import { WifiPanel } from "./components/WifiPanel";
import { HardwarePanel } from "./components/HardwarePanel";
import { ConfirmDialog } from "./components/ConfirmDialog";
const phases: Record<string, string> = {
  streaming: "Receiving video",
  waiting_video: "Waiting for camera",
  waiting_usb: "Waiting for goggles",
  stopped: "Stopped",
  starting: "Starting",
  retrying: "Reconnecting",
  accessory: "Goggles connected",
  aoa_negotiation: "Connecting goggles",
  usb_connected: "Goggles connected",
};
const protocols: Record<string, string> = {
  goggles3: "Goggles 3",
  goggles2: "Goggles 2",
};
const protocolStates: Record<string, string> = {
  probing: "trying",
  video: "detected",
  forced: "fixed",
};
function protocolLabel(status: Status | null): string {
  const id = status?.goggles_protocol;
  if (!id) return "";
  const state = status?.goggles_protocol_state;
  return `${protocols[id] ?? id}${state ? ` (${protocolStates[state] ?? state})` : ""}`;
}
// Phases the gadget only reaches once the accessory handshake has succeeded.
const usbPhases = [
  "usb_connected",
  "accessory",
  "aoa_negotiation",
  "waiting_video",
  "streaming",
];
// The goggles can be plugged in and powering the port while the handshake is
// still stuck, so attachment comes from the controller, not from the phase.
function attached(status: Status): boolean {
  return status.usb_state
    ? status.usb_state !== "not attached"
    : usbPhases.includes(status.phase);
}

export default function App() {
  const { value: status, error: connectionError } = usePolling<Status>(
    "status",
    1000,
  );
  const { value: history } = usePolling<History>("history", 5000);
  const [settings, setSettings] = useState<Settings | null>(null);
  const [images, setImages] = useState<ImageEntry[]>([]);
  const [mode, setMode] = useState("auto");
  const [busy, setBusy] = useState(false);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [showRelease, setShowRelease] = useState(false);
  const releases = useReleases(status?.version);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [powerAction, setPowerAction] = useState<"reboot" | "shutdown" | null>(
    null,
  );
  const [powerBusy, setPowerBusy] = useState(false);
  const [timeBusy, setTimeBusy] = useState(false);
  const autoSynced = useRef(false);
  useEffect(() => {
    void Promise.all([
      api<Settings>("settings"),
      api<{ images: ImageEntry[] }>("images"),
    ])
      .then(([s, i]) => {
        setSettings(s);
        setMode(s.hdmi_mode);
        setImages(i.images);
      })
      .catch((e) => setError(String(e)));
  }, []);
  function toast(message: string) {
    setNotice(message);
    window.setTimeout(() => {
      setNotice((current) => (current === message ? "" : current));
    }, 8000);
  }
  async function syncTime(automatic: boolean, quiet = false) {
    if (!status?.time_unix_ms || timeBusy || updateBusy || connectionError)
      return;
    const skew = Date.now() - status.time_unix_ms;
    setTimeBusy(true);
    setError("");
    try {
      const timezone = Intl.DateTimeFormat().resolvedOptions().timeZone;
      try {
        await api("system/time", {
          method: "POST",
          headers,
          body: JSON.stringify({ unix_ms: Date.now(), timezone }),
        });
      } catch (first) {
        await api("system/time", {
          method: "POST",
          headers,
          body: JSON.stringify({ unix_ms: Date.now() }),
        }).catch(() => {
          throw first;
        });
      }
      if (!quiet)
        toast(
          automatic
            ? `Pi clock was ${skewLabel(skew)}; set to this browser.`
            : `Pi clock set to this browser.`,
        );
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setTimeBusy(false);
    }
  }
  useEffect(() => {
    if (
      autoSynced.current ||
      status?.time_unix_ms == null ||
      connectionError ||
      updateBusy
    )
      return;
    if (Math.abs(Date.now() - status.time_unix_ms) <= 1000) {
      autoSynced.current = true;
      // Already correct, but confirm it so the Pi stops its unsynced beep.
      if (status.time_synced === false) void syncTime(true, true);
      return;
    }
    autoSynced.current = true;
    void syncTime(true);
  }, [status?.time_unix_ms, connectionError, updateBusy]);
  async function run(task: () => Promise<void>) {
    setBusy(true);
    setError("");
    try {
      await task();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  async function save(next: Settings) {
    await api("settings", {
      method: "POST",
      headers,
      body: JSON.stringify(next),
    });
    setSettings(next);
    setMode(next.hdmi_mode);
  }
  const live =
    status?.phase === "streaming" &&
    (status?.bitrate_mbps ?? 0) > 0 &&
    !connectionError;
  const selected = images.find((i) => i.id === settings?.fallback_image);
  const selectedIsVideo = selected?.kind === "video";
  const videoCount = images.filter((i) => i.kind === "video").length;
  const modes = Array.from(new Set(status?.hdmi_modes ?? []));
  // Without a reachable receiver the three states are unknown, not "No".
  const known = !!status && !connectionError;
  const gogglesLinked = known && attached(status);
  const handshakeDone = known && usbPhases.includes(status.phase);
  return (
    <main>
      <header>
        <div className="header-top">
          <h1>ungoggled</h1>
          <span
            className={`connection ${connectionError ? "bad" : live ? "good" : ""}`}
          >
            <i />
            {connectionError
              ? "Disconnected"
              : !status
                ? "Connecting"
                : status.phase === "waiting_usb" && gogglesLinked
                  ? "Goggles attached · handshake pending"
                  : (phases[status.phase] ?? status.phase)}
          </span>
          <span className="version">
            {status?.version && `v${status.version}`}
          </span>
        </div>
        <div className="states" aria-label="Receiver state">
          <State
            label="Goggles"
            on={gogglesLinked}
            known={known}
            detail={
              known
                ? gogglesLinked
                  ? `USB ${status.usb_state ?? "attached"} · ${
                      handshakeDone
                        ? (phases[status.phase] ?? status.phase)
                        : "accessory handshake not started"
                    }${protocolLabel(status) ? ` · ${protocolLabel(status)}` : ""}`
                  : "Nothing powering the accessory port"
                : "Receiver unreachable"
            }
          />
          <State
            label="Video"
            on={!!live}
            known={known}
            detail={
              known
                ? live
                  ? `${number(status.input_fps)} fps · ${number(status.bitrate_mbps, 2)} Mbps from the goggles`
                  : "No video arriving from the goggles"
                : "Receiver unreachable"
            }
          />
          <State
            label="HDMI"
            on={known && !!status.hdmi_connected}
            known={known}
            detail={
              known
                ? status.hdmi_connected
                  ? `Display attached · ${size(status.hdmi_width, status.hdmi_height)} at ${number(status.hdmi_hz, 0)} Hz${scan(status.hdmi_interlaced)}`
                  : "No display detected on HDMI0"
                : "Receiver unreachable"
            }
          />
        </div>
      </header>
      {(error || (connectionError && !updateBusy)) && (
        <div className="alert" role="alert">
          {error || "Cannot reach the receiver. Check your network connection."}
        </div>
      )}
      {notice && (
        <div className="notice" role="status">
          {notice}
          <button
            aria-label="Dismiss notification"
            onClick={() => setNotice("")}
          >
            ×
          </button>
        </div>
      )}
      {releases.latest && (
        <button
          className="notice release-notice"
          onClick={() => {
            setShowRelease(true);
            document
              .getElementById("software-update")
              ?.scrollIntoView({ behavior: "smooth" });
          }}
        >
          ungoggled v{releases.latest.version} available · View release notes
        </button>
      )}
      <section className="metrics" aria-label="Video status">
        <Metric
          title="Goggles input"
          primary={size(status?.input_width, status?.input_height)}
          secondary={`${number(status?.input_fps)} fps · ${number(status?.bitrate_mbps, 2)} Mbps`}
        />
        <Metric
          title="HDMI output"
          primary={size(status?.hdmi_width, status?.hdmi_height)}
          secondary={`${number(status?.hdmi_hz, 0)} Hz${scan(status?.hdmi_interlaced)} · ${status?.hdmi === "playing" ? number(status?.output_fps) : "0"} rendered fps`}
        />
        <Metric
          title="Pi temperature"
          primary={`${number(status?.temperature_c)} °C`}
          secondary={
            status
              ? `Uptime ${Math.floor(status.uptime_seconds / 3600)}h ${Math.floor(status.uptime_seconds / 60) % 60}m`
              : "—"
          }
        />
      </section>
      <div className="columns">
        <div className="column">
          <section className="panel">
            <div className="section-title">
              <h2>Preview</h2>
              <label className="toggle">
                <input
                  type="checkbox"
                  checked={settings?.preview_enabled ?? false}
                  disabled={busy || updateBusy || !settings}
                  onChange={(e) => {
                    if (settings)
                      void run(() =>
                        save({
                          ...settings,
                          preview_enabled: e.target.checked,
                        }),
                      );
                  }}
                />
                Live preview
              </label>
            </div>
            <Preview
              enabled={!!settings?.preview_enabled && !!live}
              fallback={selectedIsVideo ? undefined : selected?.url}
              state={
                live
                  ? "live"
                  : status?.hdmi === "fallback"
                    ? "fallback"
                    : "waiting"
              }
            />
            <div className="preview-caption">
              <span>
                {live
                  ? settings?.preview_enabled
                    ? "640 × 360 · up to 5 fps"
                    : "Preview disabled"
                  : status?.hdmi === "fallback"
                    ? status?.fallback_video
                      ? "Fallback video looping on HDMI"
                      : "Fallback on HDMI"
                    : "Waiting for video"}
              </span>
            </div>
            <div className="actions">
              <button
                className="primary"
                disabled={busy || updateBusy || !status || !!connectionError}
                onClick={() =>
                  void run(async () => {
                    await api(status?.enabled ? "stop" : "start", {
                      method: "POST",
                      headers,
                    });
                  })
                }
              >
                {status?.enabled ? "Stop receiver" : "Start receiver"}
              </button>
              <button
                disabled={busy || updateBusy || !status || !!connectionError}
                onClick={() =>
                  void run(async () => {
                    await api("restart", { method: "POST", headers });
                  })
                }
              >
                Reconnect goggles
              </button>
            </div>
          </section>
          <Timeline history={history} />
        </div>
        <section className="panel settings">
          <h2>HDMI</h2>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (settings)
                void run(() => save({ ...settings, hdmi_mode: mode }));
            }}
          >
            <label htmlFor="hdmi-mode">Output mode</label>
            <div className="input-row">
              <select
                id="hdmi-mode"
                value={mode}
                onChange={(e) => setMode(e.target.value)}
              >
                <option value="auto">Automatic · prefer 1080p</option>
                {!modes.includes(mode) && mode !== "auto" && (
                  <option value={mode}>{modeLabel(mode)} (saved)</option>
                )}
                {modes.map((m) => (
                  <option key={m} value={m}>
                    {modeLabel(m)}
                  </option>
                ))}
              </select>
              <button
                disabled={
                  busy || updateBusy || !settings || mode === settings.hdmi_mode
                }
              >
                Apply
              </button>
            </div>
          </form>
          <p className="help">
            Refresh rate is the HDMI signal timing. Rendered fps is the rate of
            camera frames reaching the display.
          </p>
          <hr />
          <h2>Fallback image</h2>
          <div className="fallback-preview">
            {selectedIsVideo ? (
              <span>▶ Video loop selected</span>
            ) : selected ? (
              <img src={selected.url} alt="Selected HDMI fallback" />
            ) : (
              <span>Solid dark background</span>
            )}
          </div>
          <div className="image-library" aria-label="Fallback images">
            <button
              className={!settings?.fallback_image ? "selected" : ""}
              title="Use solid dark background"
              aria-label="Use solid dark background"
              disabled={busy || updateBusy}
              onClick={() => {
                if (settings)
                  void run(() => save({ ...settings, fallback_image: null }));
              }}
            >
              <span>None</span>
            </button>
            {images.map((img, i) => (
              <button
                key={img.id}
                className={
                  settings?.fallback_image === img.id ? "selected" : ""
                }
                aria-label={`Select fallback ${img.kind === "video" ? "video" : "image"} ${i + 1}`}
                title={img.kind === "video" ? "Video loop" : undefined}
                disabled={busy || updateBusy}
                onClick={() => {
                  if (settings)
                    void run(() =>
                      save({ ...settings, fallback_image: img.id }),
                    );
                }}
              >
                {img.kind === "video" ? (
                  <span className="video-tile">▶</span>
                ) : (
                  <img src={img.url} alt={`Fallback ${i + 1}`} />
                )}
              </button>
            ))}
          </div>
          <div className="actions">
            <label className={`button ${busy ? "disabled" : ""}`}>
              Upload image or video
              <input
                className="file-input"
                type="file"
                accept="image/png,image/jpeg,image/webp,video/mp4,video/quicktime,.mp4,.mov"
                disabled={busy || updateBusy || !settings}
                onChange={(e) => {
                  const file = e.target.files?.[0];
                  e.target.value = "";
                  if (!file || !settings) return;
                  void run(async () => {
                    const data = new FormData();
                    data.append("image", file);
                    const uploaded = await api<ImageEntry>("images", {
                      method: "POST",
                      headers: { "X-DJI-Control": "1" },
                      body: data,
                    });
                    setImages(
                      (await api<{ images: ImageEntry[] }>("images")).images,
                    );
                    await save({ ...settings, fallback_image: uploaded.id });
                  });
                }}
              />
            </label>
            {selected && (
              <button
                disabled={busy || updateBusy}
                onClick={() =>
                  void run(async () => {
                    await save({ ...settings!, fallback_image: null });
                    await api(`images/${selected.id}`, {
                      method: "DELETE",
                      headers,
                    });
                    setImages(
                      (await api<{ images: ImageEntry[] }>("images")).images,
                    );
                  })
                }
              >
                Delete selected
              </button>
            )}
          </div>
          <p className="help">
            Shown when video stops or the receiver is stopped. Images: PNG, JPEG
            or WebP, up to 12 MiB, aspect ratio preserved. Videos: MP4 with
            H.264 (not HEVC), up to 1920×1080 and 64 MiB, played in an endless
            loop ({videoCount}/4 stored). Converting a video takes a few
            seconds.
          </p>
        </section>
      </div>
      <WifiPanel notify={setNotice} disabled={updateBusy} />
      <HardwarePanel
        settings={settings}
        onSave={save}
        disabled={busy || updateBusy}
      />
      <section className="panel system">
        <h2>System</h2>
        <p className="clock">
          {status?.time
            ? `${status.time}${status.timezone ? ` · ${status.timezone}` : ""}`
            : "Pi time unavailable"}
        </p>
        <p className="help">
          The Pi has no battery-backed clock. Opening this page sets it from
          your browser when they differ by more than a second.
        </p>
        <div className="actions">
          <button
            disabled={
              timeBusy ||
              updateBusy ||
              !status?.time_unix_ms ||
              !!connectionError
            }
            onClick={() => void syncTime(false)}
          >
            {timeBusy ? "Setting time…" : "Sync time from this browser"}
          </button>
          <button
            disabled={updateBusy || !status || !!connectionError}
            onClick={() => setPowerAction("reboot")}
          >
            Reboot Pi
          </button>
          <button
            disabled={updateBusy || !status || !!connectionError}
            onClick={() => setPowerAction("shutdown")}
          >
            Shut down Pi
          </button>
        </div>
      </section>
      <ConfirmDialog
        open={powerAction !== null}
        busy={powerBusy}
        title={
          powerAction === "reboot" ? "Reboot the Pi?" : "Shut down the Pi?"
        }
        body={
          powerAction === "reboot"
            ? "Video and the web interface stop until the Pi finishes restarting."
            : "Video and the web interface stop. Power-cycle the Pi to turn it back on."
        }
        confirmLabel={powerAction === "reboot" ? "Reboot" : "Shut down"}
        onCancel={() => setPowerAction(null)}
        onConfirm={() =>
          void run(async () => {
            setPowerBusy(true);
            try {
              await api(
                `system/${powerAction === "reboot" ? "reboot" : "shutdown"}`,
                {
                  method: "POST",
                  headers,
                },
              );
              setPowerAction(null);
            } finally {
              setPowerBusy(false);
            }
          })
        }
      />
      <SoftwareUpdate
        releases={releases}
        showRelease={showRelease}
        setShowRelease={setShowRelease}
        onBusy={setUpdateBusy}
      />
      <details className="panel diagnostics">
        <summary>Diagnostics</summary>
        <dl>
          <dt>Receiver</dt>
          <dd>{status?.phase ?? "—"}</dd>
          <dt>Goggles protocol</dt>
          <dd>{protocolLabel(status) || "—"}</dd>
          <dt>HDMI</dt>
          <dd>{status?.hdmi ?? "—"}</dd>
          <dt>Received</dt>
          <dd>{number((status?.video_bytes ?? 0) / 1e6)} MB</dd>
          <dt>USB resync bytes</dt>
          <dd>{status?.discarded_bytes ?? 0}</dd>
          <dt>Decoder starts</dt>
          <dd>{status?.decoder_starts ?? 0}</dd>
          <dt>Advertised camera fps</dt>
          <dd>{number(status?.input_nominal_fps)}</dd>
          <dt>Last event</dt>
          <dd>{status?.message ?? "—"}</dd>
          <dt>Beeper</dt>
          <dd>{status?.beeper_error ?? "—"}</dd>
          <dt>Power button</dt>
          <dd>{status?.power_button_error ?? "—"}</dd>
        </dl>
        <a href="/api/diagnostics" target="_blank" rel="noreferrer">
          Hardware report
        </a>
      </details>
      <footer>
        Clean feed: Goggles → Camera → Advanced Camera Settings → Camera View
        Recording off.
      </footer>
    </main>
  );
}
function State({
  label,
  on,
  known,
  detail,
}: {
  label: string;
  on: boolean;
  known: boolean;
  detail: string;
}) {
  return (
    <span
      className={`state ${!known ? "unknown" : on ? "yes" : "no"}`}
      title={detail}
    >
      <i />
      {label}
      <b>{!known ? "—" : on ? "Yes" : "No"}</b>
    </span>
  );
}
function Metric({
  title,
  primary,
  secondary,
}: {
  title: string;
  primary: string;
  secondary: string;
}) {
  return (
    <div className="panel metric">
      <h2>{title}</h2>
      <strong>{primary}</strong>
      <span>{secondary}</span>
    </div>
  );
}
