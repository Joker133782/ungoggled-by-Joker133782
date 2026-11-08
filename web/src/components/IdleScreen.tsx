import { useState, type ReactNode } from "react";
import type { ImageEntry, ImageLibrary, Settings, Status } from "../types";
import { api, headers } from "../api";
import { PanelHead } from "./PanelHead";
import { ConfirmDialog } from "./ConfirmDialog";
import {
  IconCheck,
  IconImage,
  IconPlay,
  IconTrash,
  IconUpload,
  IconVideos,
} from "./icons";

/** What the idle screen switches to when the selected one is deleted. */
const DEFAULT_ID = "default-loop.h264";

function megabytes(n: number): string {
  return n < 1024 * 1024
    ? `${Math.max(1, Math.round(n / 1024))} KB`
    : `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
function details(entry: ImageEntry): string {
  const parts: string[] = [];
  if (entry.width && entry.height) parts.push(`${entry.width}×${entry.height}`);
  if (entry.kind === "video" && entry.fps)
    parts.push(`${Math.round(entry.fps)} fps`);
  if (!entry.builtin) parts.push(megabytes(entry.size));
  return parts.join(" · ");
}

/**
 * The idle screen: what the TV shows while there's no live picture. A list
 * like the USB player's — thumbnail, name, what it is, and one button. That
 * button reads "Play" while the idle screen is what's on the TV (it switches
 * right away) and "Use" while live video is (it becomes the next idle screen
 * without touching the live picture; the Pi only ever shows it when there's
 * no live video).
 */
export function IdleScreen({
  library,
  settings,
  status,
  disabled,
  onSave,
  onLibraryChange,
}: {
  library: ImageLibrary | null;
  settings: Settings | null;
  status: Status | null;
  disabled: boolean;
  onSave: (next: Settings) => Promise<void>;
  onLibraryChange: () => Promise<void>;
}) {
  const [busy, setBusy] = useState<string | null>(null);
  const [error, setError] = useState("");
  const [deleting, setDeleting] = useState<ImageEntry | null>(null);
  const selected = settings?.fallback_image ?? null;
  // The idle screen is on the TV right now, so choosing one shows it at once.
  const onTv = status?.hdmi === "fallback";
  const live = ["playing", "starting", "waiting for frames"].includes(
    status?.hdmi ?? "",
  );
  const blocked = disabled || !settings || busy !== null;

  async function task(label: string, run: () => Promise<void>) {
    setBusy(label);
    setError("");
    try {
      await run();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  }
  function choose(id: string | null) {
    if (!settings) return;
    void task(id ?? "dark", () => onSave({ ...settings, fallback_image: id }));
  }
  function upload(file: File) {
    if (!settings) return;
    void task("upload", async () => {
      const data = new FormData();
      data.append("image", file);
      const uploaded = await api<{ id: string }>("images", {
        method: "POST",
        headers: { "X-DJI-Control": "1" },
        body: data,
      });
      await onLibraryChange();
      await onSave({ ...settings, fallback_image: uploaded.id });
    });
  }
  function remove(entry: ImageEntry) {
    if (!settings) return;
    void task(`delete:${entry.id}`, async () => {
      // The Pi refuses to delete the idle screen in use; switch away first.
      if (selected === entry.id)
        await onSave({ ...settings, fallback_image: DEFAULT_ID });
      await api(`images/${entry.id}`, { method: "DELETE", headers });
      await onLibraryChange();
    }).then(() => setDeleting(null));
  }

  const where = onTv
    ? "On the TV now."
    : live
      ? "Live video is on the TV; your choice shows whenever the signal drops."
      : status?.hdmi_connected === false
        ? "No TV connected; your choice shows once one is."
        : "Shows on the TV whenever there's no live picture.";
  const limits = library?.limits;
  return (
    <section className="panel idle-screen" id="idle-screen">
      <PanelHead icon={<IconImage />} title="Idle screen" wiki="hdmi" />
      <p className="status-line">
        What the TV shows while there&apos;s no live picture: before the goggles
        connect, or when the signal drops. {where}
      </p>
      <ul className="media-list" aria-label="Idle screens">
        <MediaRow
          name="Dark screen"
          chips={["Built-in"]}
          detail="Plain dark background"
          thumb={<span className="media-thumb media-thumb-dark" />}
          selected={selected === null}
          onTv={onTv}
          busy={busy === "dark"}
          disabled={blocked}
          onChoose={() => choose(null)}
        />
        {library?.images.map((entry) => (
          <MediaRow
            key={entry.id}
            name={entry.name}
            chips={[
              entry.kind === "video" ? "Video" : "Image",
              ...(entry.builtin ? ["Built-in"] : []),
            ]}
            detail={details(entry)}
            thumb={<Thumb entry={entry} />}
            selected={selected === entry.id}
            onTv={onTv}
            busy={busy === entry.id}
            disabled={blocked}
            onChoose={() => choose(entry.id)}
            onDelete={entry.builtin ? undefined : () => setDeleting(entry)}
          />
        ))}
      </ul>
      {!library && <p className="status-line">Loading…</p>}
      <div className="actions">
        <label className={`button ${blocked ? "disabled" : ""}`}>
          <IconUpload />
          {busy === "upload" ? "Uploading…" : "Upload image or video"}
          <input
            className="file-input"
            type="file"
            accept="image/png,image/jpeg,image/webp,video/mp4,video/quicktime,.mp4,.mov"
            disabled={blocked}
            onChange={(e) => {
              const file = e.target.files?.[0];
              e.target.value = "";
              if (file) upload(file);
            }}
          />
        </label>
      </div>
      {limits && (
        <p className="status-line centered">
          Your videos: {limits.videos.used} of {limits.videos.max} · your
          images: {limits.images.used} of {limits.images.max} (built-ins
          don&apos;t count)
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <p className="help">
        Images: PNG, JPEG or WebP, up to 12 MiB, shown whole (never cropped).
        Videos: MP4 with H.264 (not HEVC), up to 1920×1080 and 64 MiB, played in
        an endless loop; converting one takes a few seconds. An upload becomes
        the idle screen straight away.
      </p>
      <ConfirmDialog
        open={deleting !== null}
        busy={busy !== null}
        title={`Delete “${deleting?.name ?? ""}”?`}
        body={
          deleting?.id === selected
            ? "It's the current idle screen, so the Logo loop takes over. The file is removed from the Pi."
            : "The file is removed from the Pi."
        }
        confirmLabel="Delete"
        onCancel={() => setDeleting(null)}
        onConfirm={() => deleting && remove(deleting)}
      />
    </section>
  );
}

function MediaRow({
  name,
  chips,
  detail,
  thumb,
  selected,
  onTv,
  busy,
  disabled,
  onChoose,
  onDelete,
}: {
  name: string;
  chips: string[];
  detail: string;
  thumb: ReactNode;
  selected: boolean;
  onTv: boolean;
  busy: boolean;
  disabled: boolean;
  onChoose: () => void;
  onDelete?: () => void;
}) {
  return (
    <li className={selected ? "selected" : ""}>
      {thumb}
      <div className="media-text">
        <span className="media-name">{name}</span>
        <span className="media-meta">
          {chips.map((c) => (
            <span key={c} className="chip">
              {c}
            </span>
          ))}
          {detail && <span>{detail}</span>}
        </span>
      </div>
      {onDelete && (
        <button
          className="icon-button"
          aria-label={`Delete ${name}`}
          title="Delete"
          disabled={disabled}
          onClick={onDelete}
        >
          <IconTrash />
        </button>
      )}
      {selected ? (
        <span className={`pill current ${onTv ? "on-tv" : ""}`}>
          {onTv ? (
            <>
              <i aria-hidden="true" /> On TV
            </>
          ) : (
            <>
              <IconCheck /> In use
            </>
          )}
        </span>
      ) : (
        <button
          className="pill"
          disabled={disabled}
          aria-label={onTv ? `Play ${name} on the TV now` : `Use ${name}`}
          onClick={onChoose}
        >
          {onTv ? <IconPlay /> : <IconCheck />}
          {busy ? "…" : onTv ? "Play" : "Use"}
        </button>
      )}
    </li>
  );
}

function Thumb({ entry }: { entry: ImageEntry }) {
  const [failed, setFailed] = useState(false);
  return (
    <span className="media-thumb">
      {failed ? (
        <span className="media-thumb-icon" aria-hidden="true">
          {entry.kind === "video" ? <IconVideos /> : <IconImage />}
        </span>
      ) : (
        <img
          src={entry.thumbnail}
          alt=""
          loading="lazy"
          decoding="async"
          onError={() => setFailed(true)}
        />
      )}
      {entry.kind === "video" && !failed && (
        <span className="media-thumb-badge" aria-hidden="true">
          <IconPlay />
        </span>
      )}
    </span>
  );
}
