import { useSyncExternalStore } from "react";

/** The groups of graphs on the System tab, each switchable in Settings. */
export const GRAPH_GROUPS = [
  { id: "video", title: "Video", detail: "Bitrate and frame rate" },
  { id: "health", title: "Pi health", detail: "Temperature, CPU and RAM" },
  { id: "network", title: "Network", detail: "Wi-Fi and Ethernet traffic" },
  { id: "sd", title: "SD card", detail: "How much is read and written" },
] as const;
export type GraphGroup = (typeof GRAPH_GROUPS)[number]["id"];
export type GraphGroups = Record<GraphGroup, boolean>;

const KEY = "ungoggled-graph-groups";
const DEFAULTS: GraphGroups = {
  video: true,
  health: true,
  network: true,
  sd: false,
};
function load(): GraphGroups {
  try {
    const saved = JSON.parse(localStorage.getItem(KEY) ?? "{}");
    const groups = { ...DEFAULTS };
    for (const { id } of GRAPH_GROUPS)
      if (typeof saved?.[id] === "boolean") groups[id] = saved[id];
    return groups;
  } catch {
    return { ...DEFAULTS };
  }
}
let current = load();
const listeners = new Set<() => void>();
function subscribe(onChange: () => void) {
  listeners.add(onChange);
  return () => listeners.delete(onChange);
}

/**
 * Which graph groups to show. A viewing preference of this one device (a
 * phone might want fewer graphs than a laptop), so it lives in the browser
 * rather than in the Pi's settings, and falls back to the defaults wherever
 * the browser won't store anything.
 */
export function useGraphGroups(): [
  GraphGroups,
  (id: GraphGroup, on: boolean) => void,
] {
  const groups = useSyncExternalStore(subscribe, () => current);
  function set(id: GraphGroup, on: boolean) {
    current = { ...current, [id]: on };
    try {
      localStorage.setItem(KEY, JSON.stringify(current));
    } catch {
      /* Kept for this visit only. */
    }
    listeners.forEach((l) => l());
  }
  return [groups, set];
}
