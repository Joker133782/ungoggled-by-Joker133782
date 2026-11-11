import { useState, type FormEvent } from "react";
import { api, headers } from "../api";
import { PanelHead } from "./PanelHead";
import { IconCheck, IconLock } from "./icons";

/**
 * Settings → Password: change the control password. The server keeps the
 * Pi's SSH/root password and the terminal's password identical to it, logs
 * out every other session and hands this browser a new session cookie, so
 * it stays logged in here.
 */
export function PasswordPanel({ disabled }: { disabled: boolean }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const [confirm, setConfirm] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  const [done, setDone] = useState(false);
  async function submit(e: FormEvent) {
    e.preventDefault();
    setError("");
    setDone(false);
    // The same rules as the server, so the common mistakes don't need a
    // round trip (and don't count as a wrong guess).
    if (next.length < 8) {
      setError("Use at least 8 characters for the new password.");
      return;
    }
    if (/[\u0000-\u001f\u007f]/.test(next)) {
      setError("The new password can't contain line breaks or tabs.");
      return;
    }
    if (next !== confirm) {
      setError("The new passwords don't match.");
      return;
    }
    setBusy(true);
    try {
      await api("auth/change-password", {
        method: "POST",
        headers,
        body: JSON.stringify({ current, new: next }),
      });
      setCurrent("");
      setNext("");
      setConfirm("");
      setDone(true);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel password" id="password">
      <PanelHead icon={<IconLock />} title="Password" wiki="password" />
      <form onSubmit={(e) => void submit(e)}>
        <div className="password-fields">
          <label>
            Current password
            <input
              type="password"
              value={current}
              required
              autoComplete="current-password"
              onChange={(e) => setCurrent(e.target.value)}
            />
          </label>
          <label>
            New password
            <input
              type="password"
              value={next}
              required
              autoComplete="new-password"
              onChange={(e) => setNext(e.target.value)}
            />
          </label>
          <label>
            Confirm new password
            <input
              type="password"
              value={confirm}
              required
              autoComplete="new-password"
              onChange={(e) => setConfirm(e.target.value)}
            />
          </label>
          <button disabled={busy || disabled}>
            <IconCheck /> {busy ? "Changing…" : "Change password"}
          </button>
        </div>
      </form>
      {done && (
        <p role="status" className="status-line">
          Password changed. It is now also the Pi&apos;s SSH/root password.
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <p className="help">
        Also the SSH/root password. Other devices log in again after a change.
        Forgot it?{" "}
        <a href="/wiki.html#password" target="_blank" rel="noreferrer">
          Help
        </a>
      </p>
    </section>
  );
}
