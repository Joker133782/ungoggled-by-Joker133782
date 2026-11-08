import { useEffect, useRef, useState } from "react";
import { api, headers, usePolling } from "../api";
import type { useNextcloudRelease } from "../nextcloud";
import { download as downloadFromNextcloud } from "../nextcloud";
import { PanelHead } from "./PanelHead";
import {
  IconUpdate,
  IconReconnect,
  IconDownload,
  IconUpload,
  IconCheck,
  IconClock,
} from "./icons";
type UpdateState = {
  id: string | null;
  phase: string;
  version: string | null;
  message: string;
  warnings: string[];
  release_notes: string | null;
};
type Info = {
  available: boolean;
  current_version: string;
  current_build?: string;
  state?: UpdateState;
  error?: string;
};
const pendingKey = "ungoggled-pending-update";
function remembered(): string | null {
  try {
    return sessionStorage.getItem(pendingKey);
  } catch {
    return null;
  }
}
function remember(id: string | null) {
  try {
    if (id) sessionStorage.setItem(pendingKey, id);
    else sessionStorage.removeItem(pendingKey);
  } catch {
    /* Optional. */
  }
}
const active = (phase?: string) =>
  ["queued", "installing", "restarting"].includes(phase ?? "");
function bytes(n: number): string {
  if (n < 1024 * 1024) return `${Math.max(1, Math.round(n / 1024))} KB`;
  return `${(n / (1024 * 1024)).toFixed(1)} MB`;
}
/** How long a check result counts as current before the next visit re-checks. */
const RECHECK_MS = 5 * 60 * 1000;

export function SoftwareUpdate({
  nextcloud,
  showRelease,
  setShowRelease,
  onBusy,
  visible,
}: {
  nextcloud: ReturnType<typeof useNextcloudRelease>;
  showRelease: boolean;
  setShowRelease: (v: boolean) => void;
  onBusy: (v: boolean) => void;
  /** Whether the System tab (where this panel lives) is on screen. */
  visible: boolean;
}) {
  // Checking for updates doubles as the reachability probe: opening the tab
  // shows straight away whether this phone can get updates from here, or
  // needs a file instead. Quietly re-checked when the phone comes back
  // online, and on a later visit once the last result is a few minutes old.
  const { check, checkedAt } = nextcloud;
  useEffect(() => {
    if (!visible) return;
    if (checkedAt == null || Date.now() - checkedAt > RECHECK_MS) void check();
    const online = () => void check();
    window.addEventListener("online", online);
    return () => window.removeEventListener("online", online);
    // checkedAt only gates the check on arrival, it isn't a trigger.
  }, [visible, check]);
  const { value: polled, error: pollingError } = usePolling<Info>(
    "update",
    1000,
  );
  const [uploaded, setUploaded] = useState<UpdateState | null>(null);
  const [progress, setProgress] = useState<number | null>(null);
  const [error, setError] = useState("");
  const [dragOver, setDragOver] = useState(false);
  const dragDepth = useRef(0);
  const [pending, setPending] = useState<string | null>(remembered);
  const [sending, setSending] = useState(false);
  const request = useRef<XMLHttpRequest | null>(null);
  const previousBuild = useRef<string | undefined>(undefined);
  const [downloadProgress, setDownloadProgress] = useState<number | null>(null);
  const [downloadError, setDownloadError] = useState("");
  const [splash, setSplash] = useState(false);
  const expectSplash = useRef(false);
  // The immediate upload result bridges the next status poll only.
  const state = uploaded ?? polled?.state;
  const busy = active(state?.phase) || sending || !!pending;
  useEffect(() => {
    if (polled?.state?.id === uploaded?.id) setUploaded(null);
  }, [polled, uploaded?.id]);
  useEffect(() => {
    onBusy(busy);
  }, [busy, onBusy]);
  useEffect(() => () => request.current?.abort(), []);
  useEffect(() => {
    if (!polled?.state) return;
    const s = polled.state;
    if (pending && s.id !== pending && !sending) {
      // Another browser may have replaced a prepared update while we were away.
      remember(null);
      setPending(null);
    }
    if (
      s.id === pending &&
      ["succeeded", "rolled_back", "failed"].includes(s.phase)
    ) {
      remember(null);
      setPending(null);
      setSending(false);
      if (s.phase === "succeeded") window.location.reload();
    }
    if (
      previousBuild.current &&
      polled.current_build &&
      previousBuild.current !== polled.current_build &&
      s.phase === "succeeded"
    )
      window.location.reload();
    previousBuild.current = polled.current_build;
  }, [polled, pending, sending]);
  useEffect(() => {
    if (expectSplash.current && state?.phase === "ready") {
      expectSplash.current = false;
      setSplash(true);
    }
  }, [state?.phase]);
  const uploadDisabled = !polled?.available || busy || progress !== null;
  function upload(file: File) {
    if (file.size > 64 * 1024 * 1024) {
      setError("Update files must be smaller than 64 MiB.");
      return;
    }
    setError("");
    setProgress(0);
    setUploaded(null);
    const xhr = new XMLHttpRequest();
    request.current = xhr;
    xhr.open("POST", "/api/update/upload");
    xhr.setRequestHeader("X-DJI-Control", "1");
    xhr.timeout = 180000;
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable)
        setProgress(Math.round((e.loaded / e.total) * 100));
    };
    xhr.onload = () => {
      try {
        const result = JSON.parse(xhr.responseText);
        if (xhr.status >= 200 && xhr.status < 300) setUploaded(result);
        else setError(result.error || "Update validation failed.");
      } catch {
        setError("Invalid response from receiver.");
      }
      setProgress(null);
      request.current = null;
    };
    xhr.onerror = xhr.ontimeout = () => {
      setError("Upload interrupted. Reconnect and try again.");
      setProgress(null);
      request.current = null;
    };
    xhr.onabort = () => {
      setProgress(null);
      request.current = null;
    };
    const form = new FormData();
    form.append("update", file);
    xhr.send(form);
  }
  async function downloadAndStage() {
    if (!nextcloud.latest) return;
    setDownloadError("");
    setDownloadProgress(0);
    try {
      const file = await downloadFromNextcloud(
        nextcloud.latest,
        setDownloadProgress,
      );
      setDownloadProgress(null);
      expectSplash.current = true;
      upload(file);
    } catch (e) {
      setDownloadProgress(null);
      setDownloadError(
        e instanceof Error ? e.message : "Could not download the update.",
      );
    }
  }
  async function install(skipHealthCheck = false) {
    if (!state?.id) return;
    setError("");
    setSending(true);
    remember(state.id);
    setPending(state.id);
    try {
      await api("update/install", {
        method: "POST",
        headers,
        body: JSON.stringify({
          id: state.id,
          skip_health_check: skipHealthCheck,
        }),
      });
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not start update.");
      remember(null);
      setPending(null);
    } finally {
      setSending(false);
      setUploaded(null);
    }
  }
  // The exact wording install.rs's switch() wraps a failed health check
  // in — only then does retrying without one make sense; some other
  // failure (a corrupt package, a migration error) needs a fresh upload
  // or a different fix instead, not skipping the check.
  const canRetryWithoutHealthCheck =
    state?.phase === "rolled_back" &&
    !!state.id &&
    state.message.includes("did not pass its health check");
  return (
    <section className="panel software-update" id="software-update">
      <PanelHead icon={<IconUpdate />} title="Software update" wiki="updates">
        <span>v{polled?.current_version ?? "—"}</span>
      </PanelHead>
      <Reachability nextcloud={nextcloud} />
      <div className="actions">
        <button
          disabled={nextcloud.checking}
          onClick={() => void nextcloud.check()}
        >
          <IconReconnect />{" "}
          {nextcloud.checking
            ? "Checking…"
            : nextcloud.checked
              ? "Check again"
              : "Check for updates"}
        </button>
      </div>
      {nextcloud.latest && (
        <div className="release-summary">
          <button onClick={() => setShowRelease(!showRelease)}>
            <IconUpdate /> v{nextcloud.latest.version} available (
            {bytes(nextcloud.latest.size)}) ·{" "}
            {showRelease ? "Hide" : "Release notes"}
          </button>
          {showRelease && (
            <div className="release-details">
              <h3>What&apos;s new in v{nextcloud.latest.version}</h3>
              <ul className="release-notes-list">
                {nextcloud.latest.notes.map((note, i) => (
                  <li key={i}>{note}</li>
                ))}
              </ul>
            </div>
          )}
          <div className="actions">
            <button
              className="primary"
              disabled={downloadProgress !== null || busy}
              onClick={() => void downloadAndStage()}
            >
              <IconDownload /> Download
            </button>
            {downloadProgress !== null && (
              <>
                <progress
                  max={100}
                  value={downloadProgress}
                  aria-label="Update download"
                />
                <span>
                  {downloadProgress === 100
                    ? "Finishing…"
                    : `${downloadProgress}%`}
                </span>
              </>
            )}
          </div>
          {downloadError && (
            <p role="alert" className="error">
              {downloadError}
            </p>
          )}
        </div>
      )}
      {polled && !polled.available && (
        <p className="help">
          Web installation is available on a Pi installed with updater support.
        </p>
      )}
      <div
        className={`update-upload-zone ${dragOver ? "drag-over" : ""} ${uploadDisabled ? "disabled" : ""}`}
        onDragEnter={(e) => {
          e.preventDefault();
          if (uploadDisabled) return;
          dragDepth.current += 1;
          setDragOver(true);
        }}
        onDragOver={(e) => {
          e.preventDefault();
        }}
        onDragLeave={(e) => {
          e.preventDefault();
          dragDepth.current = Math.max(0, dragDepth.current - 1);
          if (dragDepth.current === 0) setDragOver(false);
        }}
        onDrop={(e) => {
          e.preventDefault();
          dragDepth.current = 0;
          setDragOver(false);
          const file = e.dataTransfer.files?.[0];
          if (file && !uploadDisabled) upload(file);
        }}
      >
        <div className="actions update-upload">
          <label className={`button ${uploadDisabled ? "disabled" : ""}`}>
            <IconUpload /> Upload update
            <input
              className="file-input"
              type="file"
              accept=".gz,.tgz"
              disabled={uploadDisabled}
              onChange={(e) => {
                const file = e.target.files?.[0];
                e.target.value = "";
                if (file) upload(file);
              }}
            />
          </label>
          {progress !== null && (
            <>
              <progress max={100} value={progress} aria-label="Update upload" />
              <span>
                {progress === 100 ? "Checking file…" : `${progress}%`}
              </span>
            </>
          )}
        </div>
        <p className="help">
          Choose the release’s .update.tar.gz file, or drag and drop it here.
          Video pauses during installation; compatible settings are retained.
        </p>
      </div>
      {state?.phase === "ready" && (
        <div className="update-ready">
          <strong>v{state.version} ready to install</strong>
          {state.release_notes && (
            <details>
              <summary>Package release notes</summary>
              <div className="release-notes">{state.release_notes}</div>
            </details>
          )}
          <button
            className="primary"
            disabled={busy || progress !== null}
            onClick={() => void install()}
          >
            <IconCheck /> Install v{state.version}
          </button>
        </div>
      )}
      {busy && (
        <p role="status">
          {pollingError
            ? "Receiver restarting. Waiting for it to reconnect…"
            : state?.message || "Starting update…"}
        </p>
      )}
      {state &&
        ["succeeded", "rolled_back", "failed"].includes(state.phase) && (
          <p
            role="status"
            className={state.phase === "succeeded" ? "" : "error"}
          >
            {state.message}
          </p>
        )}
      {canRetryWithoutHealthCheck && (
        <div className="update-skip-health-check">
          <p className="help">
            If the new version actually looked fine on the HDMI screen before
            this reverted it, the health check itself may be wrong rather than
            the update — this has happened before. Skipping it keeps the new
            version without that automatic verification.
          </p>
          <button disabled={busy} onClick={() => void install(true)}>
            <IconCheck /> Retry v{state?.version} without the health check
          </button>
        </div>
      )}
      {!!state?.warnings.length && (
        <ul>
          {state.warnings.map((w) => (
            <li key={w}>{w}</li>
          ))}
        </ul>
      )}
      {(error || polled?.error || (!busy && pollingError)) && (
        <p role="alert" className="error">
          {error || polled?.error || "Cannot reach the receiver."}
        </p>
      )}
      <UpdateReadySplash
        open={splash}
        version={state?.version ?? ""}
        notes={nextcloud.latest?.notes ?? []}
        busy={busy}
        onInstallNow={() => {
          setSplash(false);
          void install();
        }}
        onInstallLater={() => setSplash(false)}
      />
    </section>
  );
}
/**
 * One quiet line (dot + text) saying whether this phone can reach the update
 * server — the Pi itself never does; the phone downloads and hands the file
 * over. Offline isn't an error, just a pointer to the file upload instead.
 */
function Reachability({
  nextcloud,
}: {
  nextcloud: ReturnType<typeof useNextcloudRelease>;
}) {
  const { reach, latest, error } = nextcloud;
  const tone =
    reach === "checking" || reach === "unchecked"
      ? "neutral"
      : reach === "reachable"
        ? latest
          ? "found"
          : "good"
        : "warn";
  const text =
    reach === "unchecked"
      ? "Not checked for updates yet."
      : reach === "checking"
        ? "Checking whether this phone can reach the update server…"
        : reach === "reachable"
          ? latest
            ? `Update found: v${latest.version}.`
            : "Update server reachable from this phone. You're up to date."
          : reach === "offline"
            ? "This phone can't reach the update server right now (no internet here?). Upload an update file below instead."
            : `${error || "The update server answered with an error"}. Upload an update file below instead.`;
  return (
    <p className={`reach ${tone}`} role="status">
      <i aria-hidden="true" />
      <span>{text}</span>
    </p>
  );
}
function UpdateReadySplash({
  open,
  version,
  notes,
  busy,
  onInstallNow,
  onInstallLater,
}: {
  open: boolean;
  version: string;
  notes: string[];
  busy: boolean;
  onInstallNow: () => void;
  onInstallLater: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      className="confirm update-splash"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onInstallLater();
      }}
    >
      <h3>v{version} downloaded and ready</h3>
      {notes.length > 0 && (
        <ul className="release-notes-list">
          {notes.map((note, i) => (
            <li key={i}>{note}</li>
          ))}
        </ul>
      )}
      <p className="help">
        Install now, or install later from the Software update section — the
        download stays on the Pi either way.
      </p>
      <div className="actions">
        <button disabled={busy} onClick={onInstallLater}>
          <IconClock /> Install later
        </button>
        <button className="primary" disabled={busy} onClick={onInstallNow}>
          <IconCheck /> {busy ? "Working…" : "Install now"}
        </button>
      </div>
    </dialog>
  );
}
