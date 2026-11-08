import { useEffect, useRef, useState, type SyntheticEvent } from "react";
import { Terminal } from "@xterm/xterm";
import { FitAddon } from "@xterm/addon-fit";
import "@xterm/xterm/css/xterm.css";
import { IconClose, IconTerminal } from "./icons";

/** Why a terminal ended, from the Pi's `closed` message (or this page). */
const REASONS: Record<string, string> = {
  idle: "Closed after {idle} without typing.",
  limit: "Closed: the {max} limit was reached.",
  disabled: "Closed: the terminal was switched off in Settings.",
  "signed-out": "Closed: this browser was logged out.",
  shutdown: "Closed: the Pi's software is restarting.",
  exited: "The shell has exited.",
  lost: "Closed: the connection to the Pi was lost.",
  closed: "Closed.",
  error: "The Pi couldn't start a shell.",
  stuck: "Closed: the shell stopped taking input.",
  hidden: "Closed because this page was in the background for a minute.",
  refused:
    "The Pi refused the terminal: it may already be open somewhere else. Open it again to retry.",
};
/** Read-only, so they can run straight away from a tap. */
const QUICK = [
  { label: "Service log", command: "journalctl -u dji-hdmi -n 40 --no-pager" },
  { label: "Service status", command: "systemctl status dji-hdmi --no-pager" },
  { label: "Disk space", command: "df -h" },
  { label: "Memory", command: "free -h" },
  { label: "Temperature", command: "vcgencmd measure_temp" },
  { label: "Network", command: "ip -br addr" },
];
/** Keys a phone keyboard lacks; arrows follow the shell's cursor-key mode. */
const KEYS: { label: string; name: string; send: (app: boolean) => string }[] =
  [
    { label: "Esc", name: "Escape", send: () => "\x1b" },
    { label: "Tab", name: "Tab", send: () => "\t" },
    { label: "↑", name: "Up", send: (app) => (app ? "\x1bOA" : "\x1b[A") },
    { label: "↓", name: "Down", send: (app) => (app ? "\x1bOB" : "\x1b[B") },
    { label: "←", name: "Left", send: (app) => (app ? "\x1bOD" : "\x1b[D") },
    { label: "→", name: "Right", send: (app) => (app ? "\x1bOC" : "\x1b[C") },
    { label: "|", name: "Pipe", send: () => "|" },
    { label: "/", name: "Slash", send: () => "/" },
    { label: "-", name: "Dash", send: () => "-" },
    { label: "~", name: "Tilde", send: () => "~" },
  ];
const FONT_KEY = "ungoggled-terminal-font";
function initialFont(): number {
  try {
    const saved = Number(localStorage.getItem(FONT_KEY));
    if (saved >= 9 && saved <= 24) return saved;
  } catch {
    /* Not stored here; use the default. */
  }
  return window.innerWidth < 500 ? 12 : 14;
}
const clock = (seconds: number) => {
  const s = Math.max(0, Math.ceil(seconds));
  return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, "0")}`;
};
const duration = (seconds: number) =>
  seconds % 60 === 0 ? `${seconds / 60} minutes` : `${seconds} seconds`;
// The extra keys must not take the focus (and with it the phone keyboard)
// away from the terminal.
const keepFocus = {
  onMouseDown: (e: SyntheticEvent) => e.preventDefault(),
  onPointerDown: (e: SyntheticEvent) => e.preventDefault(),
};

/**
 * The open terminal, full screen: xterm.js on a WebSocket to the Pi's pty.
 * The limits shown here are the Pi's (sent in its `hello`); the Pi enforces
 * them, this only counts down. Loaded on demand (see TerminalPanel).
 */
export default function TerminalScreen({
  ticket,
  onClose,
}: {
  ticket: string;
  onClose: () => void;
}) {
  const root = useRef<HTMLDivElement>(null);
  const host = useRef<HTMLDivElement>(null);
  const term = useRef<Terminal | null>(null);
  const fit = useRef<FitAddon | null>(null);
  const socket = useRef<WebSocket | null>(null);
  const ctrl = useRef(false);
  const [ctrlOn, setCtrlOn] = useState(false);
  const [open, setOpen] = useState(false);
  const [reason, setReason] = useState<string | null>(null);
  const reasonRef = useRef<string | null>(null);
  const [limits, setLimits] = useState<{ idle: number; max: number } | null>(
    null,
  );
  const openedAt = useRef(0);
  const lastInput = useRef(0);
  const [, setTick] = useState(0);
  const [font, setFont] = useState(initialFont);

  function finish(why: string) {
    if (reasonRef.current) return;
    reasonRef.current = why;
    setReason(why);
    if (term.current) term.current.options.disableStdin = true;
  }
  function send(text: string | Uint8Array) {
    const ws = socket.current;
    if (ws?.readyState !== WebSocket.OPEN) return;
    ws.send(typeof text === "string" ? new TextEncoder().encode(text) : text);
    lastInput.current = Date.now();
  }
  function end() {
    finish("closed");
    socket.current?.close(1000);
    onClose();
  }

  useEffect(() => {
    const t = new Terminal({
      fontSize: font,
      fontFamily: '"JetBrains Mono", ui-monospace, monospace',
      cursorBlink: true,
      scrollback: 3000,
      theme: {
        background: "#0c0f14",
        foreground: "#e6ebf2",
        cursor: "#4fd1c5",
        selectionBackground: "#4fd1c555",
      },
    });
    const f = new FitAddon();
    t.loadAddon(f);
    t.open(host.current!);
    term.current = t;
    fit.current = f;
    const refit = () => {
      try {
        f.fit();
      } catch {
        /* Not laid out yet. */
      }
    };
    refit();
    let ws: WebSocket | null = null;
    const sendSize = () => {
      if (ws?.readyState === WebSocket.OPEN)
        ws.send(JSON.stringify({ t: "resize", rows: t.rows, cols: t.cols }));
    };
    // One tick later: React's development double-mount would otherwise
    // spend the single-use ticket on a connection it closes right away.
    const start = setTimeout(() => {
      const scheme = window.location.protocol === "https:" ? "wss" : "ws";
      ws = new WebSocket(
        `${scheme}://${window.location.host}/api/terminal/ws?ticket=${encodeURIComponent(ticket)}`,
      );
      ws.binaryType = "arraybuffer";
      socket.current = ws;
      let wasOpen = false;
      ws.onopen = () => {
        wasOpen = true;
        setOpen(true);
        sendSize();
        t.focus();
      };
      ws.onmessage = (e) => {
        if (typeof e.data !== "string") {
          t.write(new Uint8Array(e.data as ArrayBuffer));
          return;
        }
        try {
          const msg = JSON.parse(e.data);
          if (msg.t === "hello") {
            openedAt.current = lastInput.current = Date.now();
            setLimits({ idle: msg.idle_seconds, max: msg.max_seconds });
          } else if (msg.t === "closed") finish(String(msg.reason));
        } catch {
          /* Not a message this page knows. */
        }
      };
      ws.onclose = () => {
        setOpen(false);
        finish(wasOpen ? "lost" : "refused");
      };
    }, 0);
    const typed = t.onData((data) => {
      let out = data;
      if (ctrl.current && data.length === 1) {
        const code = data.toUpperCase().charCodeAt(0);
        if (code >= 64 && code <= 95) out = String.fromCharCode(code - 64);
        ctrl.current = false;
        setCtrlOn(false);
      }
      send(out);
    });
    // Mouse reports some full-screen programs ask for.
    const binary = t.onBinary((data) =>
      send(Uint8Array.from(data, (c) => c.charCodeAt(0) & 0xff)),
    );
    const resized = t.onResize(sendSize);
    const observer = new ResizeObserver(refit);
    observer.observe(host.current!);
    return () => {
      clearTimeout(start);
      observer.disconnect();
      typed.dispose();
      binary.dispose();
      resized.dispose();
      ws?.close(1000);
      t.dispose();
      term.current = null;
    };
    // Set up once per ticket; the font size is applied separately below.
  }, [ticket]);

  useEffect(() => {
    if (term.current) term.current.options.fontSize = font;
    try {
      fit.current?.fit();
    } catch {
      /* Not laid out yet. */
    }
    try {
      localStorage.setItem(FONT_KEY, String(font));
    } catch {
      /* Not remembered then. */
    }
  }, [font]);

  // Countdowns tick once a second.
  useEffect(() => {
    const timer = setInterval(() => setTick((n) => n + 1), 1000);
    return () => clearInterval(timer);
  }, []);

  // A phone that's put away mustn't keep the only terminal slot (or a root
  // shell) open: a minute in the background ends it. The Pi also notices on
  // its own once the browser stops answering.
  useEffect(() => {
    let timer: ReturnType<typeof setTimeout> | undefined;
    const onVisibility = () => {
      clearTimeout(timer);
      if (document.hidden)
        timer = setTimeout(() => {
          finish("hidden");
          socket.current?.close(1000);
        }, 60_000);
    };
    document.addEventListener("visibilitychange", onVisibility);
    return () => {
      clearTimeout(timer);
      document.removeEventListener("visibilitychange", onVisibility);
    };
  }, []);

  // Fill the part of the screen the phone keyboard leaves visible, and keep
  // the page behind from scrolling.
  useEffect(() => {
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    const vv = window.visualViewport;
    const el = root.current;
    const apply = () => {
      if (!vv || !el) return;
      el.style.height = `${vv.height}px`;
      el.style.transform = `translateY(${vv.offsetTop}px)`;
    };
    apply();
    vv?.addEventListener("resize", apply);
    vv?.addEventListener("scroll", apply);
    return () => {
      document.body.style.overflow = overflow;
      vv?.removeEventListener("resize", apply);
      vv?.removeEventListener("scroll", apply);
    };
  }, []);

  const now = Date.now();
  const left = limits ? limits.max - (now - openedAt.current) / 1000 : null;
  const idleLeft = limits
    ? limits.idle - (now - lastInput.current) / 1000
    : null;
  const closedText = reason
    ? (REASONS[reason] ?? `Closed (${reason}).`)
        .replace("{idle}", limits ? duration(limits.idle) : "a while")
        .replace("{max}", limits ? duration(limits.max) : "time")
    : "";
  return (
    <div
      className="terminal-screen"
      ref={root}
      role="dialog"
      aria-modal="true"
      aria-label="Root shell on the Pi"
    >
      <div className="terminal-bar">
        <span className="terminal-title">
          <IconTerminal /> Root shell
        </span>
        <span
          className={`terminal-timer ${idleLeft != null && idleLeft < 60 ? "soon" : ""}`}
          aria-live="off"
        >
          {reason
            ? "Closed"
            : !open || left == null || idleLeft == null
              ? "Connecting…"
              : idleLeft < left
                ? `Closes in ${clock(idleLeft)} without typing`
                : `Closes in ${clock(left)}`}
        </span>
        <button onClick={end}>
          <IconClose /> End
        </button>
      </div>
      <p className="terminal-warning">
        You are root on the Pi: commands run immediately, and the wrong one can
        break the installation.
      </p>
      <div className="terminal-host" ref={host} />
      {reason ? (
        <div className="terminal-closed" role="status">
          <span>{closedText}</span>
          <button className="primary" onClick={onClose}>
            <IconClose /> Close
          </button>
        </div>
      ) : (
        <>
          <div className="terminal-quick" aria-label="Quick commands">
            {QUICK.map((q) => (
              <button
                key={q.label}
                title={q.command}
                disabled={!open}
                {...keepFocus}
                onClick={() => {
                  send(`${q.command}\r`);
                  term.current?.focus();
                }}
              >
                {q.label}
              </button>
            ))}
          </div>
          <div className="terminal-keys" aria-label="Extra keys">
            <button
              className={ctrlOn ? "on" : ""}
              aria-pressed={ctrlOn}
              disabled={!open}
              {...keepFocus}
              onClick={() => {
                ctrl.current = !ctrl.current;
                setCtrlOn(ctrl.current);
                term.current?.focus();
              }}
            >
              Ctrl
            </button>
            {KEYS.map((k) => (
              <button
                key={k.name}
                aria-label={k.name}
                disabled={!open}
                {...keepFocus}
                onClick={() => {
                  send(
                    k.send(
                      term.current?.modes.applicationCursorKeysMode ?? false,
                    ),
                  );
                  term.current?.focus();
                }}
              >
                {k.label}
              </button>
            ))}
            <button
              aria-label="Smaller text"
              {...keepFocus}
              onClick={() => setFont((f) => Math.max(9, f - 1))}
            >
              A−
            </button>
            <button
              aria-label="Larger text"
              {...keepFocus}
              onClick={() => setFont((f) => Math.min(24, f + 1))}
            >
              A+
            </button>
          </div>
        </>
      )}
    </div>
  );
}
