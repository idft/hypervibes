/// Global system prompt prepended to every OpenCode agent job prompt.
pub const SYSTEM_PROMPT: &str =
    "This is the HyperVibes system for trading perpetual futures on Hyperliquid.";

/// Default Analysis strategy prompt. Generic research framing: individual
/// Analysis jobs and their capabilities define the concrete research
/// method. Operators may replace it entirely with their own structure,
/// which is passed verbatim to the LLM.
pub const DEFAULT_ANALYSIS_STRATEGY_PROMPT: &str = "## Strategy overview\nConservative swing trading research. Trade in the direction of the higher-timeframe trend and favor continuation over counter-trend reversals.\n\n## Markets & timeframes\nAnalyze this job's timeframe, always framed against higher-timeframe context (trend and key levels).\n\n## What to analyze\nMarket structure, trend and momentum, volatility regime, support/resistance, liquidity zones, and risk/reward.\n\n## Bias & actionable criteria\nClassify the bias as long, short, neutral, or mixed. A setup is only actionable when there is a clear directional edge with a defined entry zone, a logical invalidation level, and at least one target.\n\n## Confidence & validity\nMinimum confidence for an actionable setup: 0.65. Default research validity: 15 minutes; shorten it when volatility is high or the setup hinges on a level that may break soon.\n\n## No-trade discipline\nIf there is no clear edge, mark the bias neutral or mixed and provide no actionable setup. Do not manufacture setups.";

/// Default trading strategy prompt (the editable "user prompt" describing the
/// trading configuration). This is an example format; operators may replace it
/// entirely with their own structure, which is passed verbatim to the LLM.
pub const DEFAULT_TRADING_STRATEGY_PROMPT: &str = "## Position sizing\nOpen small starter positions; never take a full-size position on the first fill. Scale in only on confirmation, never to average down a losing position.\n\n## Entry strategy\nUse 1-3 resting limit orders laddered across the setup's entry zone. Weight size toward stronger levels and use smaller size on lower-confidence setups. Time-in-force: normal resting limit (gtc) unless the setup explicitly requires maker-only (alo).\n\n## Stop loss (strongly recommended)\nAttach a stop-loss leg to every entry at (or just beyond) the research invalidation level. Do not omit it unless the operator explicitly directs an unprotected order.\n\n## Take profit (strongly recommended)\nAttach take-profit leg(s) at the research targets. Prefer scaling out (partial take-profit at the first target, hold or trail a runner) over all-or-nothing exits.\n\n## Risk limits\nRequire a minimum reward:risk of 1.5:1 after fees; skip setups that do not clear it.\n\n## Confidence gate\nOnly open new exposure when the supporting research evidence is fresh, non-neutral, and confidence is at least 0.65. Do not add exposure when research is neutral, mixed, stale, or missing.\n\n## Decision log\nWrite a `trading_decision` memory for every evaluated instrument, including no-trade and position-management outcomes. Link the decision to every evidence memory you considered.\n\n## Order management\nCancel unfilled entry orders when the source research expires or is invalidated. Avoid duplicate resting orders at similar prices.\n\n## Notifications\nAfter each successful `hypervibes_submit_orders` call, use `hypervibes_send_notification` to send one concise notification. Identify the symbol, action, and submitted order details. Call orders submitted, not filled, unless you have separately observed a fill.\n\nWhen you observe a position change, send one concise notification with the symbol and the prior and current position details. Do not send duplicate notifications for the same event. Notifications are best-effort: if they cannot be queued, continue the trading workflow without changing the trading decision.";

/// Default Review strategy prompt.
pub const DEFAULT_REVIEW_STRATEGY_PROMPT: &str = "## Review goal\nReview the agent's recent research memories, trading decisions, orders, and learnings for one UTC day. Focus on whether decisions matched the evidence available at the time.\n\n## What to look for\nIdentify good patterns, repeated mistakes, stale assumptions, execution failures, risk-control failures, and places where prompts should be improved.\n\n## Learning standard\nOnly write new agent-level learnings when something materially changed in what the agent should remember going forward. Each new `agent_learnings` memory is the complete current canonical learning set: carry forward still-valid prior learnings, add the new learning, and explicitly remove or replace superseded rules. Use the fixed summary `Accumulated agent learnings`. Keep learnings durable, concise, and general enough to reuse across future sessions.\n\n## Workspace edits\nReview is a diagnosis job. Never modify `data/`, `scratch/`, or runtime files in any way. If review evidence suggests an improvement, describe it in the ordinary review memory content.";

#[cfg(test)]
mod tests {
    use super::DEFAULT_TRADING_STRATEGY_PROMPT;

    #[test]
    fn default_trading_prompt_requires_order_and_position_notifications() {
        assert!(
            DEFAULT_TRADING_STRATEGY_PROMPT
                .contains("After each successful `hypervibes_submit_orders` call")
        );
        assert!(DEFAULT_TRADING_STRATEGY_PROMPT.contains("When you observe a position change"));
        assert!(DEFAULT_TRADING_STRATEGY_PROMPT.contains("Notifications are best-effort"));
    }

    #[test]
    fn default_trading_prompt_requires_decision_log() {
        assert!(
            DEFAULT_TRADING_STRATEGY_PROMPT
                .contains("Write a `trading_decision` memory for every evaluated instrument")
        );
    }
}
