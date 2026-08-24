# HyperVibes MCP Server

This directory contains the source for the `hypervibes` Model Context
Protocol (MCP) server that OpenCode agents use to talk to the HyperVibes
backend.

It is **runtime infrastructure, not an agent-authored script**:

- It is installed into the custom OpenCode container image at the fixed
  path `/opt/hypervibes/mcp/`.
- Generated agent workspaces register it as a local stdio MCP server in
  their `opencode.json`.
- The MCP server process is launched **per workspace** by OpenCode, not
  once globally.
- Auth credentials are read from the workspace's own `.env` file in the
  MCP process working directory:
  - `HYPERVIBES_API_BASE_URL`
  - `HYPERVIBES_API_KEY`
  - `HYPERVIBES_AGENT_KEY`

The HyperVibes backend remains the source of truth for agent identity,
memory scoping, instrument allowlists, and order permissions. The MCP
server is a thin transport layer; it does not hold any Hyperliquid private
keys, and its tool schemas never expose `api_key` (or any other credential)
as a callable argument.

## Strategy Prompts

Chat sessions can review and edit their own per-agent strategy prompts through
`list_strategy_prompts`, `get_strategy_prompt`, and `update_strategy_prompt`.
The available prompt kinds are `analysis`, `market_analysis`, `trading`,
`daily_review`, and `analysis_coding`. These tools are chat-only: scheduled
sub-agents receive their selected strategy prompt but do not edit it. Updates require
the chat operator's one-time OpenCode approval.

## Operator Logs

The MCP server redirects its stderr to the container log while leaving stdout
exclusively for the MCP stdio protocol. Entries include safe request
parameters, HTTP status, and response shape, but never API keys,
authorization headers, request bodies, or memory content.

If you are an agent reading this file from inside a generated workspace:
do not edit, import, or invoke this module directly. Use the `hypervibes`
MCP tools instead.

## Coding Sessions

The MCP server registers its full API for every workspace. OpenCode agent
permissions select the MCP tools available to each sub-agent session. The
`analysis-coding` agent permits only agent-scoped evidence reads, candidate
filesystem operations through path-scoped native OpenCode tools, fixed local
validation, and one structured report submission.

Coding candidate workspaces set only `HYPERVIBES_CODING_TASK_ID`.
Candidate file operations are not MCP calls: generated OpenCode native
read/edit/glob permissions restrict them to the current candidate's
`scripts/user/` tree, including the root-relative path OpenCode uses for non-Git
projects. This lets native edit events receive Pyright LSP diagnostics.
Independent worker manifests reject symlinks, unapproved extensions, oversized
files, and any post-validation tree change.
The coding MCP surface is reserved for operations ordinary filesystem
tools cannot provide. Validation runs the fixed container-global validator, accepts no
model-supplied command or path, and records a task-scoped result bound to the
candidate tree hash. Validation failures include bounded diagnostics. Any later
candidate write, patch, or deletion invalidates that result.

Coding reports are accepted only after successful fixed validation.
Reported paths are relative to the candidate's `scripts/user` root, such as
`analyze.py`, rather than workspace-relative `scripts/user/analyze.py`.
