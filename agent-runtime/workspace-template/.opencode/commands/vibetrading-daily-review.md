Run the daily review workflow for Vibetrading.

Review recent analyses, market analyses, orders, and learnings for the provided UTC review window.

Write one `daily_review` memory with `reviews` links to the memories that informed it.

If the durable agent-level learnings changed, write a new `agent_learnings` memory and link the daily review to it with `updates_learnings`.

Do not place or cancel orders.

Do not edit `scripts/user/`, `data/`, or `scratch/`. If reusable analysis
code should improve, record `analysis_coding_requested` and its reason
in the daily-review metadata for the separate coding job.
