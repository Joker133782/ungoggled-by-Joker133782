import { useCallback, useEffect, useState } from "react";
import { api, headers } from "../api";
import { ConfirmDialog } from "./ConfirmDialog";
import { PanelHead } from "./PanelHead";
import { IconUpdate, IconReconnect } from "./icons";

type Release = {
  name: string;
  version: string;
  build: string | null;
  installed: number | null;
  current: boolean;
  factory: boolean;
};
type Entry = {
  time: number;
  action: string;
  from: string | null;
  to: string | null;
  version: string | null;
  result: string;
  message: string;
};
type Listing = { releases: Release[]; history: Entry[] };
type UpdateInfo = { state?: { id: string | null; phase: string } };

const actions: Record<string, string> = {
  install: "Update",
  rollback: "Switch",
  recovery: "Recovery",
};
const results: Record<string, string> = {
  succeeded: "succeeded",
  rolled_back: "undone",
  failed: "failed",
};
const when = (s: number | null) =>
  s ? new Date(s * 1000).toLocaleString() : "unknown date";
const shortBuild = (name: string) => name.slice(name.lastIndexOf("-") + 1);

/** Installed versions on the card, switching between them, and the update log. */
export function Versions({ disabled }: { disabled: boolean }) {
  const [listing, setListing] = useState<Listing | null>(null);
  const [choice, setChoice] = useState<Release | null>(null);
  const [job, setJob] = useState<string | null>(null);
  const [sending, setSending] = useState(false);
  const [error, setError] = useState("");
  const load = useCallback(async () => {
    try {
      setListing(await api<Listing>("update/releases"));
    } catch {
      /* The receiver may be restarting; the next refresh retries. */
    }
  }, []);
  useEffect(() => {
    void load();
    const timer = setInterval(() => void load(), job ? 2000 : 15000);
    return () => clearInterval(timer);
  }, [load, job]);
  // Follow a switch across the receiver restart; reload the matching UI after.
  useEffect(() => {
    if (!job) return;
    const timer = setInterval(async () => {
      try {
        const info = await api<UpdateInfo>("update");
        const s = info.state;
        if (s?.id !== job) return;
        if (s.phase === "succeeded") window.location.reload();
        if (s.phase === "rolled_back" || s.phase === "failed") {
          setJob(null);
          void load();
        }
      } catch {
        /* Restarting. */
      }
    }, 1000);
    return () => clearInterval(timer);
  }, [job, load]);
  async function activate(release: Release) {
    setSending(true);
    setError("");
    try {
      const r = await api<{ id: string }>("update/rollback", {
        method: "POST",
        headers,
        body: JSON.stringify({ release: release.name }),
      });
      setJob(r.id);
      setChoice(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : "Could not switch version.");
      setChoice(null);
    } finally {
      setSending(false);
    }
  }
  if (!listing) return null;
  const blocked = disabled || sending || !!job;
  return (
    <section className="panel versions">
      <PanelHead
        icon={<IconUpdate />}
        title="Installed versions"
        wiki="updates"
      />
      <p className="help">
        The card keeps the active version, the one before it, one older update
        and the version from the SD-card image. Switching back uses the same
        safe procedure as an update: settings are backed up and the previous
        state returns automatically if the version does not start.
      </p>
      <ul className="release-list">
        {listing.releases.map((r) => (
          <li key={r.name} className={r.current ? "current" : ""}>
            <div>
              <strong>v{r.version}</strong>{" "}
              <span className="build">{shortBuild(r.name)}</span>
              {r.current && <span className="badge">active</span>}
              {r.factory && <span className="badge">SD-card image</span>}
              <div className="help">installed {when(r.installed)}</div>
            </div>
            {!r.current && (
              <button disabled={blocked} onClick={() => setChoice(r)}>
                <IconReconnect /> Switch to this version
              </button>
            )}
          </li>
        ))}
      </ul>
      {job && (
        <p role="status">
          Switching version. Video pauses; this page reloads when done…
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <details className="update-history">
        <summary>Update history ({listing.history.length})</summary>
        {listing.history.length === 0 ? (
          <p className="help">No updates recorded yet.</p>
        ) : (
          <ol>
            {listing.history.map((e, i) => (
              <li key={`${e.time}-${i}`} className={e.result}>
                <span className="when">{when(e.time)}</span>{" "}
                <strong>{actions[e.action] ?? e.action}</strong>
                {e.version && ` to v${e.version}`}
                {e.to && ` (${shortBuild(e.to)})`}
                {" · "}
                {results[e.result] ?? e.result}
                {e.message && <div className="help">{e.message}</div>}
              </li>
            ))}
          </ol>
        )}
      </details>
      <ConfirmDialog
        open={choice !== null}
        busy={sending}
        title={`Switch to v${choice?.version ?? ""}?`}
        body={
          "Video stops for about half a minute while the receiver restarts. " +
          "Settings are kept where that version supports them. Older versions " +
          "may not offer this list; to come back, upload the newer update file again."
        }
        confirmLabel="Switch version"
        onCancel={() => setChoice(null)}
        onConfirm={() => choice && void activate(choice)}
      />
    </section>
  );
}
