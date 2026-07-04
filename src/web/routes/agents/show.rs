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

use super::memories::{AgentMemoriesQuery, parse_memory_date_filter};
use super::settings::build_opencode_workspace_settings_view;
use super::transactions::apply_live_cash_balance_anchor;
use crate::{
    agents::{
        model::BACKEND_KIND_OPENCODE,
        store::{get_agent, list_agent_instrument_options},
    },
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
        queries::{
            BalanceSeriesBucket, fetch_balance_series,
            list_account_sync_state, list_all_account_transactions,
        },
    },
    memory::{get_latest_agent_memory_by_type, list_agent_memories, memory_expires_at},
    web::{
        error::AppError,
        AppState,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentShowTab,
            AgentsShowPageTemplate, BalanceSparklinesPartialTemplate,
            LatestAnalysisSummaryPartialTemplate,
            LatestTradeExecutionSummaryPartialTemplate,
            OpenOrdersPartialTemplate, OpenOrdersView, OpenPositionsPartialTemplate,
            OpenPositionsView, SparklineView,
            SyncStateView, TransactionView,
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
pub(in crate::web::routes) struct AgentSettingsQuery {
    #[serde(default)]
    pub workspace_warning: Option<String>,
}
pub(in crate::web::routes) async fn render_agent_show_page(
    state: &Arc<AppState>,
    agent_key: &str,
    active_tab: AgentShowTab,
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
            template.transactions = match list_all_account_transactions(
                &state.db_pool,
                &agent.wallet_address,
                &agent.environment,
            )
            .await
            {
                Ok(mut rows) => {
                    apply_live_cash_balance_anchor(state, &agent, &mut rows);
                    rows.into_iter().map(TransactionView::from_row).collect()
                }
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        wallet_address = %agent.wallet_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to list full account transactions for agent page"
                    );
                    Vec::new()
                }
            };
        }
        AgentShowTab::Memories => {
            let memory_query = memories_query.unwrap_or_default();
            let (filter_date_value, selected_date_text, filter_error_text, since, until) =
                parse_memory_date_filter(&memory_query.date);

            match list_agent_memories(&state.db_pool, &agent.agent_key, since, until).await {
                Ok(rows) => template.set_memories(
                    rows,
                    filter_date_value,
                    selected_date_text,
                    filter_error_text,
                ),
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        error = ?error,
                        "failed to list agent memories for operator page"
                    );
                }
            }
        }
        AgentShowTab::Prompts => {}
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
pub(in crate::web::routes) async fn populate_positions_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
) -> Result<(), AppError> {
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

    let open_positions_view = OpenPositionsView::from_live_state(live_snapshot.clone());
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
