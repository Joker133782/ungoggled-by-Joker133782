import { useCallback, useEffect, useState } from "react";
import { api, headers } from "../api";
import type { UsbVideo } from "../types";

const bytes = (n: number) => {
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  const mb = n / (1024 * 1024);
  return mb < 1024 ? `${mb.toFixed(0)} MB` : `${(mb / 1024).toFixed(1)} GB`;
};

/**
 * Videos found on USB drives plugged into a host port, playable on the HDMI
 * output. Experimental: codec and resolution support depend on what the Pi's
 * GStreamer install can decode, which this page cannot know in advance.
 */
export function UsbVideos({
  playing,
  disabled,
}: {
  playing?: string | null;
  disabled: boolean;
}) {
  const [videos, setVideos] = useState<UsbVideo[] | null>(null);
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      const r = await api<{ videos: UsbVideo[] }>("usb-videos");
      setVideos(r.videos);
      setError("");
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not list USB videos.");
    }
  }, []);
  useEffect(() => {
    void load();
  }, [load]);
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
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not stop playback.");
    } finally {
      setBusy(null);
    }
  }
  return (
    <section className="panel usb-videos">
      <h2>USB videos (experimental)</h2>
      <p className="help">
        Plug an action camera or USB drive into one of the Pi's regular USB-A
        ports (not the goggles port) to list its video files here and play
        one on the HDMI output. Not hardware-verified for every camera and
        codec.
      </p>
      {playing && (
        <p role="status">
          Playing <strong>{playing}</strong> on the HDMI output.{" "}
          <button disabled={disabled || busy === "stop"} onClick={() => void stop()}>
            Stop
          </button>
        </p>
      )}
      <button disabled={videos === null} onClick={() => void load()}>
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
              <div>
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
