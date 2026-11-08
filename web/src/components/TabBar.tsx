import type { ReactNode } from "react";
import type { Tab } from "../route";
import { IconChip, IconSettings, IconStream, IconVideos } from "./icons";

const ITEMS: { id: Tab; label: string; icon: ReactNode }[] = [
  { id: "live", label: "Live", icon: <IconStream /> },
  { id: "videos", label: "Videos", icon: <IconVideos /> },
  { id: "settings", label: "Settings", icon: <IconSettings /> },
  { id: "system", label: "System", icon: <IconChip /> },
];

/**
 * Switches between the four sections. App renders this twice: once inside
 * the sticky header (shown on wide screens) and once fixed to the bottom of
 * the screen (shown on phones, where it sits under the thumb); CSS shows
 * exactly one of them. Plain "#tab" links, so the browser's back button or
 * gesture steps back through the tabs visited.
 */
export function TabBar({
  current,
  placement,
  badges = {},
}: {
  current: Tab;
  placement: "top" | "bottom";
  /** Tabs that get a small dot, e.g. System while an update is available. */
  badges?: Partial<Record<Tab, string>>;
}) {
  return (
    <nav className={`tabbar tabbar-${placement}`} aria-label="Sections">
      {ITEMS.map((item) => (
        <a
          key={item.id}
          href={`#${item.id}`}
          className={item.id === current ? "active" : undefined}
          aria-current={item.id === current ? "page" : undefined}
        >
          <span className="tab-icon">
            {item.icon}
            {badges[item.id] && <i className="tab-badge" aria-hidden="true" />}
          </span>
          <span className="tab-label">
            {item.label}
            {badges[item.id] && (
              <span className="sr-only"> ({badges[item.id]})</span>
            )}
          </span>
        </a>
      ))}
    </nav>
  );
}
