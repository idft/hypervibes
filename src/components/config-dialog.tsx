import { useEffect, useId, useRef, type ReactNode } from "react";
import { createPortal } from "react-dom";

export const configInput = "block w-full rounded-xl border border-zinc-700 bg-zinc-950 p-3 text-sm text-white";
export const configButton = "cursor-pointer rounded-full border border-zinc-700 px-4 py-2 text-sm text-zinc-200 hover:bg-zinc-800 disabled:cursor-not-allowed disabled:opacity-50";

export function ConfigDialog({ title, onCancel, onApply, children, applyDisabled = false, applyLabel = "Apply" }: { title: string; onCancel: () => void; onApply: () => void; children: ReactNode; applyDisabled?: boolean; applyLabel?: string }) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const dialog = ref.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  return createPortal(<dialog ref={ref} aria-labelledby={titleId} onCancel={(event) => { event.preventDefault(); onCancel(); }} className="m-auto max-h-[calc(100dvh-2rem)] w-[calc(100%_-_2rem)] max-w-3xl overflow-y-auto rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-200 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <form onSubmit={(event) => { event.preventDefault(); event.stopPropagation(); onApply(); }} className="space-y-5">
      <h3 id={titleId} className="text-lg font-semibold text-white">{title}</h3>
      {children}
      <footer className="flex justify-end gap-3 border-t border-zinc-800 pt-4">
        <button type="button" onClick={onCancel} className={configButton}>Cancel</button>
        <button disabled={applyDisabled} className={`${configButton} bg-violet-600 hover:bg-violet-500`}>{applyLabel}</button>
      </footer>
    </form>
  </dialog>, document.body);
}
