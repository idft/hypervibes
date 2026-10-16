import { useEffect, useId, useRef } from "react";

export function ArchiveSubagentDialog({ name, onCancel, onConfirm }: { name: string; onCancel: () => void; onConfirm: () => void }) {
  const dialog = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  useEffect(() => {
    const element = dialog.current!;
    element.showModal();
    return () => element.close();
  }, []);
  return <dialog ref={dialog} aria-labelledby={titleId} aria-describedby={`${titleId}-description`} onCancel={(event) => { event.preventDefault(); onCancel(); }} className="m-auto w-[calc(100%_-_2rem)] max-w-md rounded-2xl border border-zinc-800 bg-[#0d0d10] p-6 text-zinc-100 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <h3 id={titleId} className="text-lg font-semibold text-white">Archive sub-agent?</h3>
    <p id={`${titleId}-description`} className="mt-2 text-sm leading-6 text-zinc-400">Archive “{name}”? New assignments will be disabled. Its configuration and run history will be preserved.</p>
    <div className="mt-6 flex flex-col-reverse gap-3 sm:flex-row sm:justify-end">
      <button type="button" autoFocus onClick={onCancel} className="cursor-pointer rounded-full border border-zinc-800 px-4 py-2 text-sm font-medium text-zinc-300 hover:border-zinc-700 hover:text-white">Cancel</button>
      <button type="button" onClick={onConfirm} className="cursor-pointer rounded-full bg-red-600 px-4 py-2 text-sm font-medium text-white hover:bg-red-500">Archive sub-agent</button>
    </div>
  </dialog>;
}
