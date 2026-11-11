import { useCallback, useEffect, useState } from "react";
import { api, headers, usePolling, withTimeout } from "../api";
import type {
  Recording,
  RecordingStatus,
  Settings,
  UsbVideoStatus,
} from "../types";
import { PanelHead } from "./PanelHead";
import { ConfirmDialog } from "./ConfirmDialog";
import {
  IconRecord,
  IconClose,
  IconPlay,
  IconStop,
  IconClip,
  IconDownload,
  IconTrash,
} from "./icons";

const bytes = (n: number) => {
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  const mb = n / (1024 * 1024);
  return mb < 1024 ? `${mb.toFixed(0)} MB` : `${(mb / 1024).toFixed(1)} GB`;
};
const time = (s: number) => {
  const m = Math.floor(s / 60);
  const sec = Math.floor(s % 60);
  return `${m}:${sec.toString().padStart(2, "0")}`;
};
const when = (s: number | null) =>
  s ? new Date(s * 1000).toLocaleString() : "unknown date";

/*
 * Record the live goggles feed to the SD card (raw, no re-encoding). Four
 * independent ways to capture: auto-record every flight start to finish, a
 * crash buffer that auto-saves the last 30 seconds when the feed drops
 * (like the goggles' own instant-replay), manual start/stop, and a "save
 * the last 30 seconds" shortcut for a moment you didn't plan to record.
 *
 * Split across three tabs, one component each: the buttons pressed
 * mid-flight (RecordingControls, Live), the set-once switches
 * (RecordingSettings, Settings) and the saved files (RecordingList, Videos).
 * Each fetches only what it shows, so a tab that isn't on screen doesn't
 * poll for it. Switches save through App's single Settings copy
 * (`settings`/`onSave`), never a copy of their own — see StreamPanel.
 */

/** Manual start/stop and "save the last 30 seconds", on the Live tab. */
export function RecordingControls({
  diskWarning,
  disabled,
  settings,
}: {
  diskWarning?: boolean;
  disabled: boolean;
  settings: Settings | null;
}) {
  // A failed poll keeps the last status (the receiver may be restarting);
  // the next one retries.
  const { value: status } = usePolling<RecordingStatus>("recording", 2000);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  // The saved file lands on the Videos tab, out of sight from here, so say
  // so instead of leaving the button press without any visible result.
  const [saved, setSaved] = useState("");
  useEffect(() => {
    if (!saved) return;
    const timer = setTimeout(() => setSaved(""), 10000);
    return () => clearTimeout(timer);
  }, [saved]);

  async function start() {
    setBusy("start");
    setError("");
    setSaved("");
    try {
      await api("recording", { method: "POST", headers });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start recording.");
    } finally {
      setBusy(null);
    }
  }
  async function stop() {
    setBusy("stop");
    try {
      await api("recording/stop", { method: "POST", headers });
      setSaved("Recording saved.");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not stop recording.");
    } finally {
      setBusy(null);
    }
  }
  async function saveBuffer() {
    setBusy("buffer");
    setError("");
    setSaved("");
    try {
      await api("recording/save-buffer", { method: "POST", headers });
      setSaved("Saved the last 30 seconds.");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Nothing recent to save yet.");
    } finally {
      setBusy(null);
    }
  }
  async function dismissWarning() {
    try {
      await api("recording/dismiss-warning", { method: "POST", headers });
    } catch {
      /* Not critical; the monitor notice clears on the next recording anyway. */
    }
  }

  const recording = status?.recording ?? false;
  return (
    <section className="panel recording">
      <PanelHead icon={<IconRecord />} title="Recording" wiki="recording" />
      {diskWarning && (
        <p role="alert" className="error">
          Storage is full; recording stopped. Delete some recordings under{" "}
          <a href="#videos">Videos</a>, then{" "}
          <button disabled={disabled} onClick={() => void dismissWarning()}>
            <IconClose /> dismiss
          </button>
        </p>
      )}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        {recording ? (
          <button
            disabled={disabled || busy === "stop"}
            onClick={() => void stop()}
          >
            <IconStop /> Stop recording ({time(status?.seconds ?? 0)},{" "}
            {bytes(status?.bytes ?? 0)})
          </button>
        ) : (
          <button
            disabled={disabled || busy === "start"}
            onClick={() => void start()}
          >
            <IconPlay /> Start recording
          </button>
        )}
        <button
          disabled={disabled || busy === "buffer"}
          onClick={() => void saveBuffer()}
        >
          <IconClip /> Save last 30 seconds now
        </button>
      </div>
      {saved && (
        <p role="status" className="status-line">
          {saved} <a href="#videos">Show in Videos</a>
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {settings && (
        <p className="status-line">
          Auto-record {settings.auto_record ? "on" : "off"} · crash buffer{" "}
          {settings.crash_buffer ? "on" : "off"} ·{" "}
          <a href="#settings/recording-settings">change</a>
        </p>
      )}
    </section>
  );
}

/** The set-once recording switches, on the Settings tab. */
export function RecordingSettings({
  disabled,
  settings,
  onSave,
}: {
  disabled: boolean;
  settings: Settings | null;
  onSave: (patch: Partial<Settings>) => Promise<void>;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const { value: usbStatus } = usePolling<UsbVideoStatus>(
    "usb-videos/status",
    3000,
  );
  const drives = usbStatus?.drives ?? [];

  async function setAutoRecord(auto_record: boolean) {
    if (!settings) return;
    setBusy("auto");
    setError("");
    try {
      await onSave({ auto_record });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setBusy(null);
    }
  }
  async function setCrashBuffer(crash_buffer: boolean) {
    if (!settings) return;
    setBusy("crash");
    setError("");
    try {
      await onSave({ crash_buffer });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setBusy(null);
    }
  }
  async function setRecordToUsb(record_to_usb: boolean) {
    if (!settings) return;
    setBusy("record_to_usb");
    setError("");
    try {
      await onSave({ record_to_usb });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setBusy(null);
    }
  }

  return (
    <section className="panel recording-settings" id="recording-settings">
      <PanelHead icon={<IconRecord />} title="Recording" wiki="recording" />
      {settings && (
        <ul className="toggle-list">
          <li>
            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.auto_record}
                disabled={disabled || busy === "auto"}
                onChange={(e) => void setAutoRecord(e.currentTarget.checked)}
              />
              Auto-record every flight
            </label>
          </li>
          <li>
            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.crash_buffer}
                disabled={disabled || busy === "crash"}
                onChange={(e) => void setCrashBuffer(e.currentTarget.checked)}
              />
              Crash buffer (last 30 seconds)
            </label>
          </li>
          <li>
            <label className="toggle">
              <input
                type="checkbox"
                checked={settings.record_to_usb}
                disabled={disabled || busy === "record_to_usb"}
                onChange={(e) => void setRecordToUsb(e.currentTarget.checked)}
              />
              Record to USB drive instead of SD card
            </label>
            <span className="help">
              {drives.length > 0
                ? `Detected: ${drives.join(", ")}`
                : "No USB drive detected — recordings stay on the SD card until one is plugged in."}
            </span>
          </li>
        </ul>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </section>
  );
}

/**
 * Saved recordings, clips and flights, on the Videos tab: play one on the
 * HDMI output, download it or delete it. Pause, seek and stop for a playing
 * recording appear in the USB player panel, which drives the same player.
 */
export function RecordingList({ disabled }: { disabled: boolean }) {
  const [recordings, setRecordings] = useState<Recording[] | null>(null);
  const [loadError, setLoadError] = useState("");
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState<Recording | null>(null);

  const loadList = useCallback(async (parent: AbortSignal | null = null) => {
    const request = withTimeout(parent, 8000);
    try {
      const r = await api<{ recordings: Recording[] }>("recordings", {
        signal: request.signal,
      });
      if (parent?.aborted) return;
      setRecordings(r.recordings);
      setLoadError("");
    } catch (e) {
      // Shown only while there's no list yet; a list already on screen
      // stays, and the next reload retries.
      if (!parent?.aborted)
        setLoadError(
          e instanceof Error ? e.message : "Could not load recordings.",
        );
    } finally {
      request.done();
    }
  }, []);
  // Recordings appear (auto-record, crash buffer) and disappear (rotation)
  // on their own, so the list reloads while it's on screen.
  useEffect(() => {
    const abort = new AbortController();
    void loadList(abort.signal);
    const timer = setInterval(() => {
      if (!document.hidden) void loadList(abort.signal);
    }, 10000);
    const onVisible = () => {
      if (!document.hidden) void loadList(abort.signal);
    };
    document.addEventListener("visibilitychange", onVisible);
    return () => {
      abort.abort();
      clearInterval(timer);
      document.removeEventListener("visibilitychange", onVisible);
    };
  }, [loadList]);

  async function play(name: string) {
    setBusy(name);
    try {
      await api(`recordings/${name}/play`, { method: "POST", headers });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start playback.");
    } finally {
      setBusy(null);
    }
  }
  async function remove(name: string) {
    setBusy(name);
    try {
      await api(`recordings/${name}`, { method: "DELETE", headers });
      await loadList();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not delete recording.");
    } finally {
      setBusy(null);
      setDeleting(null);
    }
  }

  return (
    <section className="panel recording-files">
      <PanelHead icon={<IconRecord />} title="Recordings" wiki="recording" />
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {!recordings &&
        (loadError ? (
          <p role="alert" className="error">
            Could not load the recordings: {loadError}{" "}
            <button onClick={() => void loadList()}>Retry</button>
          </p>
        ) : (
          <p className="status-line">Loading…</p>
        ))}
      {recordings && recordings.length === 0 && (
        <p className="help">No recordings yet.</p>
      )}
      {recordings && recordings.length > 0 && (
        <ul className="recording-list">
          {recordings.map((r) => (
            <li key={r.name}>
              {r.thumbnail ? (
                <img
                  className="usb-thumbnail"
                  src={`/api/recordings/${r.name}/thumbnail`}
                  alt=""
                />
              ) : (
                <div
                  className="usb-thumbnail usb-thumbnail-empty"
                  aria-hidden="true"
                />
              )}
              <div style={{ flex: 1, minWidth: 0 }}>
                {r.name}
                <div className="help">
                  {when(r.modified)} · {bytes(r.size)}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <button
                  disabled={disabled || busy === r.name}
                  onClick={() => void play(r.name)}
                >
                  <IconPlay /> Play on HDMI
                </button>
                <a href={`/api/recordings/${r.name}`}>
                  <button disabled={disabled}>
                    <IconDownload /> Download
                  </button>
                </a>
                <button
                  disabled={disabled || busy === r.name}
                  onClick={() => setDeleting(r)}
                >
                  <IconTrash /> Delete
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
      <ConfirmDialog
        open={deleting !== null}
        busy={deleting !== null && busy === deleting.name}
        title={`Delete “${deleting?.name ?? ""}”?`}
        body="The file is removed from the Pi (or from the USB drive it was recorded to) and cannot be restored."
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => deleting && void remove(deleting.name)}
      />
    </section>
  );
}
