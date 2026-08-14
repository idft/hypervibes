Run the daily review workflow for HyperVibes.

Review analyses, market analyses, orders, account transactions, and learnings
only for the provided UTC review window. Always pass both window bounds to
listing tools; do not inspect or mention records outside the window.

Page `hypervibes_list_account_transactions` with a fixed `limit` and
increasing `offset` until a page contains fewer rows than the limit.

Write one `daily_review` memory with `reviews` links to the memories that informed it.

If the durable agent-level learnings changed, write a new `agent_learnings` memory
whose summary is exactly `Accumulated agent learnings`. Its content must be the
complete canonical learning set: carry forward still-valid rules, add new rules,
and explicitly mark replaced rules. Link the daily review to it with
`updates_learnings`.

Do not place or cancel orders.

Do not edit `scripts/user/`, `data/`, or `scratch/`. If reusable analysis
code should improve, record `analysis_coding_requested` and its reason
in the daily-review metadata for the separate coding job.
