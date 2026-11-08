import { lazy, Suspense, useEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import type { Settings, TerminalStatus } from "../types";
import { api, headers, usePolling } from "../api";
import { PanelHead } from "./PanelHead";
import { ConfirmDialog } from "./ConfirmDialog";
import { IconClose, IconTerminal } from "./icons";

// xterm.js only loads once a terminal actually opens (it's most of a
// megabyte); `preload` starts fetching it while the password is typed.
const loadScreen = () => import("./TerminalScreen");
const TerminalScreen = lazy(loadScreen);

const minutes = (s: number) => `${Math.round(s / 60)} minutes`;

/**
 * System → Terminal: a root shell on the Pi, in the browser. Off unless
 * switched on in Settings; every opening asks for the password again (see
 * terminal.rs on the Pi for everything that is enforced there).
 */
export function TerminalPanel({ disabled }: { disabled: boolean }) {
  const { value: status } = usePolling<TerminalStatus>("terminal", 3000);
  const [askPassword, setAskPassword] = useState(false);
  const [ticket, setTicket] = useState<string | null>(null);
  return (
    <section className="panel terminal-panel" id="terminal">
      <PanelHead icon={<IconTerminal />} title="Terminal" wiki="terminal" />
      {!status ? (
        <p className="status-line">Loading…</p>
      ) : !status.enabled ? (
        <p className="status-line">
          Off. A root shell on the Pi, for troubleshooting with someone who
          knows Linux. Turn it on in{" "}
          <a href="#settings/terminal-settings">Settings → Terminal</a>.
        </p>
      ) : (
        <>
          <p className="status-line">
            A root shell on the Pi. Asks for the password every time it opens,
            and closes after {minutes(status.idle_seconds)} without typing or{" "}
            {minutes(status.max_seconds)} at most.
          </p>
          {status.active && !ticket && (
            <p className="status-line warning-text">
              A terminal is already open, on this or another device. Only one
              can be open at a time.
            </p>
          )}
          <div className="actions">
            <button
              disabled={disabled || status.active || !!ticket}
              onClick={() => {
                void loadScreen();
                setAskPassword(true);
              }}
            >
              <IconTerminal /> Open terminal
            </button>
          </div>
        </>
      )}
      <StepUpDialog
        open={askPassword}
        onCancel={() => setAskPassword(false)}
        onTicket={(t) => {
          setAskPassword(false);
          setTicket(t);
        }}
      />
      {/* Rendered at the end of <body>: inside this panel, whose
          backdrop-filter turns "position: fixed" into "fixed to the panel",
          the full-screen terminal would be confined to the panel. */}
      {ticket &&
        createPortal(
          <Suspense
            fallback={
              <div className="terminal-screen terminal-loading">
                Loading the terminal…
              </div>
            }
          >
            <TerminalScreen ticket={ticket} onClose={() => setTicket(null)} />
          </Suspense>,
          document.body,
        )}
    </section>
  );
}

/** The password again, in exchange for a single-use, seconds-long ticket. */
function StepUpDialog({
  open,
  onCancel,
  onTicket,
}: {
  open: boolean;
  onCancel: () => void;
  onTicket: (ticket: string) => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const [password, setPassword] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) {
      setPassword("");
      setError("");
      dialog.showModal();
    }
    if (!open && dialog.open) dialog.close();
  }, [open]);
  async function submit() {
    setBusy(true);
    setError("");
    try {
      const r = await api<{ ticket: string }>("terminal/session", {
        method: "POST",
        headers,
        body: JSON.stringify({ password }),
      });
      setPassword("");
      onTicket(r.ticket);
    } catch (e) {
      setPassword("");
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <dialog
      ref={ref}
      className="confirm"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onCancel();
      }}
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          void submit();
        }}
      >
        <h3>Open a root shell?</h3>
        <p>
          Commands run on the Pi immediately, as root: one wrong command can
          break the installation. Enter the control password to continue.
        </p>
        <label className="dialog-field">
          Password
          <input
            type="password"
            autoComplete="current-password"
            value={password}
            required
            disabled={busy}
            onChange={(e) => setPassword(e.target.value)}
          />
        </label>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <div className="actions">
          <button type="button" disabled={busy} onClick={onCancel}>
            <IconClose /> Cancel
          </button>
          <button className="primary" disabled={busy || !password}>
            <IconTerminal /> {busy ? "Checking…" : "Open terminal"}
          </button>
        </div>
      </form>
    </dialog>
  );
}

/** Settings → Terminal: whether the terminal can be opened at all. */
export function TerminalSettings({
  disabled,
  settings,
  onSave,
}: {
  disabled: boolean;
  settings: Settings | null;
  onSave: (next: Settings) => Promise<void>;
}) {
  const [confirming, setConfirming] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState("");
  async function set(terminal_enabled: boolean) {
    if (!settings) return;
    setBusy(true);
    setError("");
    try {
      await onSave({ ...settings, terminal_enabled });
      setConfirming(false);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <section className="panel terminal-settings" id="terminal-settings">
      <PanelHead icon={<IconTerminal />} title="Terminal" wiki="terminal" />
      <ul className="toggle-list">
        <li>
          <label className="toggle">
            <input
              type="checkbox"
              checked={settings?.terminal_enabled ?? false}
              disabled={disabled || busy || !settings}
              onChange={(e) => {
                if (e.currentTarget.checked) setConfirming(true);
                else void set(false);
              }}
            />
            Allow a root shell in the browser
          </label>
          <span className="help">
            Opened from System → Terminal. Off by default; turning it off closes
            an open terminal at once.
          </span>
        </li>
      </ul>
      {settings?.terminal_enabled && (
        <p className="status-line">
          On. Open it from <a href="#system/terminal">System → Terminal</a>.
        </p>
      )}
      {error && (
        <p role="alert" className="error">
          {error}
        </p>
      )}
      <ConfirmDialog
        open={confirming}
        busy={busy}
        title="Allow the terminal?"
        body="Anyone who knows the control password can then run any command on the Pi as root from this page. It still asks for the password every time it opens, and only one can be open at a time."
        confirmLabel="Allow"
        onCancel={() => setConfirming(false)}
        onConfirm={() => void set(true)}
      />
    </section>
  );
}
