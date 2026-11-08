import type { ReactNode } from "react";

/**
 * Shared panel header: an icon badge, the title, an optional "experimental"
 * tag, optional trailing controls (e.g. a toggle), and a "?" link into the
 * in-app wiki for anyone who wants the long explanation.
 */
export function PanelHead({
  icon,
  title,
  experimental,
  wiki,
  children,
}: {
  icon: ReactNode;
  title: string;
  experimental?: boolean;
  wiki: string;
  children?: ReactNode;
}) {
  return (
    <div className="panel-head">
      <span className="icon-badge">{icon}</span>
      <h2>{title}</h2>
      {experimental && <span className="tag">EXPERIMENTAL</span>}
      {children}
      <a
        className="help-link"
        href={`/wiki.html#${wiki}`}
        target="_blank"
        rel="noreferrer"
        aria-label={`Help: ${title}`}
        title={`Help: ${title}`}
      >
        ?
      </a>
    </div>
  );
}
