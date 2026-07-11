use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use chrono::Utc;
use serde::Deserialize;
use tracing::warn;

use super::memories::{
    AgentMemoriesQuery, parse_memory_date_filter, prepare_memory_timeline_page,
};
use super::settings::build_opencode_workspace_settings_view;
use super::transactions::apply_live_cash_balance_anchor;
use crate::{
    agents::{
        model::BACKEND_KIND_OPENCODE,
        strategy_prompts::{
            default_prompt_for_kind, list_agent_strategy_prompts, PROMPT_KIND_ANALYSIS,
            PROMPT_KIND_DAILY_REVIEW, PROMPT_KIND_MARKET_ANALYSIS, PROMPT_KIND_TRADING,
        },
        store::{get_agent, list_agent_instrument_ids, list_agent_instrument_options},
    },
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
        queries::{
            BalanceSeriesBucket, count_account_transactions, fetch_balance_series,
            latest_account_running_balance, list_account_sync_state,
            list_account_transactions_page,
        },
    },
    memory::{
        get_latest_agent_memory_by_type, get_memory, list_agent_memory_timeline,
        memory_expires_at,
    },
    web::{
        AppState,
        error::AppError,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentShowTab,
            AgentsShowPageTemplate, BalanceSparklinesPartialTemplate,
            LatestAnalysisSummaryPartialTemplate, LatestTradeExecutionSummaryPartialTemplate,
            OpenOrdersPartialTemplate, OpenOrdersView, OpenPositionsPartialTemplate,
            OpenPositionsView, SparklineView, SyncStateView, TransactionView,
        },
    },
};
pub(in crate::web::routes) async fn agents_show(
    State(state): State<Arc<AppState>>,
    Path(agent_key): Path<String>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &agent_key,
        AgentShowTab::Positions,
        None,
        None,
        None,
        None,
    )
    .await
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentJobsQuery {
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub warning: Option<String>,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentTransactionsQuery {
    #[serde(default)]
    pub page: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentSettingsQuery {
    #[serde(default)]
    pub workspace_warning: Option<String>,
}
pub(in crate::web::routes) async fn render_agent_show_page(
    state: &Arc<AppState>,
    agent_key: &str,
    active_tab: AgentShowTab,
    transactions_query: Option<AgentTransactionsQuery>,
    memories_query: Option<AgentMemoriesQuery>,
    settings_query: Option<AgentSettingsQuery>,
    jobs_query: Option<AgentJobsQuery>,
) -> Result<Response, AppError> {
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };

    let mut template = AgentsShowPageTemplate::new(agent.clone(), active_tab);

    let instrument_options =
        match list_agent_instrument_options(&state.db_pool, &agent.agent_key).await {
            Ok(rows) => {
                template.instrument_options_loaded = true;
                template.has_selected_instruments = rows.iter().any(|row| row.selected);
                Some(rows)
            }
            Err(error) => {
                warn!(
                    agent_key = %agent.agent_key,
                    error = ?error,
                    "failed to list agent instrument options for agent page"
                );
                None
            }
        };

    match active_tab {
        AgentShowTab::Positions => {
            populate_positions_tab(state, &agent, &mut template).await?;
        }
        AgentShowTab::Transactions => {
            let requested_page = transactions_query
                .as_ref()
                .map(|query| parse_positive_page(&query.page))
                .unwrap_or(1);
            populate_transactions_tab(state, &agent, &mut template, requested_page).await;
        }
        AgentShowTab::Memories => {
            let memory_query = memories_query.unwrap_or_default();
            let (filter_date_value, selected_date_text, filter_error_text, since, until) =
                parse_memory_date_filter(&memory_query.date);

            match list_agent_memory_timeline(
                &state.db_pool,
                &agent.agent_key,
                since,
                until,
                None,
            )
            .await
            {
                Ok(rows) => {
                    let (rows, next_page_url) = prepare_memory_timeline_page(
                        &agent.agent_key,
                        rows,
                        selected_date_text
                            .as_ref()
                            .map(|_| filter_date_value.as_str()),
                    );
                    let selected_memory = match rows.first() {
                        Some(row) => get_memory(&state.db_pool, &agent.agent_key, row.id).await?,
                        None => None,
                    };
                    template.set_memories(
                        rows,
                        selected_memory,
                        filter_date_value,
                        selected_date_text,
                        filter_error_text,
                        next_page_url,
                    );
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list agent memories for operator page"
                    );
                }
            }
        }
        AgentShowTab::Prompts => {
            match list_agent_strategy_prompts(&state.db_pool, &agent.agent_key).await {
                Ok(rows) => {
                    let prompt_map: std::collections::BTreeMap<String, String> = rows
                        .into_iter()
                        .map(|row| (row.prompt_kind, row.prompt))
                        .collect();
                    template.set_prompt_editors(vec![
                        crate::web::templates::PromptEditorView::new(
                            PROMPT_KIND_ANALYSIS,
                            prompt_map
                                .get(PROMPT_KIND_ANALYSIS)
                                .cloned()
                                .unwrap_or_default(),
                            default_prompt_for_kind(PROMPT_KIND_ANALYSIS),
                        ),
                        crate::web::templates::PromptEditorView::new(
                            PROMPT_KIND_MARKET_ANALYSIS,
                            prompt_map
                                .get(PROMPT_KIND_MARKET_ANALYSIS)
                                .cloned()
                                .unwrap_or_default(),
                            default_prompt_for_kind(PROMPT_KIND_MARKET_ANALYSIS),
                        ),
                        crate::web::templates::PromptEditorView::new(
                            PROMPT_KIND_TRADING,
                            prompt_map
                                .get(PROMPT_KIND_TRADING)
                                .cloned()
                                .unwrap_or_default(),
                            default_prompt_for_kind(PROMPT_KIND_TRADING),
                        ),
                        crate::web::templates::PromptEditorView::new(
                            PROMPT_KIND_DAILY_REVIEW,
                            prompt_map
                                .get(PROMPT_KIND_DAILY_REVIEW)
                                .cloned()
                                .unwrap_or_default(),
                            default_prompt_for_kind(PROMPT_KIND_DAILY_REVIEW),
                        ),
                    ]);
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list strategy prompts for operator page"
                    );
                }
            }
        }
        AgentShowTab::Settings => {
            if agent.backend_kind == BACKEND_KIND_OPENCODE {
                template.settings_workspace_warning = settings_query
                    .as_ref()
                    .and_then(|query| query.workspace_warning.clone());
                template.opencode_workspace =
                    build_opencode_workspace_settings_view(state, &agent).await;
            }
            template.sync_state = match list_account_sync_state(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(rows) => rows.into_iter().map(SyncStateView::from_row).collect(),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list account sync state for agent settings page"
                    );
                    Vec::new()
                }
            };
            match instrument_options {
                Some(rows) => {
                    template.instrument_options = rows;
                }
                None => {}
            }
        }
        AgentShowTab::Jobs => {
            if agent.backend_kind != BACKEND_KIND_OPENCODE {
                return Ok((
                    StatusCode::NOT_FOUND,
                    "jobs are only available for OpenCode agents",
                )
                    .into_response());
            }
            let requested_page = jobs_query
                .as_ref()
                .map(|query| parse_positive_page(&query.page))
                .unwrap_or(1);
            template.jobs_warning = jobs_query.as_ref().and_then(|query| query.warning.clone());
            populate_jobs_tab(state, &agent, &mut template, requested_page).await;
        }
    }

    Ok(Html(template.render()?).into_response())
}
pub(in crate::web::routes) async fn populate_jobs_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
    requested_runs_page: usize,
) {
    const RUNS_PER_PAGE: usize = 10;

    match crate::agentic::store::list_agent_schedules(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => {
            template.jobs_loaded = true;
            template.jobs = rows
                .iter()
                .map(crate::web::templates::AgenticJobScheduleView::from_row)
                .collect();
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list agent jobs for operator page"
            );
        }
    }

    match crate::agentic::store::list_agent_hooks(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => {
            template.hooks_loaded = true;
            template.hooks = rows
                .iter()
                .map(crate::web::templates::AgenticJobHookView::from_row)
                .collect();
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list agent hooks for operator page"
            );
        }
    }

    if template.jobs_loaded && template.hooks_loaded {
        template.can_enable_all_jobs = template.jobs.iter().any(|job| !job.enabled)
            || template.hooks.iter().any(|hook| !hook.enabled);
        template.can_disable_all_jobs = template.jobs.iter().any(|job| job.enabled)
            || template.hooks.iter().any(|hook| hook.enabled);
    }

    match crate::agentic::store::count_agent_runs(&state.db_pool, &agent.agent_key).await {
        Ok(total_count) => {
            let total_count = total_count as usize;
            let total_pages = if total_count == 0 {
                0
            } else {
                (total_count + RUNS_PER_PAGE - 1) / RUNS_PER_PAGE
            };
            let current_page = if total_pages == 0 {
                1
            } else {
                requested_runs_page.min(total_pages)
            };

            template.recent_runs_page = current_page;
            template.recent_runs_total_pages = total_pages;
            template.recent_runs_total_count = total_count;
            template.recent_runs_previous_page_url = (current_page > 1)
                .then(|| format!("/agents/{}/jobs?page={}", agent.agent_key, current_page - 1));
            template.recent_runs_next_page_url = (total_pages > 0 && current_page < total_pages)
                .then(|| format!("/agents/{}/jobs?page={}", agent.agent_key, current_page + 1));

            if total_count == 0 {
                template.recent_runs_loaded = true;
                return;
            }

            let offset = ((current_page - 1) * RUNS_PER_PAGE) as i64;
            match crate::agentic::store::list_agent_runs_page(
                &state.db_pool,
                &agent.agent_key,
                RUNS_PER_PAGE as i64,
                offset,
            )
            .await
            {
                Ok(rows) => {
                    let run_count = rows.len();
                    template.recent_runs_loaded = true;
                    template.recent_runs = rows
                        .iter()
                        .map(crate::web::templates::AgenticRunView::from_row)
                        .collect();
                    template.recent_runs_range_start = offset as usize + 1;
                    template.recent_runs_range_end = offset as usize + run_count;
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list recent agent runs for jobs page"
                    );
                }
            }
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to count recent agent runs for jobs page"
            );
        }
    }
}
pub(in crate::web::routes) fn parse_positive_page(raw: &str) -> usize {
    raw.trim()
        .parse::<usize>()
        .ok()
        .filter(|page| *page > 0)
        .unwrap_or(1)
}
pub(in crate::web::routes) async fn populate_transactions_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
    requested_transactions_page: usize,
) {
    const TRANSACTIONS_PER_PAGE: usize = 25;

    match count_account_transactions(&state.db_pool, &agent.wallet_address, &agent.environment)
        .await
    {
        Ok(total_count) => {
            let total_count = total_count as usize;
            let total_pages = if total_count == 0 {
                0
            } else {
                (total_count + TRANSACTIONS_PER_PAGE - 1) / TRANSACTIONS_PER_PAGE
            };
            let current_page = if total_pages == 0 {
                1
            } else {
                requested_transactions_page.min(total_pages)
            };

            template.transactions_page = current_page;
            template.transactions_total_pages = total_pages;
            template.transactions_total_count = total_count;
            template.transactions_previous_page_url = (current_page > 1).then(|| {
                format!(
                    "/agents/{}/transactions?page={}",
                    agent.agent_key,
                    current_page - 1
                )
            });
            template.transactions_next_page_url = (total_pages > 0 && current_page < total_pages)
                .then(|| {
                    format!(
                        "/agents/{}/transactions?page={}",
                        agent.agent_key,
                        current_page + 1
                    )
                });

            if total_count == 0 {
                return;
            }

            let offset = ((current_page - 1) * TRANSACTIONS_PER_PAGE) as i64;
            let latest_running_balance = match latest_account_running_balance(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to fetch latest account running balance for transactions page"
                    );
                    None
                }
            };

            match list_account_transactions_page(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
                TRANSACTIONS_PER_PAGE as i64,
                offset,
            )
            .await
            {
                Ok(mut rows) => {
                    apply_live_cash_balance_anchor(state, agent, latest_running_balance, &mut rows);
                    let row_count = rows.len();
                    template.transactions =
                        rows.into_iter().map(TransactionView::from_row).collect();
                    template.transactions_range_start = offset as usize + 1;
                    template.transactions_range_end = offset as usize + row_count;
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list account transactions page for agent page"
                    );
                }
            }
        }
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                wallet_address = %agent.wallet_address,
                environment = %agent.environment,
                error = ?error,
                "failed to count account transactions for agent page"
            );
        }
    }
}
pub(in crate::web::routes) async fn populate_positions_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
) -> Result<(), AppError> {
    let configured_coins = match list_agent_instrument_ids(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => rows,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                error = ?error,
                "failed to list configured instruments for positions tab"
            );
            Vec::new()
        }
    };

    let account_key = AccountKey::new(&agent.wallet_address, &agent.environment);
    let live_snapshot = state
        .live_accounts
        .get(&account_key)
        .unwrap_or_else(|| AccountLiveState {
            account_address: account_key.account_address.clone(),
            environment: account_key.environment.clone(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        });

    let account_balance_view = AccountBalanceView::from_live_state(live_snapshot.clone());
    template.account_balance_html =
        AccountBalancePartialTemplate::render_view(account_balance_view.clone())
            .map_err(anyhow::Error::from)?;

    let open_positions_view = OpenPositionsView::from_live_state_with_configured_coins(
        live_snapshot.clone(),
        &configured_coins,
    );
    template.open_positions_html = OpenPositionsPartialTemplate::render_view(open_positions_view)
        .map_err(anyhow::Error::from)?;

    let open_orders_view = OpenOrdersView::from_live_state(live_snapshot.clone());
    template.open_orders_html =
        OpenOrdersPartialTemplate::render_view(open_orders_view).map_err(anyhow::Error::from)?;

    let latest_trade_execution =
        get_latest_agent_memory_by_type(&state.db_pool, &agent.agent_key, "trade_execution")
            .await?;
    template.latest_trade_execution_summary_html =
        LatestTradeExecutionSummaryPartialTemplate::render_view(
            latest_trade_execution
                .as_ref()
                .map(|memory| memory.summary.clone()),
            latest_trade_execution
                .as_ref()
                .map(|memory| memory.created_at),
        )
        .map_err(anyhow::Error::from)?;

    let latest_market_analysis =
        get_latest_agent_memory_by_type(&state.db_pool, &agent.agent_key, "market_analysis")
            .await?;
    let analysis_detail_url = latest_market_analysis
        .as_ref()
        .map(|memory| format!("/agents/{}/memories/{}", agent.agent_key, memory.id));
    template.latest_analysis_summary_html = LatestAnalysisSummaryPartialTemplate::render_view(
        latest_market_analysis
            .as_ref()
            .map(|memory| memory.summary.clone()),
        analysis_detail_url,
        latest_market_analysis
            .as_ref()
            .map(|memory| memory.created_at),
        latest_market_analysis.as_ref().and_then(memory_expires_at),
    )
    .map_err(anyhow::Error::from)?;

    let now = Utc::now();
    let since_24h = now - chrono::Duration::hours(24);
    let since_30d = now - chrono::Duration::days(30);
    let series_24h = match fetch_balance_series(
        &state.db_pool,
        &agent.wallet_address,
        &agent.environment,
        since_24h,
        BalanceSeriesBucket::Hour,
    )
    .await
    {
        Ok(points) => points,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                wallet_address = %agent.wallet_address,
                environment = %agent.environment,
                error = ?error,
                "failed to fetch 24h balance series for agent page"
            );
            Vec::new()
        }
    };
    let series_30d = match fetch_balance_series(
        &state.db_pool,
        &agent.wallet_address,
        &agent.environment,
        since_30d,
        BalanceSeriesBucket::Day,
    )
    .await
    {
        Ok(points) => points,
        Err(error) => {
            warn!(
                agent_key = %agent.agent_key,
                wallet_address = %agent.wallet_address,
                environment = %agent.environment,
                error = ?error,
                "failed to fetch 30d balance series for agent page"
            );
            Vec::new()
        }
    };
    let sparklines = vec![
        SparklineView::from_series("24 hours", &series_24h, 240, 48),
        SparklineView::from_series("30 days", &series_30d, 240, 48),
    ];
    template.sparklines_html =
        BalanceSparklinesPartialTemplate::render_view(sparklines).map_err(anyhow::Error::from)?;

    Ok(())
}
