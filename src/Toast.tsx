import { useEffect } from "react";

export type ToastData = { id: number; text: string; detail?: string; kind: "ok" | "error"; action?: { label: string; run: () => void } };

/** A small "Loki finished" card, bottom right; dismisses itself. */
export default function Toast({ toast, onClose }: { toast: ToastData | null; onClose: () => void }) {
  useEffect(() => {
    if (!toast) return;
    const t = window.setTimeout(onClose, 14000);
    return () => window.clearTimeout(t);
  }, [toast, onClose]);
  if (!toast) return null;
  return (
    <div className={`loki-toast loki-toast-${toast.kind}`} role="status">
      <div className="loki-toast-text">
        <b>{toast.text}</b>
        {toast.detail && <span>{toast.detail}</span>}
      </div>
      {toast.action && (
        <button type="button" onClick={() => { toast.action?.run(); onClose(); }}>{toast.action.label}</button>
      )}
      <button type="button" className="loki-toast-x" onClick={onClose} aria-label="Dismiss">✕</button>
    </div>
  );
}
