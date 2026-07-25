SET search_path TO public;

-- 2.1 Expand strategy prompt kinds to include analysis_coding.
ALTER TABLE agent_strategy_prompts
    DROP CONSTRAINT IF EXISTS agent_strategy_prompts_prompt_kind_check;

ALTER TABLE agent_strategy_prompts
    ADD CONSTRAINT agent_strategy_prompts_prompt_kind_check
    CHECK (prompt_kind IN (
        'analysis',
        'market_analysis',
        'trading',
        'daily_review',
        'analysis_coding'
    ));

INSERT INTO agent_strategy_prompts (agent_key, prompt_kind, prompt)
SELECT agent_key, 'analysis_coding', $analysis_coding_prompt$## Role
Improve the agent's reusable quantitative analysis code under `scripts/user` only when the analysis strategy or accumulated evidence supports a change.

## Required survey
- Inspect the current analysis strategy for calculations or custom indicators that reusable code can support.
- Read the latest accumulated agent learnings.
- Read the existing candidate files before creating or replacing code.

## Quantitative boundary
Produce auditable measurements and deterministic quantitative signals. A signal may report an indicator state, threshold crossing, crossover, breakout, divergence, or other calculation-derived event. Include the measurements and thresholds needed to audit it.

Do not generate trading policy. Reusable code must not decide final long/short bias, actionability, trading confidence, entries, exits, stops, targets, position sizing, or order instructions. Analysis jobs combine quantitative output with qualitative market evidence and decide what it means.

## Stable interface
Keep `scripts/user/analyze.py` as the canonical entrypoint with `--symbol`, `--timeframe`, `--boundary-ms`, `--input`, and `--output`. Supporting modules under `scripts/user` are allowed. Improve the stable implementation instead of creating suffixed duplicate entrypoints.

The canonical input is the JSON envelope written by `fetch_ohlcv.py`: `symbol`, `timeframe`, positive `interval_ms`, and normalized `candles` containing `timestamp_ms`, `open`, `high`, `low`, `close`, and `volume`. Validate that CLI and input symbol/timeframe agree. Treat `interval_ms` as authoritative; never infer cadence from candle spacing.

## Evidence discipline
Prefer a small correct implementation over a broad strategy engine. Make evidence-supported changes and return `no_change` when no reusable quantitative improvement is justified.

## Dependencies
Production code may use the immutable analysis runtime, including NumPy, pandas, SciPy, statsmodels, pandas-ta-classic, and Polars. Prefer established library calculations over handwritten indicator implementations. Never install or modify packages.

## Tests
Candidate tests are optional. Add a focused standard-library `unittest` regression when fixing a demonstrated bug or implementing nontrivial custom quantitative math. Do not recreate the platform contract tests or generate a comprehensive suite by default.

## Future-data discipline
Every candle path must exclude candles whose complete close time is not strictly before `--boundary-ms`. Candle timestamps are start times, so use the requested timeframe duration when determining close time. Treat the boundary as authoritative.

The exact eligibility rule is `timestamp_ms + interval_ms < boundary_ms`; a candle closing exactly at the boundary is excluded. Reject invalid context, missing intervals, unsupported input, and calculation failures with a non-zero exit rather than emitting a successful empty measurement set.

## Safety
Use native OpenCode filesystem tools only under the isolated candidate `scripts/user` tree, with workspace-relative paths such as `scripts/user/analyze.py`. Use Pyright LSP diagnostics for Python. Never call Vibetrading APIs, place or cancel orders, edit prompts, access another workspace, or hard-code agent-specific paths. Run fixed validation and submit exactly one structured report.$analysis_coding_prompt$
FROM agents
ON CONFLICT (agent_key, prompt_kind) DO NOTHING;

-- 2.2 Expand hook supported (job_kind, hook_event) pairs by replacing
-- the two separate kind/event checks with one paired constraint that
-- only permits the combinations the rest of the system understands.
ALTER TABLE agentic_job_hooks
    DROP CONSTRAINT IF EXISTS agentic_job_hooks_job_kind_check;
ALTER TABLE agentic_job_hooks
    DROP CONSTRAINT IF EXISTS agentic_job_hooks_hook_event_check;

ALTER TABLE agentic_job_hooks
    ADD CONSTRAINT agentic_job_hooks_kind_event_pair_check
    CHECK (
        (job_kind = 'market_analysis'
            AND hook_event = 'analysis_batch_completed')
     OR (job_kind = 'analysis_coding'
            AND hook_event = 'daily_review_completed')
    );

-- Seed one disabled analysis_coding hook per OpenCode agent. The
-- operator must enable it explicitly and select a model before any
-- autonomous coding run can queue. `ON CONFLICT` keeps the insert
-- idempotent if a hook was already created out-of-band.
INSERT INTO agentic_job_hooks (
    agent_key,
    job_key,
    job_kind,
    hook_event,
    enabled,
    model_provider_id,
    model_id,
    timeout_seconds,
    operator_prompt
)
SELECT
    agent_key,
    'analysis-coding',
    'analysis_coding',
    'daily_review_completed',
    false,
    NULL,
    NULL,
    1800,
    ''
FROM agents
ON CONFLICT (agent_key, job_kind, hook_event) DO NOTHING;

-- 2.3 Add analysis_coding to the run-kind allowlist.
ALTER TABLE agentic_runs
    DROP CONSTRAINT IF EXISTS agentic_runs_job_kind_check;

ALTER TABLE agentic_runs
    ADD CONSTRAINT agentic_runs_job_kind_check
    CHECK (job_kind IN (
        'analysis',
        'trading',
        'market_analysis',
        'daily_review',
        'analysis_coding'
    ));

-- 2.4 Generalize maintenance tasks for the coding worker state
-- machine. `phase` distinguishes coding phases (preparing through
-- rolling_back) from the simpler workspace_regenerate flow. `run_id`
-- links an coding task to its owning agentic run; workspace
-- regeneration has no associated run. `source_run_id` /
-- `source_memory_id` record the daily-review trigger that requested an
-- automatic coding run so duplicate triggers are deduplicated.
ALTER TABLE agentic_maintenance_tasks
    DROP CONSTRAINT IF EXISTS agentic_maintenance_tasks_task_kind_check;

-- `task_kind` is validated at the application layer so new global
-- maintenance kinds (e.g. provider_config_reload) can be added
-- without a migration. The original task_kind CHECK is intentionally
-- not re-added.

ALTER TABLE agentic_maintenance_tasks
    ADD COLUMN IF NOT EXISTS phase TEXT NOT NULL DEFAULT 'queued';

ALTER TABLE agentic_maintenance_tasks
    ADD CONSTRAINT agentic_maintenance_tasks_phase_check
    CHECK (phase IN (
        'queued',
        'preparing',
        'generating',
        'validating',
        'waiting_for_promotion',
        'promoting',
        'smoke_testing',
        'rolling_back',
        'completed'
    ));

ALTER TABLE agentic_maintenance_tasks
    ADD COLUMN IF NOT EXISTS run_id BIGINT UNIQUE REFERENCES agentic_runs(id)
    ON DELETE CASCADE;

ALTER TABLE agentic_maintenance_tasks
    ADD COLUMN IF NOT EXISTS source_run_id BIGINT REFERENCES agentic_runs(id)
    ON DELETE SET NULL;

ALTER TABLE agentic_maintenance_tasks
    ADD COLUMN IF NOT EXISTS source_memory_id UUID
    REFERENCES memory.records(id) ON DELETE SET NULL;

ALTER TABLE agentic_maintenance_tasks
    ADD COLUMN IF NOT EXISTS heartbeat_at TIMESTAMPTZ;

ALTER TABLE agentic_maintenance_tasks
    ADD COLUMN IF NOT EXISTS attempt_count INTEGER NOT NULL DEFAULT 0;

ALTER TABLE agentic_maintenance_tasks
    ADD CONSTRAINT agentic_maintenance_tasks_attempt_count_check
    CHECK (attempt_count >= 0);

ALTER TABLE agentic_maintenance_tasks
    ADD CONSTRAINT agentic_maintenance_tasks_regenerate_no_run_check
    CHECK (
        task_kind <> 'workspace_regenerate' OR run_id IS NULL
    );

ALTER TABLE agentic_maintenance_tasks
    ADD CONSTRAINT agentic_maintenance_tasks_coding_has_run_check
    CHECK (
        task_kind <> 'analysis_coding' OR run_id IS NOT NULL
    );

-- Backfill `phase` for existing rows so the new default ('queued') does
-- not lie about a finished or in-progress workspace regeneration.
UPDATE agentic_maintenance_tasks
   SET phase = 'completed'
 WHERE status IN ('succeeded', 'failed', 'aborted');

UPDATE agentic_maintenance_tasks
   SET phase = 'promoting'
 WHERE status = 'running';

-- Re-key the active maintenance uniqueness onto the agent regardless of
-- task kind so only one maintenance/coding task may be active per
-- agent at a time. This intentionally permits the new coding
-- worker to claim exclusivity that overlaps regeneration.
DROP INDEX IF EXISTS agentic_maintenance_tasks_one_active_workspace_task_idx;

CREATE UNIQUE INDEX agentic_maintenance_tasks_one_active_agent_task_idx
    ON agentic_maintenance_tasks (agent_key)
    WHERE status IN ('queued', 'running');

-- Idempotency index: a single source daily-review memory can produce at
-- most one analysis_coding task. A NULL `source_memory_id` (manual
-- runs) is excluded from the uniqueness.
CREATE UNIQUE INDEX agentic_coding_source_memory_once_idx
    ON agentic_maintenance_tasks (source_memory_id)
    WHERE task_kind = 'analysis_coding'
      AND source_memory_id IS NOT NULL;

-- Worker poll index: order queued tasks oldest first with bounded
-- pagination. Include `phase` for quick filtering of coding phases
-- during recovery prior to per-phase loading.
CREATE INDEX agentic_maintenance_tasks_status_phase_created_idx
    ON agentic_maintenance_tasks (status, phase, created_at, id);
