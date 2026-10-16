import { useRef, useState } from "react";
import { ConfigDialog, configInput } from "./config-dialog.js";

export function RunSubagentDialog({ task, onTaskChange, onCancel, onRun }: { task: string; onTaskChange: (task: string) => void; onCancel: () => void; onRun: () => Promise<void> }) {
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState("");
  const inFlight = useRef(false);
  async function run() {
    if (inFlight.current || !task.trim()) return;
    inFlight.current = true; setSaving(true); setError("");
    try { await onRun(); }
    catch (cause) { setError(cause instanceof Error ? cause.message : "Unable to start the assignment. Please retry."); }
    finally { inFlight.current = false; setSaving(false); }
  }
  return <ConfigDialog title="Run manually" onCancel={() => { if (!inFlight.current) onCancel(); }} onApply={() => { void run(); }} applyDisabled={saving || !task.trim()} applyLabel={saving ? "Starting…" : "Run assignment"}>
    <label className="block space-y-2 text-sm">Assignment<textarea autoFocus required disabled={saving} rows={5} maxLength={16000} value={task} onChange={(event) => onTaskChange(event.target.value)} className={configInput} /></label>
    {error && <p role="alert" className="text-sm text-red-300">{error}</p>}
  </ConfigDialog>;
}
