export function SubagentEnabledToggle({ enabled, disabled, onClick }: { enabled: boolean; disabled?: boolean; onClick: () => void }) {
  return <button type="button" disabled={disabled} aria-label={enabled ? "Pause sub-agent" : "Enable sub-agent"} aria-pressed={enabled} title={enabled ? "Pause new assignments" : "Enable new assignments"} onClick={onClick} className={`inline-flex cursor-pointer items-center gap-1.5 rounded-full px-2 py-1 text-xs font-medium disabled:cursor-not-allowed disabled:opacity-50 ${enabled ? "text-emerald-300 hover:bg-emerald-950" : "text-amber-300 hover:bg-amber-950"}`}>
    <svg aria-hidden="true" viewBox="0 0 16 16" className="size-3.5" fill="currentColor">{enabled ? <path d="M4 2.5v11L13 8z" /> : <path d="M3 2h4v12H3zm6 0h4v12H9z" />}</svg>
    {enabled ? "Enabled" : "Paused"}
  </button>;
}
