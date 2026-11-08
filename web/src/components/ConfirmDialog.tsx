import { useEffect, useRef } from "react";
import { IconCheck, IconClose } from "./icons";
export function ConfirmDialog({
  open,
  title,
  body,
  confirmLabel,
  busy,
  onConfirm,
  onCancel,
}: {
  open: boolean;
  title: string;
  body: string;
  confirmLabel: string;
  busy: boolean;
  onConfirm: () => void;
  onCancel: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    if (!dialog) return;
    if (open && !dialog.open) dialog.showModal();
    if (!open && dialog.open) dialog.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      className="confirm"
      onCancel={(e) => {
        e.preventDefault();
        if (!busy) onCancel();
      }}
    >
      <h3>{title}</h3>
      <p>{body}</p>
      <div className="actions">
        <button disabled={busy} onClick={onCancel}>
          <IconClose /> Cancel
        </button>
        <button className="primary" disabled={busy} onClick={onConfirm}>
          <IconCheck /> {busy ? "Working…" : confirmLabel}
        </button>
      </div>
    </dialog>
  );
}
