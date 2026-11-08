import { useCallback, useEffect, useRef, useState } from "react";
import { api, headers } from "../api";
import { number } from "../format";
import type { UsbVideo, UsbVideoStatus } from "../types";
import { PanelHead } from "./PanelHead";
import { IconUsb, IconPlay, IconPause, IconStop, IconReconnect } from "./icons";

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
const when = (unixSeconds: number) =>
  new Date(unixSeconds * 1000).toLocaleString(undefined, {
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });

/**
 * Videos found on USB drives plugged into a host port, playable on the HDMI
 * output with play/pause/seek from here. Experimental: codec and resolution
 * support depend on what the Pi's mpv/GStreamer install can decode, which
 * this page cannot know in advance.
 */
export function UsbVideos({
  disabled,
  speed,
  fps,
  clipFps,
}: {
  disabled: boolean;
  /** Playback speed, 1 = real time; null while it is still being measured. */
  speed: number | null;
  /** New pictures per second that reach the screen. */
  fps: number | null;
  /** The clip's own frame rate. */
  clipFps: number | null;
}) {
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
  // Read inside the poll loop below without re-subscribing it on every list
  // update (it already re-fetches the list itself — see thumbnailRetriesLeft
  // below — so depending on `videos` directly would restart this loop on its
  // own output).
  const videosRef = useRef(videos);
  useEffect(() => {
    videosRef.current = videos;
  }, [videos]);
  const thumbnailRetriesLeft = useRef(3);
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      try {
        const s = await api<UsbVideoStatus>("usb-videos/status");
        setStatus(s);
        // A background thumbnailing pass can finish between two polls —
        // even start *and* finish between them, now that a thumbnail is a
        // single un-seeked frame grab rather than something slow enough to
        // reliably catch "in progress". So rather than watching for a
        // scanning-to-idle transition (which a fast pass can simply never
        // show), just check on every idle tick whether a clip we know about
        // still has no thumbnail, and re-fetch the list if so — picking up
        // anything list_videos() already retried for us. Bounded, since a
        // clip mpv can never open would otherwise retry forever.
        if (
          !s.scan &&
          thumbnailRetriesLeft.current > 0 &&
          videosRef.current?.some((v) => !v.thumbnail)
        ) {
          thumbnailRetriesLeft.current -= 1;
          void loadList();
        }
      } catch {
        /* Receiver may be restarting; the next tick retries. */
      }
      timer = setTimeout(poll, 1000);
    };
    void poll();
    return () => clearTimeout(timer);
  }, [loadList]);
  // The list itself is only ever fetched above (on mount) or from Rescan —
  // re-fetch it whenever the set of attached drives actually changes, so an
  // unplugged drive's clips are purged (and a newly plugged one's appear)
  // without the user having to notice and click Rescan themselves.
  const drivesKey = status?.drives.join("\0") ?? "";
  const lastDrivesKey = useRef(drivesKey);
  useEffect(() => {
    if (drivesKey !== lastDrivesKey.current) {
      lastDrivesKey.current = drivesKey;
      thumbnailRetriesLeft.current = 3;
      void loadList();
    }
  }, [drivesKey, loadList]);

  async function play(video: UsbVideo) {
    setBusy(video.id);
    setError("");
    // A drag on the previous clip's seek bar left this set, and it
    // overrides status.position in the render below — switching clips
    // without clearing it kept showing the old clip's last scrub position
    // forever (not just until the next poll, since nothing else resets it).
    setScrub(null);
    // Optimistic: show the new clip at 0:00 immediately, rather than the
    // previous clip's stale position/duration until the next 1s status poll
    // catches up with the switch.
    setStatus((s) =>
      s
        ? {
            ...s,
            playing: true,
            name: video.name,
            position: 0,
            duration: 0,
            paused: false,
          }
        : s,
    );
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
      setStatus((s) => (s ? { ...s, playing: false } : s));
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

  // Nothing to list once the drive is gone, even before the list itself has
  // been fetched again (an unplugged camera's clips must not linger).
  const shown = status && status.drives.length === 0 ? [] : videos;
  const playing = status?.playing ?? false;
  const duration = status?.duration ?? 0;
  const position = scrub ?? status?.position ?? 0;

  return (
    <section className="panel usb-videos">
      <PanelHead
        icon={<IconUsb />}
        title="USB - ActionCam Video Player"
        experimental
        wiki="usb"
      />
      <p className="help">
        {status && status.drives.length > 0
          ? `Detected: ${status.drives.join(", ")}`
          : "No USB drive detected."}
      </p>
      {status?.scan && (
        <div className="usb-scan">
          <progress
            max={Math.max(1, status.scan.total)}
            value={status.scan.done}
            aria-label="Scanning for videos"
          />
          <span className="help">
            Generating thumbnails… {status.scan.done}/{status.scan.total}
          </span>
        </div>
      )}
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
          <p className="help usb-rate">
            {status?.paused
              ? "Paused"
              : speed == null
                ? "Measuring speed…"
                : `${number(fps ?? 0)} fps on screen · ${number(speed, 2)}× speed${
                    speed >= 0.97 ? " (real time)" : ""
                  }${clipFps ? ` · clip is ${number(clipFps)} fps` : ""}`}
          </p>
          <div style={{ display: "flex", gap: 8 }}>
            <button
              disabled={disabled || busy === "pause" || status?.paused == null}
              onClick={() => void togglePause()}
            >
              {status?.paused ? (
                <>
                  <IconPlay /> Resume
                </>
              ) : (
                <>
                  <IconPause /> Pause
                </>
              )}
            </button>
            <button
              disabled={disabled || busy === "stop"}
              onClick={() => void stop()}
            >
              <IconStop /> Stop
            </button>
          </div>
        </div>
      )}
      <button
        disabled={videos === null || !!status?.scan}
        onClick={() => void loadList()}
      >
        <IconReconnect /> Rescan
      </button>
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      {shown && shown.length === 0 && (
        <p className="help">No USB drive with video files found.</p>
      )}
      {shown && shown.length > 0 && (
        <ul className="usb-video-list">
          {shown.map((v) => (
            <li key={v.id}>
              <Thumbnail video={v} scanning={!!status?.scan} />
              <div style={{ flex: 1, minWidth: 0 }}>
                {v.name}
                <div className="help">
                  {v.device} · {bytes(v.size)}
                  {v.mtime != null ? ` · ${when(v.mtime)}` : ""}
                </div>
              </div>
              <button
                disabled={disabled || busy === v.id}
                onClick={() => void play(v)}
              >
                <IconPlay /> Play
              </button>
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

/**
 * Falls back to a plain placeholder box until a thumbnail exists. Thumbnails
 * are generated in the background after the file list itself appears, so
 * while a scan is running this polls for one instead of giving up after the
 * first miss — only switching to the `<img>` once a HEAD request confirms
 * the file is there, to avoid a flash of a broken-image icon in between.
 */
function Thumbnail({
  video,
  scanning,
}: {
  video: UsbVideo;
  scanning: boolean;
}) {
  const [ready, setReady] = useState(video.thumbnail);
  useEffect(() => {
    if (ready || !scanning) return;
    let cancelled = false;
    const check = async () => {
      try {
        const r = await fetch(`/api/usb-videos/${video.id}/thumbnail`, {
          method: "HEAD",
          cache: "no-store",
        });
        if (!cancelled && r.ok) setReady(true);
      } catch {
        /* retried on the next tick */
      }
    };
    const timer = setInterval(() => void check(), 2000);
    return () => {
      cancelled = true;
      clearInterval(timer);
    };
  }, [ready, scanning, video.id]);
  if (!ready) {
    return (
      <div className="usb-thumbnail usb-thumbnail-empty" aria-hidden="true" />
    );
  }
  return (
    <img
      className="usb-thumbnail"
      src={`/api/usb-videos/${video.id}/thumbnail`}
      alt=""
      onError={() => setReady(false)}
    />
  );
}
