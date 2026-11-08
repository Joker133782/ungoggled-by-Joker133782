import { useEffect, useRef, useState } from "react";
export function Preview({
  enabled,
  fallback,
  state,
}: {
  enabled: boolean;
  fallback?: string;
  state: string;
}) {
  const [src, setSrc] = useState<string | null>(null);
  const current = useRef<string | null>(null);
  useEffect(() => {
    setSrc(null);
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    let disposed = false;
    async function poll() {
      try {
        const response = await fetch("/api/preview.jpg", {
          signal: abort.signal,
          cache: "no-store",
        });
        if (response.ok && response.status !== 204) {
          const blob = await response.blob();
          if (!disposed) {
            const next = URL.createObjectURL(blob);
            const old = current.current;
            current.current = next;
            setSrc(next);
            if (old) URL.revokeObjectURL(old);
          }
        } else if (!disposed) setSrc(null);
      } catch {
        /* Status polling reports network errors. */
      }
      if (!disposed) timer = setTimeout(poll, 200);
    }
    if (enabled) void poll();
    return () => {
      disposed = true;
      abort.abort();
      clearTimeout(timer);
      if (current.current) URL.revokeObjectURL(current.current);
      current.current = null;
    };
  }, [enabled]);
  return (
    <div className="video">
      {enabled && src ? (
        <img src={src} alt="Live camera preview" />
      ) : state === "fallback" && fallback ? (
        <img src={fallback} alt="Idle screen" />
      ) : (
        <span>
          {enabled
            ? "Loading preview…"
            : state === "fallback"
              ? "No signal"
              : state === "live"
                ? "Preview disabled"
                : "No camera video"}
        </span>
      )}
    </div>
  );
}
