import {
  useCallback,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";
import type { Status, Settings, ImageLibrary, HdmiConnector } from "./types";
import { api, headers, usePolling } from "./api";
import { is4k60, modeLabel, number, scan, size, skewLabel } from "./format";
import { useRoute } from "./route";
import { Preview } from "./components/Preview";
import { Versions } from "./components/Versions";
import { Graphs } from "./components/Graphs";
import { GraphSettings } from "./components/GraphSettings";
import { IdleScreen } from "./components/IdleScreen";
import { TerminalPanel, TerminalSettings } from "./components/TerminalPanel";
import { SoftwareUpdate } from "./components/SoftwareUpdate";
import { useNextcloudRelease } from "./nextcloud";
import { WifiPanel } from "./components/WifiPanel";
import { HardwarePanel } from "./components/HardwarePanel";
import { UsbVideos } from "./components/UsbVideos";
import {
  RecordingControls,
  RecordingList,
  RecordingSettings,
} from "./components/Recording";
import { StreamPanel } from "./components/StreamPanel";
import { ConfirmDialog } from "./components/ConfirmDialog";
import { PanelHead } from "./components/PanelHead";
import { Background } from "./components/Background";
import { TabBar } from "./components/TabBar";
import {
  IconPreview,
  IconHdmi,
  IconClock,
  IconGoggles,
  IconThermometer,
  IconReboot,
  IconPower,
  IconSyncTime,
  IconPlay,
  IconStop,
  IconReconnect,
  IconUpdate,
  IconCheck,
  IconDownload,
} from "./components/icons";
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
const updateTarget = "#system/software-update";

export default function App() {
  // Four tabs (Live · Videos · Settings · System), chosen by the URL hash. A
  // tab's panels only exist while it's shown, so their polling stops when
  // you switch away — with one exception, SoftwareUpdate, see below.
  const { tab, sub } = useRoute();
  const { value: status, error: connectionError } = usePolling<Status>(
    "status",
    1000,
  );
  const { value: hdmiConnectors } = usePolling<HdmiConnector[]>(
    "hdmi-connectors",
    5000,
    tab === "settings",
  );
  const [settings, setSettings] = useState<Settings | null>(null);
  const [library, setLibrary] = useState<ImageLibrary | null>(null);
  const reloadLibrary = useCallback(async () => {
    setLibrary(await api<ImageLibrary>("images"));
  }, []);
  const [mode, setMode] = useState("auto");
  const [connectorChoice, setConnectorChoice] = useState("auto");
  const [busy, setBusy] = useState(false);
  const [updateBusy, setUpdateBusy] = useState(false);
  const [showRelease, setShowRelease] = useState(false);
  const nextcloud = useNextcloudRelease(status?.version);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [powerAction, setPowerAction] = useState<"reboot" | "shutdown" | null>(
    null,
  );
  const [powerBusy, setPowerBusy] = useState(false);
  const [timeBusy, setTimeBusy] = useState(false);
  const autoSynced = useRef(false);
  const [scrolled, setScrolled] = useState(false);
  useEffect(() => {
    const onScroll = () => setScrolled(window.scrollY > 8);
    onScroll();
    window.addEventListener("scroll", onScroll, { passive: true });
    return () => window.removeEventListener("scroll", onScroll);
  }, []);
  // A newly opened tab starts at its top, or at the section named after the
  // slash ("#system/software-update"), rather than at the previous tab's
  // scroll position.
  useEffect(() => {
    if (sub)
      document.getElementById(sub)?.scrollIntoView({ behavior: "smooth" });
    else window.scrollTo(0, 0);
  }, [tab, sub]);
  useEffect(() => {
    void Promise.all([api<Settings>("settings"), api<ImageLibrary>("images")])
      .then(([s, i]) => {
        setSettings(s);
        setMode(s.hdmi_mode);
        setConnectorChoice(
          s.hdmi_connector == null ? "auto" : String(s.hdmi_connector),
        );
        setLibrary(i);
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
  const selected = library?.images.find(
    (i) => i.id === settings?.fallback_image,
  );
  const modes = Array.from(new Set(status?.hdmi_modes ?? []));
  // Without a reachable receiver the three states are unknown, not "No".
  const known = !!status && !connectionError;
  const gogglesLinked = known && attached(status);
  const handshakeDone = known && usbPhases.includes(status.phase);
  const badges = nextcloud.latest
    ? { system: `v${nextcloud.latest.version} available` }
    : {};
  return (
    <main>
      <Background />
      <header className={scrolled ? "scrolled" : ""}>
        <div className="header-top">
          <div className="brand">
            <span className="logo">
              <IconGoggles />
            </span>
            <h1>
              <span className="accent-text">un</span>goggled
            </h1>
          </div>
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
          <button
            className="logout"
            title="Log out"
            onClick={() =>
              void api("auth/logout", { method: "POST", headers }).then(() =>
                window.location.reload(),
              )
            }
          >
            Log out
          </button>
        </div>
        <TabBar current={tab} placement="top" badges={badges} />
      </header>
      {(error || (connectionError && !updateBusy)) && (
        <div className="alert" role="alert">
          {error || "Cannot reach the receiver. Check your network connection."}
        </div>
      )}
      {status?.reboot_required && !connectionError && (
        <div className="notice" role="status">
          <span>
            Restart the Pi once to finish the last update — whenever it suits
            you, not mid-flight. Everything keeps working until then.{" "}
            <a
              href="/wiki.html#reboot-pending"
              target="_blank"
              rel="noreferrer"
            >
              Why?
            </a>
          </span>
          <button
            className="reboot-now"
            disabled={updateBusy}
            onClick={() => setPowerAction("reboot")}
          >
            Reboot now
          </button>
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
      {nextcloud.latest && (
        <button
          className="notice release-notice"
          onClick={() => {
            setShowRelease(true);
            if (window.location.hash === updateTarget)
              document
                .getElementById("software-update")
                ?.scrollIntoView({ behavior: "smooth" });
            else window.location.hash = updateTarget;
          }}
        >
          <IconUpdate /> ungoggled v{nextcloud.latest.version} available · View
          release notes
        </button>
      )}

      {tab === "live" && (
        <div className="page">
          <section className="metrics two" aria-label="Video status">
            <Metric
              icon={<IconGoggles />}
              status={!known ? "unknown" : gogglesLinked ? "on" : "off"}
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
              title="Goggles input"
              primary={size(status?.input_width, status?.input_height)}
              secondary={`${number(status?.input_fps)} fps · ${number(status?.bitrate_mbps, 2)} Mbps`}
              sub={{
                status: !known ? "unknown" : live ? "on" : "off",
                label: live ? "Streaming" : "No video",
                detail: known
                  ? live
                    ? `${number(status.input_fps)} fps · ${number(status.bitrate_mbps, 2)} Mbps from the goggles`
                    : "No video arriving from the goggles"
                  : "Receiver unreachable",
              }}
            />
            <Metric
              icon={<IconHdmi />}
              status={!known ? "unknown" : status.hdmi_connected ? "on" : "off"}
              detail={
                known
                  ? status.hdmi_connected
                    ? `Display attached · ${size(status.hdmi_width, status.hdmi_height)} at ${number(status.hdmi_hz, 0)} Hz${scan(status.hdmi_interlaced)}`
                    : (status.display_error ?? "No display detected")
                  : "Receiver unreachable"
              }
              title="HDMI output"
              primary={size(status?.hdmi_width, status?.hdmi_height)}
              secondary={`${number(status?.hdmi_hz, 0)} Hz${scan(status?.hdmi_interlaced)} · ${status?.hdmi === "playing" || status?.usb_fps != null ? number(status?.output_fps) : "0"} rendered fps${status?.usb_speed != null ? ` · USB video at ${number(status.usb_speed, 2)}× speed` : ""}`}
            />
          </section>
          <section className="panel">
            <PanelHead icon={<IconPreview />} title="Preview" wiki="preview">
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
            </PanelHead>
            <Preview
              enabled={!!settings?.preview_enabled && !!live}
              // A video idle screen shows its poster frame here.
              fallback={
                selected?.kind === "video" ? selected.thumbnail : selected?.url
              }
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
                      ? "Idle screen video looping on HDMI"
                      : "Idle screen on HDMI"
                    : "Waiting for video"}
              </span>
            </div>
            <div className="actions">
              <button
                disabled={busy || updateBusy || !status || !!connectionError}
                onClick={() =>
                  void run(async () => {
                    await api("restart", { method: "POST", headers });
                  })
                }
              >
                <IconReconnect /> Reconnect goggles
              </button>
            </div>
          </section>
          <RecordingControls
            diskWarning={status?.disk_warning}
            disabled={updateBusy}
            settings={settings}
          />
          <StreamPanel
            disabled={updateBusy}
            settings={settings}
            onSave={save}
          />
          <footer>
            Clean feed: Goggles → Camera → Advanced Camera Settings → Camera
            View Recording off.
          </footer>
        </div>
      )}

      {tab === "videos" && (
        <div className="page">
          {/* The USB player first: its pause/seek/stop controls also drive a
              recording started with "Play on HDMI" further down, so they
              should sit at the top of the tab, not under a long list. */}
          <UsbVideos
            disabled={updateBusy}
            speed={status?.usb_speed ?? null}
            fps={status?.usb_fps ?? null}
            clipFps={status?.usb_clip_fps ?? null}
          />
          <RecordingList disabled={updateBusy} />
        </div>
      )}

      {tab === "settings" && (
        <div className="page">
          <div className="row">
            <section className="panel settings">
              <PanelHead icon={<IconHdmi />} title="HDMI" wiki="hdmi" />
              <form
                onSubmit={(e) => {
                  e.preventDefault();
                  if (settings)
                    void run(() =>
                      save({
                        ...settings,
                        hdmi_mode: mode,
                        hdmi_connector:
                          connectorChoice === "auto"
                            ? null
                            : Number(connectorChoice),
                      }),
                    );
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
                </div>
                <p className="help">
                  Automatic uses 1080p (up to 60 Hz), which is right for almost
                  everyone.
                  {modes.some(is4k60) &&
                    " 4K at 60 Hz works only on the HDMI port next to the USB-C power input and needs a high-speed (HDMI 2.0) cable. If the picture turns black after choosing a 4K mode, choose Automatic again, and restart the Pi if the picture does not come back."}
                </p>
                {hdmiConnectors && hdmiConnectors.length > 1 && (
                  <>
                    <label htmlFor="hdmi-connector">Output port</label>
                    <div className="input-row">
                      <select
                        id="hdmi-connector"
                        value={connectorChoice}
                        onChange={(e) => setConnectorChoice(e.target.value)}
                      >
                        <option value="auto">
                          Automatic · whichever port has a display
                        </option>
                        {hdmiConnectors.map((c) => (
                          <option key={c.id} value={c.id}>
                            {c.name} {c.connected ? "· display attached" : ""}
                          </option>
                        ))}
                      </select>
                    </div>
                  </>
                )}
                <div className="actions">
                  <button
                    disabled={
                      busy ||
                      updateBusy ||
                      !settings ||
                      (mode === settings.hdmi_mode &&
                        connectorChoice ===
                          (settings.hdmi_connector == null
                            ? "auto"
                            : String(settings.hdmi_connector)))
                    }
                  >
                    <IconCheck /> Apply
                  </button>
                </div>
              </form>
              <ul className="toggle-list">
                <li>
                  <label className="toggle">
                    <input
                      type="checkbox"
                      checked={settings?.stretch_live ?? false}
                      disabled={busy || updateBusy || !settings}
                      onChange={(e) => {
                        const stretch_live = e.currentTarget.checked;
                        if (settings)
                          void run(() => save({ ...settings, stretch_live }));
                      }}
                    />
                    Stretch the live picture to fill the screen
                  </label>
                  <span className="help">
                    Off shows the picture as it arrives: a 4:3 picture gets
                    black bars at the sides. On stretches it to 16:9. Restarts
                    the live picture once. The Pi&apos;s display chip refuses
                    some picture sizes; those stay unstretched.
                  </span>
                </li>
                <li>
                  <label className="toggle">
                    <input
                      type="checkbox"
                      checked={settings?.stretch_usb ?? false}
                      disabled={busy || updateBusy || !settings}
                      onChange={(e) => {
                        const stretch_usb = e.currentTarget.checked;
                        if (settings)
                          void run(() => save({ ...settings, stretch_usb }));
                      }}
                    />
                    Stretch USB videos to fill the screen
                  </label>
                  <span className="help">
                    The same for videos played from a USB drive, also while one
                    is playing.
                  </span>
                </li>
              </ul>
              <p className="help">
                Refresh rate is the HDMI signal timing. Rendered fps is the rate
                of camera frames reaching the display.
              </p>
              {known && !status?.hdmi_connected && status?.display_error && (
                <p role="alert" className="error">
                  {status.display_error}
                </p>
              )}
            </section>
            <RecordingSettings
              disabled={updateBusy}
              settings={settings}
              onSave={save}
            />
          </div>
          <IdleScreen
            library={library}
            settings={settings}
            status={known ? status : null}
            disabled={updateBusy}
            onSave={save}
            onLibraryChange={reloadLibrary}
          />
          <WifiPanel
            notify={setNotice}
            disabled={updateBusy}
            settings={settings}
            onSave={save}
          />
          <HardwarePanel
            settings={settings}
            onSave={save}
            disabled={busy || updateBusy}
          />
          <div className="row">
            <GraphSettings />
            <TerminalSettings
              disabled={updateBusy}
              settings={settings}
              onSave={save}
            />
          </div>
        </div>
      )}

      {/* System stays in the page while hidden, unlike the other tabs: its
          SoftwareUpdate panel reports "update in progress" to every tab
          (onBusy), reloads the page once a new build is running, and aborts
          a running update upload when unmounted — none of which may stop
          just because someone switched tabs. Everything else on this tab
          only renders, and polls, while it's open. */}
      <div className="page" hidden={tab !== "system"}>
        {tab === "system" && (
          <section className="metrics single" aria-label="Pi status">
            <Metric
              icon={<IconThermometer />}
              status="neutral"
              title="Pi temperature"
              primary={`${number(status?.temperature_c)} °C`}
              secondary={
                status
                  ? `Uptime ${Math.floor(status.uptime_seconds / 3600)}h ${Math.floor(status.uptime_seconds / 60) % 60}m`
                  : "—"
              }
            />
          </section>
        )}
        <div className="row">
          <SoftwareUpdate
            nextcloud={nextcloud}
            showRelease={showRelease}
            setShowRelease={setShowRelease}
            onBusy={setUpdateBusy}
            visible={tab === "system"}
          />
          {tab === "system" && <Versions disabled={updateBusy} />}
        </div>
        {tab === "system" && (
          <>
            <section className="panel system">
              <PanelHead icon={<IconClock />} title="System" wiki="system" />
              <p className="clock">
                {status?.time
                  ? `${status.time}${status.timezone ? ` · ${status.timezone}` : ""}`
                  : "Pi time unavailable"}
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
                  <IconSyncTime />
                  {timeBusy ? "Setting time…" : "Sync Time"}
                </button>
                <button
                  disabled={updateBusy || !status || !!connectionError}
                  onClick={() => setPowerAction("reboot")}
                >
                  <IconReboot />
                  Reboot Pi
                </button>
                <button
                  disabled={updateBusy || !status || !!connectionError}
                  onClick={() => setPowerAction("shutdown")}
                >
                  <IconPower />
                  Shut down Pi
                </button>
              </div>
              <div className="actions">
                <button
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
                  {status?.enabled ? (
                    <>
                      <IconStop /> Stop receiver
                    </>
                  ) : (
                    <>
                      <IconPlay /> Start receiver
                    </>
                  )}
                </button>
                <a className="button" href="/api/support-bundle">
                  <IconDownload /> Download diagnostics
                </a>
              </div>
            </section>
            <Graphs />
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
            <TerminalPanel disabled={updateBusy} />
          </>
        )}
      </div>
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
      <TabBar current={tab} placement="bottom" badges={badges} />
    </main>
  );
}
type MetricStatus = "on" | "off" | "neutral" | "unknown";
function Metric({
  icon,
  status,
  detail,
  title,
  primary,
  secondary,
  sub,
}: {
  icon: ReactNode;
  status: MetricStatus;
  detail?: string;
  title: string;
  primary: string;
  secondary: string;
  sub?: { status: MetricStatus; label: string; detail: string };
}) {
  return (
    <div className="panel metric">
      <div className="metric-head">
        <span className={`metric-icon ${status}`} title={detail}>
          {icon}
        </span>
        <h2>{title}</h2>
      </div>
      <strong>{primary}</strong>
      <span>{secondary}</span>
      {sub && (
        <div className="metric-sub" title={sub.detail}>
          <i className={sub.status} />
          {sub.label}
        </div>
      )}
    </div>
  );
}
