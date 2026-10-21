import { useEffect, useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";

export type LayoutAction = "new" | "duplicate" | "rename" | "delete" | "notes";
const actions: { action: LayoutAction; label: string }[] = [
  { action: "new", label: "New" }, { action: "duplicate", label: "Duplicate" },
  { action: "rename", label: "Rename" }, { action: "delete", label: "Delete" },
  { action: "notes", label: "Layout notes" },
];

/** A non-modal popup, portaled so the scrolling single-row toolbar cannot clip it. */
export function ChartLayoutSettings({ pending, hasLayout, onAction }: { pending: boolean; hasLayout: boolean; onAction: (action: LayoutAction) => void }) {
  const [open, setOpen] = useState(false);
  const [position, setPosition] = useState({ left: 0, top: 0 });
  const button = useRef<HTMLButtonElement>(null), popup = useRef<HTMLDivElement>(null);
  const popupId = useId();
  useLayoutEffect(() => {
    if (!open) return;
    const place = () => {
      const anchor = button.current!.getBoundingClientRect(), panel = popup.current!.getBoundingClientRect();
      setPosition({ left: Math.max(8, Math.min(anchor.left, window.innerWidth - panel.width - 8)), top: Math.max(8, Math.min(anchor.bottom + 6, window.innerHeight - panel.height - 8)) });
    };
    place(); popup.current!.querySelector<HTMLButtonElement>("button:not(:disabled)")?.focus();
    window.addEventListener("resize", place); window.addEventListener("scroll", place, true);
    return () => { window.removeEventListener("resize", place); window.removeEventListener("scroll", place, true); };
  }, [open]);
  useEffect(() => {
    if (!open) return;
    const outside = (event: Event) => {
      if (event.target instanceof Node && !popup.current?.contains(event.target) && !button.current?.contains(event.target)) setOpen(false);
    };
    const escape = (event: KeyboardEvent) => {
      if (event.key !== "Escape") return;
      event.preventDefault(); event.stopPropagation(); setOpen(false); button.current?.focus();
    };
    document.addEventListener("pointerdown", outside); document.addEventListener("focusin", outside); document.addEventListener("keydown", escape, true);
    return () => { document.removeEventListener("pointerdown", outside); document.removeEventListener("focusin", outside); document.removeEventListener("keydown", escape, true); };
  }, [open]);
  return <>
    <button ref={button} type="button" aria-label="Layout settings" title="Layout settings" aria-haspopup="dialog" aria-expanded={open} aria-controls={open ? popupId : undefined} onClick={() => setOpen(!open)} className="flex size-8 shrink-0 cursor-pointer items-center justify-center rounded-lg border border-zinc-700 text-zinc-300 hover:bg-zinc-800 aria-expanded:bg-zinc-800">
      <svg aria-hidden="true" viewBox="0 0 24 24" fill="none" stroke="currentColor" strokeWidth={1.5} strokeLinecap="round" strokeLinejoin="round" className="size-4">
        <path d="m9 3-.7 2.4-2 .9-2.3-.6-2 3.5 1.6 1.8v2l-1.6 1.8 2 3.5 2.3-.6 2 .9L9 21h6l.7-2.4 2-.9 2.3.6 2-3.5-1.6-1.8v-2L22 9.2l-2-3.5-2.3.6-2-.9L15 3Z" /><circle cx="12" cy="12" r="3" />
      </svg>
    </button>
    {open && createPortal(<div ref={popup} id={popupId} role="dialog" aria-label="Layout settings" className="fixed z-40 max-h-[calc(100dvh-1rem)] w-52 max-w-[calc(100vw-1rem)] overflow-y-auto rounded-xl border border-zinc-700 bg-zinc-950 p-1.5 shadow-2xl" style={position}>
      {actions.map(({ action, label }) => <button key={action} type="button" disabled={pending || (!hasLayout && action !== "new")} aria-label={action === "delete" ? "Delete layout" : undefined} className={`block w-full cursor-pointer rounded-lg px-3 py-2 text-left text-sm hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-40 ${action === "delete" ? "text-red-300" : "text-zinc-200"} ${action === "notes" ? "mt-1 border-t border-zinc-800" : ""}`} onClick={() => { setOpen(false); button.current?.focus(); onAction(action); }}>{label}</button>)}
    </div>, document.body)}
  </>;
}
