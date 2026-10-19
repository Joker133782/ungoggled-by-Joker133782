import { useCallback, useEffect, useState } from "react";
import { api, headers } from "../api";
import type { UsbVideo, UsbVideoStatus } from "../types";

const bytes = (n: number) => {
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  const mb = n / (1024 * 1024);
  return mb < 1024 ? `${mb.toFixed(0)} MB` : `${(mb / 1024).toFixed(1)} GB`;
};
const time = (s: number) => {
  const total = Math.max(0, Math.round(s));
  const m = Math.floor(total / 60);
  const sec = total % 60;
  return `${m}:${sec.toString().padStart(2, "0")}`;
};

/**
 * Videos found on USB drives plugged into a host port, playable on the HDMI
 * output with play/pause/seek from here. Experimental: codec and resolution
 * support depend on what the Pi's mpv/GStreamer install can decode, which
 * this page cannot know in advance.
 */
export function UsbVideos({ disabled }: { disabled: boolean }) {
  const [videos, setVideos] = useState<UsbVideo[] | null>(null);
  const [status, setStatus] = useState<UsbVideoStatus | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [scrub, setScrub] = useState<number | null>(null);

  const loadList = useCallback(async () => {
    try {
      const r = await api<{ videos: UsbVideo[] }>("usb-videos");
      setVideos(r.videos);
      setError("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not list USB videos.");
    }
  }, []);
  useEffect(() => {
    void loadList();
  }, [loadList]);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        setStatus(await api<UsbVideoStatus>("usb-videos/status"));
      } catch {
        /* Receiver may be restarting; the next tick retries. */
      }
      timer = setTimeout(poll, 1000);
    };
    void poll();
    return () => clearTimeout(timer);
  }, []);

  async function play(video: UsbVideo) {
    setBusy(video.id);
    setError("");
    try {
      await api(`usb-videos/${video.id}/play`, { method: "POST", headers });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start playback.");
    } finally {
      setBusy(null);
    }
  }
  async function stop() {
    setBusy("stop");
    try {
      await api("usb-videos/stop", { method: "POST", headers });
      setStatus({ playing: false });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not stop playback.");
    } finally {
      setBusy(null);
    }
  }
  async function togglePause() {
    if (!status?.playing) return;
    const paused = !status.paused;
    setBusy("pause");
    try {
      await api("usb-videos/pause", {
        method: "POST",
        headers,
        body: JSON.stringify({ paused }),
      });
      setStatus((s) => (s ? { ...s, paused } : s));
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not pause playback.");
    } finally {
      setBusy(null);
    }
  }
  async function commitSeek(position: number) {
    setScrub(null);
    try {
      await api("usb-videos/seek", {
        method: "POST",
        headers,
        body: JSON.stringify({ position }),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not seek.");
    }
  }

  const playing = status?.playing ?? false;
  const duration = status?.duration ?? 0;
  const position = scrub ?? status?.position ?? 0;

  return (
    <section className="panel usb-videos">
      <h2>USB videos (experimental)</h2>
      <p className="help">
        Plug an action camera or USB drive into one of the Pi's regular USB-A
        ports (not the goggles port) to list its video files here and play one
        on the HDMI output. Not hardware-verified for every camera and codec.
      </p>
      {playing && (
        <div className="usb-now-playing">
          <p className="help">Playing on the HDMI output</p>
          <p style={{ fontWeight: 500, margin: "0 0 8px" }}>{status?.name}</p>
          <input
            type="range"
            min={0}
            max={Math.max(1, Math.round(duration))}
            step={1}
            value={Math.round(position)}
            disabled={disabled || !status?.duration}
            onChange={(e) => setScrub(Number(e.currentTarget.value))}
            onMouseUp={(e) => void commitSeek(Number(e.currentTarget.value))}
            onTouchEnd={(e) => void commitSeek(Number(e.currentTarget.value))}
          />
          <div className="usb-time-row">
            <span>{time(position)}</span>
            <span>{time(duration)}</span>
          </div>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              disabled={disabled || busy === "pause" || status?.paused == null}
              onClick={() => void togglePause()}
            >
              {status?.paused ? "Resume" : "Pause"}
            </button>
            <button
              disabled={disabled || busy === "stop"}
              onClick={() => void stop()}
            >
              Stop
            </button>
          </div>
        </div>
      )}
      <button disabled={videos === null} onClick={() => void loadList()}>
        Rescan
      </button>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {videos && videos.length === 0 && (
        <p className="help">No USB drive with video files found.</p>
      )}
      {videos && videos.length > 0 && (
        <ul className="usb-video-list">
          {videos.map((v) => (
            <li key={v.id}>
              <Thumbnail video={v} />
              <div style={{ flex: 1, minWidth: 0 }}>
                {v.name}
                <div className="help">
                  {v.device} · {bytes(v.size)}
                </div>
              </div>
              <button
                disabled={disabled || busy === v.id}
                onClick={() => void play(v)}
              >
                Play
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/** Falls back to a plain placeholder box until a thumbnail exists or if the file never gets one. */
function Thumbnail({ video }: { video: UsbVideo }) {
  const [failed, setFailed] = useState(false);
  if (!video.thumbnail || failed) {
    return (
      <div className="usb-thumbnail usb-thumbnail-empty" aria-hidden="true" />
    );
  }
  return (
    <img
      className="usb-thumbnail"
      src={`/api/usb-videos/${video.id}/thumbnail`}
      alt=""
      onError={() => setFailed(true)}
    />
  );
}
