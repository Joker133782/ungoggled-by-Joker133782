import { useSyncExternalStore } from "react";

/** The four sections of the control interface, in tab-bar order. */
export const TABS = ["live", "videos", "settings", "system"] as const;
export type Tab = (typeof TABS)[number];

function subscribe(onChange: () => void) {
  window.addEventListener("hashchange", onChange);
  return () => window.removeEventListener("hashchange", onChange);
}

/**
 * The current section, read from the URL hash: "#videos" → { tab: "videos" },
 * "#system/software-update" → { tab: "system", sub: "software-update" }.
 * Anything unknown (including no hash at all) is the Live tab.
 *
 * Hash links rather than path routes on purpose: the Pi's static file server
 * has no fallback to index.html, so a reload on "/settings" would 404, while
 * "/#settings" always loads the app. Hash links also keep working inside the
 * installed (home screen) app, and every tab switch is a history entry, so
 * the phone's back gesture steps back through the tabs.
 */
export function useRoute(): { tab: Tab; sub: string | null } {
  const hash = useSyncExternalStore(subscribe, () => window.location.hash);
  const [first, ...rest] = hash.replace(/^#\/?/, "").split("/");
  const tab = (TABS as readonly string[]).includes(first)
    ? (first as Tab)
    : "live";
  return { tab, sub: rest.join("/") || null };
}
