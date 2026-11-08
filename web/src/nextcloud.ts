import { useCallback, useRef, useState } from "react";
import { compareVersions, versionParts } from "./release-version";

// Public share: a read-only WebDAV folder on the owner's own Nextcloud,
// reachable cross-origin only because of a Cloudflare Worker added
// specifically for this one path (see H:\Claude Code\ungoggled's project
// notes) — it adds CORS headers to this public share's WebDAV endpoint
// only, nothing else on that Nextcloud.
const SHARE_HOST = "nextcloud.joker1337.de";
const SHARE_TOKEN = "PgZcweFroq4yfKn";
const webdavBase = `https://${SHARE_HOST}/public.php/webdav`;
const authHeader = `Basic ${btoa(`${SHARE_TOKEN}:`)}`;

export type NextcloudRelease = {
  version: string;
  file: string;
  size: number;
  notes: string[];
};

function fileUrl(file: string): string {
  return `${webdavBase}/${file.split("/").map(encodeURIComponent).join("/")}`;
}

/** The owner maintains this file by hand alongside each uploaded build. */
type LatestJson = {
  version: string;
  file: string;
  notes: string[];
};
function parseLatestJson(data: unknown): LatestJson {
  if (
    !data ||
    typeof data !== "object" ||
    typeof (data as Record<string, unknown>).version !== "string" ||
    typeof (data as Record<string, unknown>).file !== "string" ||
    !Array.isArray((data as Record<string, unknown>).notes) ||
    !(data as { notes: unknown[] }).notes.every((n) => typeof n === "string")
  )
    throw new Error("latest.json is malformed");
  return data as LatestJson;
}

/** `null` if there's nothing newer than `current`, or nothing published yet. */
export async function checkLatest(
  current: string,
): Promise<NextcloudRelease | null> {
  if (!versionParts(current)) return null;
  const response = await fetch(`${webdavBase}/latest.json`, {
    headers: { Authorization: authHeader },
    signal: AbortSignal.timeout(10000),
    // Nextcloud's WebDAV response carries no Cache-Control header, so a
    // plain fetch() can apply the browser's own heuristic freshness and
    // keep answering from cache for a while after a release — "check for
    // updates" must always hit the network.
    cache: "no-store",
  });
  if (response.status === 404) return null;
  if (!response.ok)
    throw new Error(`Could not reach the update server (${response.status})`);
  const parsed = parseLatestJson(await response.json());
  if (!versionParts(parsed.version))
    throw new Error("latest.json is malformed");
  if (compareVersions(parsed.version, current) <= 0) return null;
  const head = await fetch(fileUrl(parsed.file), {
    method: "HEAD",
    headers: { Authorization: authHeader },
    signal: AbortSignal.timeout(10000),
  });
  if (!head.ok)
    throw new Error("The update file listed in latest.json is missing");
  const size = Number(head.headers.get("Content-Length"));
  return {
    version: parsed.version,
    file: parsed.file,
    size: Number.isFinite(size) ? size : 0,
    notes: parsed.notes,
  };
}

/** Downloads `release.file`, reporting progress via `onProgress` (0-100). */
export function download(
  release: NextcloudRelease,
  onProgress: (percent: number) => void,
): Promise<File> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest();
    xhr.open("GET", fileUrl(release.file));
    xhr.setRequestHeader("Authorization", authHeader);
    xhr.responseType = "blob";
    xhr.timeout = 10 * 60 * 1000;
    xhr.onprogress = (e) => {
      if (e.lengthComputable)
        onProgress(Math.round((e.loaded / e.total) * 100));
    };
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300)
        resolve(new File([xhr.response as Blob], release.file));
      else reject(new Error(`Download failed (${xhr.status})`));
    };
    xhr.onerror = () => reject(new Error("Download interrupted."));
    xhr.ontimeout = () => reject(new Error("Download timed out."));
    xhr.send();
  });
}

/**
 * Whether *this phone* can reach the update server — not the Pi, which never
 * talks to it (see the top of this file): "offline" here means the phone has
 * no internet right now, typically because it's on the Pi's own Wi-Fi without
 * mobile data. "error" means the server was reached but answered wrongly.
 */
export type Reachability =
  "unchecked" | "checking" | "reachable" | "offline" | "error";
/** A fetch that never got an answer, as opposed to an answer that was bad. */
function networkFailure(e: unknown): boolean {
  return (
    navigator.onLine === false ||
    e instanceof TypeError ||
    (e instanceof DOMException &&
      (e.name === "TimeoutError" || e.name === "AbortError"))
  );
}

export function useNextcloudRelease(current?: string) {
  const [latest, setLatest] = useState<NextcloudRelease | null>(null);
  const [checking, setChecking] = useState(false);
  const [checked, setChecked] = useState(false);
  const [error, setError] = useState("");
  const [reach, setReach] = useState<Reachability>("unchecked");
  const [checkedAt, setCheckedAt] = useState<number | null>(null);
  const running = useRef(false);
  const check = useCallback(async () => {
    if (!current || running.current) return;
    running.current = true;
    setChecking(true);
    setReach("checking");
    setError("");
    try {
      setLatest(await checkLatest(current));
      setReach("reachable");
    } catch (e) {
      if (networkFailure(e)) setReach("offline");
      else {
        setReach("error");
        setError(
          e instanceof Error ? e.message : "Could not check for updates.",
        );
      }
    } finally {
      running.current = false;
      setChecking(false);
      setChecked(true);
      setCheckedAt(Date.now());
    }
  }, [current]);
  return { latest, checking, checked, error, check, reach, checkedAt };
}
