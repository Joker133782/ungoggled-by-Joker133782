import {
  useEffect,
  useRef,
  useState,
  type FormEvent,
  type ReactNode,
} from "react";
import { api, headers } from "../api";
import { IconGoggles } from "./icons";

interface AuthStatus {
  configured: boolean;
  authenticated: boolean;
}

// Wraps the whole app: nothing behind it renders until a session cookie is
// confirmed. `/watch.html` and the stream endpoint never go through this —
// they're served off a separate, unauthenticated route (see src/api.rs) so
// a spectator link never doubles as a way into the control interface.
export function AuthGate({ children }: { children: ReactNode }) {
  const [status, setStatus] = useState<AuthStatus | null>(null);
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState("");
  const [busy, setBusy] = useState(false);
  const mounted = useRef(true);
  useEffect(() => {
    mounted.current = true;
    return () => {
      mounted.current = false;
    };
  }, []);
  async function refresh() {
    try {
      const s = await api<AuthStatus>("auth");
      if (mounted.current) setStatus(s);
    } catch {
      // Receiver unreachable; try again shortly rather than getting stuck.
    }
  }
  useEffect(() => {
    void refresh();
    const timer = setInterval(() => void refresh(), 5000);
    return () => clearInterval(timer);
  }, []);
  if (status?.authenticated) return <>{children}</>;
  if (!status) return null;
  const firstRun = !status.configured;
  async function submit(e: FormEvent) {
    e.preventDefault();
    setError("");
    if (firstRun && password.length < 8) {
      setError("Use at least 8 characters.");
      return;
    }
    if (firstRun && password !== confirm) {
      setError("Passwords don't match.");
      return;
    }
    setBusy(true);
    try {
      await api("auth/login", {
        method: "POST",
        headers,
        body: JSON.stringify({ password }),
      });
      setPassword("");
      setConfirm("");
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="auth-gate">
      <form className="panel" onSubmit={(e) => void submit(e)}>
        <div className="brand">
          <span className="logo">
            <IconGoggles />
          </span>
          <h1>
            <span className="accent-text">un</span>goggled
          </h1>
        </div>
        <h2>{firstRun ? "Set a control password" : "Log in"}</h2>
        {firstRun && (
          <p className="help">
            Anyone with this password can control the receiver, change settings
            and reach your recordings. The spectator stream page doesn't need it
            and won't ask for it.
          </p>
        )}
        <div className="auth-fields">
          <label>
            Password
            <input
              type="password"
              autoFocus
              autoComplete={firstRun ? "new-password" : "current-password"}
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          </label>
          {firstRun && (
            <label>
              Confirm password
              <input
                type="password"
                autoComplete="new-password"
                value={confirm}
                onChange={(e) => setConfirm(e.target.value)}
              />
            </label>
          )}
        </div>
        {error && (
          <p role="alert" className="error">
            {error}
          </p>
        )}
        <div className="actions">
          <button className="primary" disabled={busy || !password}>
            {firstRun ? "Set password" : "Log in"}
          </button>
        </div>
        {!firstRun && (
          <p className="help">
            Forgot it? <a href="/wiki.html#password">See how to reset it.</a>
          </p>
        )}
      </form>
    </main>
  );
}
