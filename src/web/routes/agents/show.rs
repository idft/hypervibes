use std::sync::Arc;

use askama::Template;
use axum::{
    extract::{Path, Query, State},
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use chrono::Utc;
use serde::Deserialize;
use tracing::warn;

use super::super::account::agent_subaccount_name;
use super::memories::{AgentMemoriesQuery, parse_memory_date_filter, prepare_memory_timeline_page};
use super::transactions::apply_live_cash_balance_anchor;
use crate::{
    agents::store::{
        get_agent, get_agent_readiness, list_agent_instrument_ids, list_agent_instrument_options,
    },
    harness::model::SUB_AGENT_KIND_ANALYSIS,
    hyperliquid::{
        live_state::{AccountKey, AccountLiveState, LiveConnectionStatus},
        queries::{
            BalanceSeriesBucket, count_account_transactions, fetch_balance_series,
            latest_account_running_balance, list_account_transactions_page,
        },
    },
    memory::{
        get_latest_trading_decision, get_memory, list_agent_memory_timeline, memory_expires_at,
    },
    notifications::store::{count_notifications, list_notification_history},
    web::{
        AppState,
        auth::AuthenticatedUser,
        error::AppError,
        templates::{
            AccountBalancePartialTemplate, AccountBalanceView, AgentRecentRunsView, AgentShowTab,
            AgentsShowPageTemplate, BalanceSparklinesPartialTemplate,
            LatestTradeDecisionSummaryPartialTemplate, OpenOrdersPartialTemplate, OpenOrdersView,
            OpenPositionsPartialTemplate, OpenPositionsView, SparklineView, TransactionView,
            load_navbar,
        },
    },
};
pub(in crate::web::routes) async fn agents_show(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(query): Query<AgentOperationQuery>,
) -> Result<Response, AppError> {
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Positions,
        AgentShowQueries {
            operation_notice: query.notice,
            ..Default::default()
        },
    )
    .await
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentSubAgentsQuery {
    #[serde(default)]
    pub page: String,
    #[serde(default)]
    pub warning: Option<String>,
    #[serde(default)]
    pub kind: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentTransactionsQuery {
    #[serde(default)]
    pub page: String,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentSettingsQuery {
    #[serde(default)]
    pub notice: Option<String>,
    #[serde(default)]
    pub gateway_link: Option<uuid::Uuid>,
}
#[derive(Debug, Clone, Default, Deserialize)]
pub(in crate::web::routes) struct AgentOperationQuery {
    #[serde(default)]
    pub notice: Option<String>,
}

#[derive(Debug, Default)]
pub(in crate::web::routes) struct AgentShowQueries {
    pub operation_notice: Option<String>,
    pub transactions: Option<AgentTransactionsQuery>,
    pub memories: Option<AgentMemoriesQuery>,
    pub settings: Option<AgentSettingsQuery>,
    pub sub_agents: Option<AgentSubAgentsQuery>,
}

pub(in crate::web::routes) async fn render_agent_show_page(
    state: &Arc<AppState>,
    user: &AuthenticatedUser,
    agent_key: &str,
    active_tab: AgentShowTab,
    queries: AgentShowQueries,
) -> Result<Response, AppError> {
    let AgentShowQueries {
        operation_notice,
        transactions: transactions_query,
        memories: memories_query,
        settings: settings_query,
        sub_agents: sub_agents_query,
    } = queries;
    let Some(agent) = get_agent(&state.db_pool, agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let readiness = get_agent_readiness(&state.db_pool, agent_key)
        .await?
        .ok_or_else(|| AppError(anyhow::anyhow!("agent disappeared while loading readiness")))?;

    let notification_count = count_notifications(&state.db_pool, &agent.agent_key).await?;
    let mut template = AgentsShowPageTemplate::new(agent.clone(), active_tab, notification_count);
    template.setup_checklist =
        crate::web::templates::AgentSetupChecklistView::from_readiness(&readiness);
    template.operation_notice = operation_notice;
    template.navbar = load_selected_agent_navbar(state, user.id, &agent).await?;
    if let Some(address) = agent.trading_account_address.as_deref() {
        let is_main = address.eq_ignore_ascii_case(&user.wallet_address);
        template.is_main_account = is_main;
        if !is_main {
            template.subaccount_name = Some(agent_subaccount_name(&agent.display_name));
        }
    }

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
        AgentShowTab::Chat => unreachable!("Chat has its own page route"),
        AgentShowTab::Trading | AgentShowTab::Review => {
            unreachable!("Trading and Review have their own page routes")
        }
        AgentShowTab::Positions => {
            populate_positions_tab(state, &agent, &mut template).await?;
        }
        AgentShowTab::Notifications => {
            template.set_notifications(
                list_notification_history(&state.db_pool, &agent.agent_key).await?,
            );
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

            match list_agent_memory_timeline(&state.db_pool, &agent.agent_key, since, until, None)
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
        AgentShowTab::Settings => {
            template.settings_notice = settings_query
                .as_ref()
                .and_then(|query| query.notice.clone());
            let gateway_link = settings_query.as_ref().and_then(|query| query.gateway_link);
            template.gateway_telegram = Some(
                super::gateway::load_telegram_gateway_view(
                    state,
                    &agent.agent_key,
                    user.id,
                    gateway_link,
                )
                .await,
            );
            if let Some(rows) = instrument_options {
                template.instrument_options = rows;
            }
        }
        AgentShowTab::Analysis | AgentShowTab::SubAgentsHeading => {
            let requested_page = sub_agents_query
                .as_ref()
                .map(|query| parse_positive_page(&query.page))
                .unwrap_or(1);
            template.jobs_warning = sub_agents_query
                .as_ref()
                .and_then(|query| query.warning.clone());
            populate_sub_agents_tab(state, &agent, &mut template, requested_page).await;
        }
    }

    Ok(Html(template.render()?).into_response())
}

pub(in crate::web::routes) async fn load_selected_agent_navbar(
    state: &Arc<AppState>,
    user_id: uuid::Uuid,
    agent: &crate::agents::model::AgentDetailRow,
) -> Result<crate::web::templates::Navbar, AppError> {
    load_navbar(&state.db_pool, user_id)
        .await
        .map(|navbar| {
            navbar.with_selected_agent(
                agent.agent_key.clone(),
                agent.display_name.clone(),
                agent.enabled,
            )
        })
        .map_err(AppError)
}
const RUNS_PER_PAGE: usize = 10;

pub(in crate::web::routes) async fn build_agent_recent_runs_view(
    state: &Arc<AppState>,
    agent_key: &str,
    requested_runs_page: usize,
) -> AgentRecentRunsView {
    build_agent_recent_runs_view_for_kind(
        state,
        agent_key,
        SUB_AGENT_KIND_ANALYSIS,
        &format!("/agents/{agent_key}/analysis"),
        requested_runs_page,
    )
    .await
}

pub(in crate::web::routes) async fn build_agent_recent_runs_view_for_kind(
    state: &Arc<AppState>,
    agent_key: &str,
    sub_agent_kind: &str,
    page_path: &str,
    requested_runs_page: usize,
) -> AgentRecentRunsView {
    let mut view =
        AgentRecentRunsView::new_for_kind(agent_key, requested_runs_page, sub_agent_kind);

    match crate::harness::store::count_agent_runs_for_kind(
        &state.db_pool,
        agent_key,
        sub_agent_kind,
    )
    .await
    {
        Ok(total_count) => {
            let total_count = total_count as usize;
            let total_pages = if total_count == 0 {
                0
            } else {
                total_count.div_ceil(RUNS_PER_PAGE)
            };
            let current_page = if total_pages == 0 {
                1
            } else {
                requested_runs_page.min(total_pages)
            };

            view.recent_runs_page = current_page;
            view.recent_runs_total_pages = total_pages;
            view.recent_runs_total_count = total_count;
            view.recent_runs_previous_page_url =
                (current_page > 1).then(|| format!("{page_path}?page={}", current_page - 1));
            view.recent_runs_next_page_url = (total_pages > 0 && current_page < total_pages)
                .then(|| format!("{page_path}?page={}", current_page + 1));

            if total_count == 0 {
                view.recent_runs_loaded = true;
                return view;
            }

            let offset = ((current_page - 1) * RUNS_PER_PAGE) as i64;
            match crate::harness::store::list_agent_runs_page_for_kind(
                &state.db_pool,
                agent_key,
                sub_agent_kind,
                RUNS_PER_PAGE as i64,
                offset,
            )
            .await
            {
                Ok(rows) => {
                    let run_count = rows.len();
                    view.recent_runs_loaded = true;
                    view.recent_runs = rows
                        .iter()
                        .map(crate::web::templates::HarnessSubAgentRunView::from_row)
                        .collect();
                    view.recent_runs_range_start = offset as usize + 1;
                    view.recent_runs_range_end = offset as usize + run_count;
                }
                Err(error) => {
                    warn!(
                        agent_key,
                        error = ?error,
                        "failed to list recent agent runs for jobs page"
                    );
                }
            }
        }
        Err(error) => {
            warn!(
                agent_key,
                error = ?error,
                "failed to count recent agent runs for jobs page"
            );
        }
    }

    view
}

pub(in crate::web::routes) async fn populate_sub_agents_tab(
    state: &Arc<AppState>,
    agent: &crate::agents::model::AgentDetailRow,
    template: &mut AgentsShowPageTemplate,
    requested_runs_page: usize,
) {
    match crate::harness::store::list_analysis_sub_agents(&state.db_pool, &agent.agent_key).await {
        Ok(rows) => {
            template.jobs_loaded = true;
            template.jobs = rows
                .iter()
                .map(crate::web::templates::HarnessSubAgentView::from_row)
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

    if template.jobs_loaded {
        template.can_enable_all_jobs = template.jobs.iter().any(|job| !job.enabled);
        template.can_disable_all_jobs = template.jobs.iter().any(|job| job.enabled);
    }

    template.recent_runs_section =
        build_agent_recent_runs_view(state, &agent.agent_key, requested_runs_page).await;
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
    let Some(account_address) = agent.trading_account_address.as_deref() else {
        return;
    };

    match count_account_transactions(&state.db_pool, account_address, &agent.environment).await {
        Ok(total_count) => {
            let total_count = total_count as usize;
            let total_pages = if total_count == 0 {
                0
            } else {
                total_count.div_ceil(TRANSACTIONS_PER_PAGE)
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
                account_address,
                &agent.environment,
            )
            .await
            {
                Ok(value) => value,
                Err(error) => {
                    warn!(
                        agent_key = %agent.agent_key,
                        trading_account_address = %account_address,
                        environment = %agent.environment,
                        error = ?error,
                        "failed to fetch latest account running balance for transactions page"
                    );
                    None
                }
            };

            match list_account_transactions_page(
                &state.db_pool,
                account_address,
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
                        trading_account_address = %account_address,
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
                trading_account_address = %account_address,
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
    let market_data = state.market_data.snapshot();

    let Some(account_address) = agent.trading_account_address.as_deref() else {
        return Ok(());
    };
    let account_key = AccountKey::new(account_address, &agent.environment);
    let live_snapshot = state
        .live_accounts
        .get(&account_key)
        .unwrap_or_else(|| AccountLiveState {
            account_address: account_key.account_address.clone(),
            environment: account_key.environment.clone(),
            status: LiveConnectionStatus::Starting,
            ..Default::default()
        });

    let health_view = crate::web::templates::LiveAccountHealthView::from_live_state(&live_snapshot);
    template.live_account_health_html =
        crate::web::templates::LiveAccountHealthPartialTemplate::render_view(health_view)
            .map_err(anyhow::Error::from)?;

    let account_balance_view = AccountBalanceView::from_live_state(live_snapshot.clone());
    template.account_balance_html =
        AccountBalancePartialTemplate::render_view(account_balance_view.clone())
            .map_err(anyhow::Error::from)?;

    let open_positions_view =
        OpenPositionsView::from_live_state_with_configured_coins_and_market_data(
            live_snapshot.clone(),
            &configured_coins,
            &market_data,
        )
        .with_agent_key(&agent.agent_key);
    template.open_positions_html = OpenPositionsPartialTemplate::render_view(open_positions_view)
        .map_err(anyhow::Error::from)?;

    let open_orders_view = OpenOrdersView::from_live_state(live_snapshot.clone());
    template.open_orders_html =
        OpenOrdersPartialTemplate::render_view(open_orders_view).map_err(anyhow::Error::from)?;

    let latest_decision = get_latest_trading_decision(&state.db_pool, &agent.agent_key).await?;
    let decision_detail_url = latest_decision
        .as_ref()
        .map(|memory| format!("/agents/{}/memories/{}", agent.agent_key, memory.id));
    template.latest_trade_decision_summary_html =
        LatestTradeDecisionSummaryPartialTemplate::render_view(
            latest_decision
                .as_ref()
                .map(|memory| memory.summary.clone()),
            decision_detail_url,
            latest_decision.as_ref().map(|memory| memory.created_at),
            latest_decision.as_ref().and_then(memory_expires_at),
        )
        .map_err(anyhow::Error::from)?;

    let now = Utc::now();
    let since_24h = now - chrono::Duration::hours(24);
    let since_30d = now - chrono::Duration::days(30);
    let series_24h = match fetch_balance_series(
        &state.db_pool,
        account_address,
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
                trading_account_address = %account_address,
                environment = %agent.environment,
                error = ?error,
                "failed to fetch 24h balance series for agent page"
            );
            Vec::new()
        }
    };
    let series_30d = match fetch_balance_series(
        &state.db_pool,
        account_address,
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
                trading_account_address = %account_address,
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
