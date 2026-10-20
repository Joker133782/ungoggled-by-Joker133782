import { useCallback, useEffect, useState } from "react";
import { api, headers } from "../api";
import type { Settings } from "../types";

/**
 * Relay the live feed to spectators' phones over the Pi's own Wi-Fi, as a
 * separate stream from the HDMI output. Bandwidth, not this setting, is the
 * real limit: the Pi's own Wi-Fi radio carries only so much, and that cost
 * is paid per viewer, not per setting. "High" sends the original goggles
 * stream unmodified, only realistic for one or two viewers at once; "Event"
 * trades quality for more simultaneous viewers.
 */
export function StreamPanel({ disabled }: { disabled: boolean }) {
  const [settings, setSettings] = useState<Settings | null>(null);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");

  const load = useCallback(async () => {
    try {
      setSettings(await api<Settings>("settings"));
    } catch {
      /* Receiver may be restarting; the next poll retries. */
    }
  }, []);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), 5000);
    return () => clearInterval(timer);
  }, [load]);

  async function save(next: Settings) {
    setBusy(true);
    setError("");
    try {
      await api("settings", { method: "POST", headers, body: JSON.stringify(next) });
      setSettings(next);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not save.");
    } finally {
      setBusy(false);
    }
  }

  if (!settings) return null;
  return (
    <section className="panel stream">
      <h2>Spectator stream (experimental)</h2>
      <p className="help">
        Lets phones and tablets connected to the Pi's Wi-Fi watch the live feed in a browser,
        separately from the HDMI output, at <a href="/watch.html">/watch.html</a>. This is not the
        goggles' own low-latency link the pilot flies with — only something extra for bystanders.
        The Pi's own Wi-Fi radio is the real limit, not this setting: roughly 50–100 Mbit/s total
        for every connected viewer combined, shared with the web interface itself.
      </p>
      <div style={{ display: "flex", alignItems: "center", gap: 12, flexWrap: "wrap" }}>
        <label>
          <input
            type="checkbox"
            checked={settings.stream_enabled}
            disabled={disabled || busy}
            onChange={(e) => void save({ ...settings, stream_enabled: e.currentTarget.checked })}
          />{" "}
          Enable spectator stream
        </label>
        <label>
          Quality{" "}
          <select
            value={settings.stream_quality}
            disabled={disabled || busy || !settings.stream_enabled}
            onChange={(e) => void save({ ...settings, stream_quality: e.currentTarget.value })}
          >
            <option value="high">High (original quality, ~1–2 viewers)</option>
            <option value="event">Event (lower quality, more viewers)</option>
          </select>
        </label>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
    </section>
  );
}
