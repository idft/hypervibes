import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

export const configInput = "block w-full rounded-xl border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
export const configButton = "cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-200 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";

export function ConfigDialog({ title, onCancel, onApply, children, applyDisabled = false, applyLabel = "Apply" }: { title: string; onCancel: () => void; onApply: () => void; children: ReactNode; applyDisabled?: boolean; applyLabel?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const formId = useId();
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  // Only the content area scrolls; the title and footer stay pinned. Callers
  // with long content should give their scroller `min-h-0` and internal scrolling.
  return createPortal(<dialog ref={ref} aria-labelledby={titleId} onCancel={(event) => { event.preventDefault(); onCancel(); }} className="m-auto flex max-h-[calc(100dvh-2rem)] w-[calc(100%_-_2rem)] max-w-3xl flex-col rounded-2xl border border-zinc-800 bg-[#0d0d10] text-zinc-200 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <form id={formId} onSubmit={(event) => { event.preventDefault(); event.stopPropagation(); onApply(); }} className="flex min-h-0 flex-col gap-5 overflow-y-auto px-6 pt-6">
      <h3 id={titleId} className="shrink-0 text-lg font-semibold text-white">{title}</h3>
      {children}
    </form>
    <footer className="flex shrink-0 justify-end gap-3 border-t border-zinc-800 p-6 pt-4">
      <button type="button" onClick={onCancel} className={configButton}>Cancel</button>
      <button type="submit" form={formId} disabled={applyDisabled} className={`${configButton} bg-violet-600 hover:bg-violet-500`}>{applyLabel}</button>
    </footer>
  </dialog>, document.body);
}