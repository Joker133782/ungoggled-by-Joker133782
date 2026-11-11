import { useEffect, useState } from "react";
export const headers = {
  "X-DJI-Control": "1",
  "Content-Type": "application/json",
};
export async function api<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(`/api/${path}`, init);
  if (!response.ok) {
    const body = await response.json().catch(() => ({}));
    throw new Error(body.error || `Request failed (${response.status})`);
  }
  return response.json() as Promise<T>;
}
/**
 * A signal that aborts after `ms` or when `parent` aborts, whichever comes
 * first. Stands in for `AbortSignal.any`/`AbortSignal.timeout`, which older
 * phones (iOS before 17.4) don't have. Call `done()` once the request has
 * finished so the timer doesn't linger.
 */
export function withTimeout(parent: AbortSignal | null, ms: number) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), ms);
  const onAbort = () => controller.abort();
  if (parent?.aborted) controller.abort();
  else parent?.addEventListener("abort", onAbort, { once: true });
  return {
    signal: controller.signal,
    done: () => {
      clearTimeout(timer);
      parent?.removeEventListener("abort", onAbort);
    },
  };
}
/**
 * Fetches `path` now and every `ms` after that. With `enabled` false it
 * stops polling (keeping the last value), so a tab that isn't on screen
 * doesn't keep spending the Pi's Wi-Fi on data nobody is looking at.
 */
export function usePolling<T>(path: string, ms: number, enabled = true) {
  const [value, setValue] = useState<T | null>(null);
  const [error, setError] = useState("");
  useEffect(() => {
    if (!enabled) return;
    const abort = new AbortController();
    let timer: ReturnType<typeof setTimeout>;
    const poll = async () => {
      const request = withTimeout(abort.signal, 5000);
      try {
        const data = await api<T>(path, { signal: request.signal });
        if (!abort.signal.aborted) {
          setValue(data);
          setError("");
        }
      } catch (e) {
        if (!abort.signal.aborted) setError(String(e));
      } finally {
        request.done();
      }
      if (!abort.signal.aborted) timer = setTimeout(poll, ms);
    };
    void poll();
    return () => {
      abort.abort();
      clearTimeout(timer);
    };
  }, [path, ms, enabled]);
  return { value, error };
}
