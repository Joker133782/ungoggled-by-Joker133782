import { lazy, Suspense, useState } from "react";
import type { Settings } from "../types";
import { PanelHead } from "./PanelHead";
import { IconQr, IconStream } from "./icons";

// The QR code generator only loads once someone opens the share sheet.
const ShareSheet = lazy(() => import("./ShareSheet"));

/**
 * Relay the live feed to spectators' phones over the Pi's own Wi-Fi, as a
 * separate stream from the HDMI output. Bandwidth, not this setting, is the
 * real limit: the Pi's own Wi-Fi radio carries only so much, and that cost
 * is paid per viewer, not per setting. "High" sends the original goggles
 * stream unmodified, only realistic for one or two viewers at once; "Event"
 * trades quality for more simultaneous viewers.
 *
 * Takes `settings`/`onSave` from the parent rather than keeping its own copy
 * — three independent copies of the same server-side Settings object (this,
 * RecordingPanel, App) used to race: saving one here could silently revert
 * a change made moments earlier in another panel, since each POSTs the
 * whole object from its own possibly-stale state.
 */
export function StreamPanel({
  disabled,
  settings,
  onSave,
}: {
  disabled: boolean;
  settings: Settings | null;
  onSave: (next: Settings) => Promise<void>;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const [sharing, setSharing] = useState(false);
  const watchUrl = `${window.location.origin}/watch.html`;

  async function copyLink() {
    try {
      await navigator.clipboard.writeText(watchUrl);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 2000);
    } catch {
      /* Clipboard refused; the link text itself stays selectable. */
    }
  }

  async function save(next: Settings) {
    setBusy(true);
    setError("");
    try {
      await onSave(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setBusy(false);
    }
  }

  if (!settings) return null;
  const highOn = settings.stream_enabled && settings.stream_quality === "high";
  const lowOn = settings.stream_enabled && settings.stream_quality === "event";
  function setQuality(quality: "high" | "event", on: boolean) {
    void save(
      on
        ? { ...settings!, stream_enabled: true, stream_quality: quality }
        : { ...settings!, stream_enabled: false },
    );
  }
  return (
    <section className="panel stream">
      <PanelHead
        icon={<IconStream />}
        title="Spectator stream"
        experimental
        wiki="stream"
      />
      <ul className="toggle-list">
        <li>
          <label className="toggle">
            <input
              type="checkbox"
              checked={highOn}
              disabled={disabled || busy}
              onChange={(e) => setQuality("high", e.currentTarget.checked)}
            />
            High quality
          </label>
        </li>
        <li>
          <label className="toggle">
            <input
              type="checkbox"
              checked={lowOn}
              disabled={disabled || busy}
              onChange={(e) => setQuality("event", e.currentTarget.checked)}
            />
            Low quality
          </label>
        </li>
      </ul>
      <p className="status-line">
        Spectators join the Pi&apos;s Wi-Fi and open{" "}
        <a href="/watch.html" target="_blank" rel="noreferrer">
          {watchUrl.replace(/^https?:\/\//, "")}
        </a>
        {navigator.clipboard && (
          <button
            type="button"
            className="link-button"
            onClick={() => void copyLink()}
          >
            {copied ? "Copied" : "Copy link"}
          </button>
        )}
      </p>
      <div className="actions">
        <button onClick={() => setSharing(true)}>
          <IconQr /> Show QR codes for spectators
        </button>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {sharing && (
        <Suspense fallback={null}>
          <ShareSheet
            streamOn={settings.stream_enabled}
            onClose={() => setSharing(false)}
          />
        </Suspense>
      )}
    </section>
  );
}
