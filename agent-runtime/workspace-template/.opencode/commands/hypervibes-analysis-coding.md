---
description: Improve reusable analysis code in an isolated candidate workspace
---

Use the `analysis-coding` skill, native path-restricted filesystem/LSP
tools, and the fixed validation/report MCP tools to inspect the candidate
analysis tree, make evidence-supported changes, validate them, and submit one
structured coding report before ending the session. In
bootstrap mode with no valid package, create a valid `manifest.json` and at
least one declared Python validation target, choosing every target ID,
filename, module, and package layout. Bootstrap the smallest one-candle-safe
baseline first; do not implement the full analysis strategy at once. When the
package exists but is invalid, rebuild a valid manifest and declared target set
while preserving only files you judge useful. Generate auditable measurements
and calculation-derived indicator signals, not trading policy. Do not probe
skill files, MCP resources, broad filesystem paths, or `scratch`; the loaded
skill is the complete contract. Do not use shell, filesystem paths outside
`scripts/user`, live workspace paths, prompts, or package installation. Treat
fixed validation as authoritative and submit the report only after it returns
`ok: true`. Do not inspect orders during bootstrap; use order evidence only when
a source review identifies a relevant analysis defect.
