import { useCallback, useEffect, useState } from "react";
import { api, headers } from "../api";
import type { Recording, RecordingStatus } from "../types";

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
const when = (s: number | null) => (s ? new Date(s * 1000).toLocaleString() : "unknown date");

/**
 * Record the live goggles feed to the SD card (raw, no re-encoding), with a
 * "save the last 30 seconds" shortcut for moments you didn't plan to record.
 */
export function RecordingPanel({
  diskWarning,
  disabled,
}: {
  diskWarning?: boolean;
  disabled: boolean;
}) {
  const [status, setStatus] = useState<RecordingStatus | null>(null);
  const [recordings, setRecordings] = useState<Recording[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");

  const loadList = useCallback(async () => {
    try {
      const r = await api<{ recordings: Recording[] }>("recordings");
      setRecordings(r.recordings);
    } catch {
      /* Receiver may be restarting; the next poll retries. */
    }
  }, []);
  useEffect(() => {
    void loadList();
  }, [loadList]);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        setStatus(await api<RecordingStatus>("recording"));
      } catch {
        /* Receiver may be restarting; the next tick retries. */
      }
      timer = setTimeout(poll, 2000);
    };
    void poll();
    return () => clearTimeout(timer);
  }, []);

  async function start() {
    setBusy("start");
    setError("");
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
      await loadList();
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not stop recording.");
    } finally {
      setBusy(null);
    }
  }
  async function saveBuffer() {
    setBusy("buffer");
    setError("");
    try {
      await api("recording/save-buffer", { method: "POST", headers });
      await loadList();
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
    }
  }

  const recording = status?.recording ?? false;
  return (
    <section className="panel recording">
      <h2>Recording</h2>
      <p className="help">
        Records the live goggles feed to the SD card, unmodified (no re-encoding). Stops on
        its own with at least 2 GB free space left. The goggles' own on-screen display (battery,
        channel, signal) is part of the recorded picture if the goggles have it turned on;
        that is set on the goggles/DJI Fly, not here.
      </p>
      {diskWarning && (
        <p role="alert" className="error">
          SD card is full; recording stopped. Delete some recordings below, then{" "}
          <button disabled={disabled} onClick={() => void dismissWarning()}>
            dismiss
          </button>
        </p>
      )}
      <div style={{ display: "flex", gap: 8, flexWrap: "wrap" }}>
        {recording ? (
          <button disabled={disabled || busy === "stop"} onClick={() => void stop()}>
            Stop recording ({time(status?.seconds ?? 0)}, {bytes(status?.bytes ?? 0)})
          </button>
        ) : (
          <button disabled={disabled || busy === "start"} onClick={() => void start()}>
            Start recording
          </button>
        )}
        <button disabled={disabled || busy === "buffer"} onClick={() => void saveBuffer()}>
          Save last 30 seconds
        </button>
      </div>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {recordings && recordings.length === 0 && (
        <p className="help">No recordings yet.</p>
      )}
      {recordings && recordings.length > 0 && (
        <ul className="recording-list">
          {recordings.map((r) => (
            <li key={r.name}>
              <div>
                {r.name}
                <div className="help">
                  {when(r.modified)} · {bytes(r.size)}
                </div>
              </div>
              <div style={{ display: "flex", gap: 8 }}>
                <button disabled={disabled || busy === r.name} onClick={() => void play(r.name)}>
                  Play on HDMI
                </button>
                <a href={`/api/recordings/${r.name}`}>
                  <button disabled={disabled}>Download</button>
                </a>
                <button disabled={disabled || busy === r.name} onClick={() => void remove(r.name)}>
                  Delete
                </button>
              </div>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}
