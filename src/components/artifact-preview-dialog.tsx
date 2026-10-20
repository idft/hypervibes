import { useEffect, useId, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { changeArtifact, readArtifact } from "../web/artifacts.js";
import { ArtifactContent, artifactContentUrl } from "./artifact-content.js";

type Preview = Extract<Awaited<ReturnType<typeof readArtifact>>, { ok: true }>["data"];
const button = "cursor-pointer rounded-lg border border-zinc-700 px-3 py-2 text-sm text-zinc-200 hover:bg-zinc-800 disabled:cursor-wait disabled:opacity-50";

export function ArtifactPreviewDialog({ agentId, artifactId, onClose }: { agentId: string; artifactId: string; onClose: () => void }) {
  const dialogRef = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  const [location, setLocation] = useState({ id: artifactId, offset: 0 });
  const [preview, setPreview] = useState<Preview | null>(null);
  const [error, setError] = useState("");
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const mutationInFlight = useRef(false);
  function close() {
    if (!mutationInFlight.current) onClose();
  }
  async function change(action: "delete" | "retain" | "expire") {
    if (!preview || mutationInFlight.current || loading) return;
    if (action === "delete" && !window.confirm("Permanently delete this artifact? Referenced attachments cannot be deleted until their report is removed.")) return;
    mutationInFlight.current = true; setSaving(true); setError("");
    try {
      const result = await changeArtifact({ data: { agentId, id: preview.id, action } });
      if (!result.ok) throw new Error(result.error);
      if (action === "delete") {
        if (preview.id === artifactId) onClose();
        else setLocation({ id: artifactId, offset: 0 });
      } else setLocation({ ...location });
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : "Unable to update artifact");
    } finally {
      mutationInFlight.current = false; setSaving(false);
    }
  }
  useEffect(() => {
    const dialog = dialogRef.current!;
    dialog.showModal();
    return () => dialog.close();
  }, []);
  useEffect(() => {
    let cancelled = false;
    setLoading(true); setError(""); setPreview(null);
    void (async () => {
      try {
        const result = await readArtifact({ data: { agentId, ...location } });
        if (!result.ok) throw new Error(result.error);
        if (!cancelled) setPreview(result.data);
      } catch (cause) {
        if (!cancelled) setError(cause instanceof Error ? cause.message : "Unable to load artifact");
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => { cancelled = true; };
  }, [agentId, location]);
  return createPortal(<dialog ref={dialogRef} aria-labelledby={titleId} onCancel={(event) => { event.preventDefault(); close(); }} className="m-auto flex max-h-[calc(100dvh-2rem)] w-[calc(100%_-_2rem)] max-w-5xl flex-col rounded-2xl border border-zinc-800 bg-[#0d0d10] text-zinc-200 shadow-2xl backdrop:bg-black/70 backdrop:backdrop-blur-sm">
    <header className="flex shrink-0 items-center justify-between gap-4 border-b border-zinc-800 p-5"><h2 id={titleId} className="text-lg font-medium text-white">{preview?.title ?? "Supporting artifact"}</h2><button type="button" className={button} disabled={saving} onClick={close}>Close</button></header>
    <div className="min-h-0 space-y-4 overflow-y-auto p-5" aria-busy={loading || saving}>
      {loading && <p role="status">Loading artifact…</p>}
      {error && <p role="alert" className="text-red-300">{error}</p>}
      {preview && <>
        {preview.description && <p className="text-sm text-zinc-400">{preview.description}</p>}
        {preview.mediaType.startsWith("image/") ? <img className="max-h-[65vh] max-w-full object-contain" src={artifactContentUrl(agentId, preview.id, true)} alt={preview.title} /> : <ArtifactContent agentId={agentId} content={preview.content ?? ""} attachments={preview.attachments} />}
        {location.offset > 0 && <button type="button" className={button} disabled={saving} onClick={() => setLocation({ id: preview.id, offset: 0 })}>First text portion</button>}
        {preview.nextOffset !== null && <button type="button" className={button} disabled={saving} onClick={() => setLocation({ id: preview.id, offset: preview.nextOffset! })}>Next text portion</button>}
        {preview.attachments.length > 0 && <section className="space-y-2"><h3 className="text-sm font-medium">Attachments</h3><div className="flex flex-wrap gap-2">{preview.attachments.map((item) => <button type="button" className={button} disabled={saving} key={item.id} onClick={() => setLocation({ id: item.id, offset: 0 })}>{item.title}</button>)}</div></section>}
      </>}
    </div>
    <footer className="flex shrink-0 flex-wrap gap-3 border-t border-zinc-800 p-5">
      <a className={button} href={artifactContentUrl(agentId, location.id)}>Download</a>
      {preview && <>
        <button type="button" disabled={loading || saving || (preview.protectedByMemory && preview.expiresAt === null)} className={button} onClick={() => { void change(preview.expiresAt === null ? "expire" : "retain"); }}>{preview.expiresAt === null ? "Expire in seven days" : "Keep / Pin"}</button>
        <button type="button" disabled={loading || saving || preview.protectedByMemory} title={preview.protectedByMemory ? "Evidence referenced by memory cannot be deleted" : undefined} className={`${button} text-red-300`} onClick={() => { void change("delete"); }}>Delete</button>
      </>}
      {location.id !== artifactId && <button type="button" className={button} disabled={loading || saving} onClick={() => setLocation({ id: artifactId, offset: 0 })}>Back to supporting artifact</button>}
    </footer>
  </dialog>, document.body);
}
