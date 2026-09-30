use axum::{
    Form,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use serde::Deserialize;
use std::sync::Arc;

use super::show::{AgentShowQueries, AgentTransactionsQuery, render_agent_show_page};
use crate::web::{
    AppState,
    auth::{AuthenticatedUser, csrf_cookie_value},
    error::AppError,
    templates::AgentShowTab,
};
pub(in crate::web::routes) async fn agents_show_transactions(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path(agent_key): Path<String>,
    Query(mut query): Query<AgentTransactionsQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    query.csrf_token = csrf_cookie_value(&headers).unwrap_or_default().to_string();
    render_agent_show_page(
        &state,
        &user,
        &agent_key,
        AgentShowTab::Transactions,
        AgentShowQueries {
            transactions: Some(query),
            ..Default::default()
        },
    )
    .await
}

pub(in crate::web::routes) async fn agents_show_trades(
    state: State<Arc<AppState>>,
    user: AuthenticatedUser,
    agent_key: Path<String>,
    Query(mut query): Query<AgentTransactionsQuery>,
    headers: HeaderMap,
) -> Result<Response, AppError> {
    query.is_trades = true;
    agents_show_transactions(state, user, agent_key, Query(query), headers).await
}

#[derive(Deserialize)]
pub(in crate::web::routes) struct JournalNoteForm {
    body: String,
    #[serde(default)]
    page: String,
}

pub(in crate::web::routes) async fn agents_add_journal_note(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Path((agent_key, kind, target)): Path<(String, String, String)>,
    Form(form): Form<JournalNoteForm>,
) -> Result<Response, AppError> {
    let Some(agent) = crate::agents::store::get_agent(&state.db_pool, &agent_key).await? else {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    };
    let (owns,): (bool,) =
        sqlx::query_as("SELECT EXISTS(SELECT 1 FROM agents WHERE agent_key=$1 AND user_id=$2)")
            .bind(&agent_key)
            .bind(user.id)
            .fetch_one(&state.db_pool)
            .await?;
    if !owns {
        return Ok((StatusCode::NOT_FOUND, "agent not found").into_response());
    }
    let Some(account) = agent.trading_account_address.as_deref() else {
        return Ok((StatusCode::NOT_FOUND, "account not assigned").into_response());
    };
    let saved = crate::hyperliquid::trade_store::add_note(
        &state.db_pool,
        crate::hyperliquid::trade_store::JournalNoteInput {
            account,
            environment: &agent.environment,
            kind: &kind,
            target: &target,
            author_kind: "human",
            author_id: &user.id.to_string(),
            body: &form.body,
            source_run_id: None,
            source_conversation_id: None,
        },
    )
    .await?;
    if saved.is_none() {
        return Ok((
            StatusCode::NOT_FOUND,
            "journal target not found or invalid note",
        )
            .into_response());
    }
    let page = form
        .page
        .parse::<usize>()
        .ok()
        .filter(|n| *n > 0)
        .unwrap_or(1);
    let path = if kind == "trade" {
        "trades"
    } else {
        "transactions"
    };
    let location = format!("/agents/{agent_key}/{path}?page={page}&kind={kind}&target={target}");
    Ok(Redirect::to(&location).into_response())
}
