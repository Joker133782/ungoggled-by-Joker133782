import { useCallback, useEffect, useState } from "react";
import { newestRelease, type Release } from "./release-version";
import { withTimeout } from "./api";
// Disabled on purpose: this must never check (or install from) the
// original TimoLehnertz/ungoggled repository's releases — doing so would
// silently overwrite every change made in this fork. The active, working
// update check is Nextcloud-based; see nextcloud.ts. Leave this code path in
// place but pointed at nothing until this fork publishes releases of its own
// and the endpoint is set to those.
const endpoint = "";
const cacheKey = "ungoggled-github-releases-v1";
const hour = 60 * 60 * 1000;
let inFlight: Promise<unknown> | null = null;
async function releases(force: boolean): Promise<unknown> {
  if (!endpoint) return [];
  if (!force) {
    try {
      const cached = JSON.parse(localStorage.getItem(cacheKey) || "null");
      if (
        cached &&
        Date.now() - cached.time >= 0 &&
        Date.now() - cached.time < hour
      )
        return cached.data;
    } catch {
      /* Storage may be disabled. */
    }
  }
  if (!inFlight)
    inFlight = (async () => {
      const request = withTimeout(null, 10000);
      let data: unknown;
      try {
        const response = await fetch(endpoint, {
          headers: { Accept: "application/vnd.github+json" },
          signal: request.signal,
        });
        if (!response.ok)
          throw new Error(
            response.status === 403 || response.status === 429
              ? "GitHub rate limit reached. Try later."
              : "Could not check GitHub releases.",
          );
        data = await response.json();
      } finally {
        request.done();
      }
      if (!Array.isArray(data)) throw new Error("Invalid GitHub response");
      try {
        localStorage.setItem(
          cacheKey,
          JSON.stringify({ time: Date.now(), data }),
        );
      } catch {
        /* Optional cache. */
      }
      return data;
    })().finally(() => {
      inFlight = null;
    });
  return inFlight;
}
export function useReleases(current?: string) {
  const [latest, setLatest] = useState<Release | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState("");
  const [checked, setChecked] = useState(false);
  const check = useCallback(
    async (force = false) => {
      if (!current) return;
      setChecking(true);
      setError("");
      try {
        setLatest(newestRelease(await releases(force), current));
        setChecked(true);
      } catch (e) {
        setError(
          e instanceof Error ? e.message : "Could not check GitHub releases.",
        );
      } finally {
        setChecking(false);
      }
    },
    [current],
  );
  useEffect(() => {
    void check();
    const timer = setInterval(() => void check(), hour);
    return () => clearInterval(timer);
  }, [check]);
  return { latest, checking, error, checked, check };
}
