Run Vibetrading analysis for this agent.

- Load the current job context with the `get_analysis_context` MCP tool
  before reasoning.
- Read account state and relevant memories through the `vibetrading` MCP
  tools.
- Write durable notes or memories through the `write_memory` MCP tool when
  appropriate. You may also keep reusable research artifacts under
  `scripts/user/`.
- Prefer the `vibetrading` MCP tools for every backend interaction. Do not
  call Vibetrading HTTP APIs directly, and do not author Python scripts to
  reach the backend.
