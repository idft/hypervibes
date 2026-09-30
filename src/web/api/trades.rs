use axum::{
    Json,
    extract::{Path, Query, State},
    response::{IntoResponse, Response},
};
use serde::Deserialize;
use serde_json::json;
use std::sync::Arc;
use uuid::Uuid;

use super::error::ApiError;
use crate::{
    agents::{AuthenticatedAgent, store::get_agent},
    harness::model::RunApiScope,
    hyperliquid::trade_store,
    web::AppState,
};

async fn account(
    state: &AppState,
    agent: &AuthenticatedAgent,
) -> Result<(String, String), ApiError> {
    let row = get_agent(&state.db_pool, &agent.agent_key)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("agent not found"))?;
    Ok((
        row.trading_account_address
            .ok_or(ApiError::NotFound("account not assigned"))?,
        row.environment,
    ))
}

#[derive(Deserialize)]
pub(super) struct Page {
    limit: Option<i64>,
    offset: Option<i64>,
}

pub(super) async fn list_account_trades(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Query(page): Query<Page>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::TransactionRead)?;
    if page.offset.is_some_and(|o| o < 0) || page.limit.is_some_and(|l| !(1..=100).contains(&l)) {
        return Err(ApiError::Validation(
            "limit must be 1..100 and offset non-negative".into(),
        ));
    }
    let (account, env) = account(&state, &agent).await?;
    Ok(Json(
        trade_store::list_trades(
            &state.db_pool,
            &account,
            &env,
            page.limit.unwrap_or(50),
            page.offset.unwrap_or(0),
        )
        .await
        .map_err(ApiError::Internal)?,
    )
    .into_response())
}

pub(super) async fn get_account_trade(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path(id): Path<Uuid>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::TransactionRead)?;
    let (account, env) = account(&state, &agent).await?;
    let trade = trade_store::get_trade(&state.db_pool, &account, &env, id)
        .await
        .map_err(ApiError::Internal)?
        .ok_or(ApiError::NotFound("trade not found"))?;
    let fills = trade_store::trade_fills(&state.db_pool, &account, &env, id)
        .await
        .map_err(ApiError::Internal)?;
    Ok(Json(json!({"trade": trade, "fills": fills})).into_response())
}

pub(super) async fn get_account_notes(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path((kind, target)): Path<(String, String)>,
) -> Result<Response, ApiError> {
    super::require_run_api_scope(&agent, RunApiScope::TransactionRead)?;
    let (account, env) = account(&state, &agent).await?;
    if !matches!(kind.as_str(), "trade" | "fill" | "funding" | "ledger")
        || !trade_store::target_exists(&state.db_pool, &account, &env, &kind, &target)
            .await
            .map_err(ApiError::Internal)?
    {
        return Err(ApiError::NotFound("journal target not found"));
    }
    Ok(Json(
        trade_store::list_notes(&state.db_pool, &account, &env, &kind, &target)
            .await
            .map_err(ApiError::Internal)?,
    )
    .into_response())
}

#[derive(Deserialize)]
pub(super) struct NoteBody {
    body: String,
}

pub(super) async fn post_account_note(
    State(state): State<Arc<AppState>>,
    agent: AuthenticatedAgent,
    Path((kind, target)): Path<(String, String)>,
    Json(note): Json<NoteBody>,
) -> Result<Response, ApiError> {
    // A permanent API key alone never authorizes journal mutations. Conversation
    // provenance must be server-validated and explicitly enabled by policy.
    let conversation = agent.conversation_provenance();
    if agent.is_run_credential() {
        super::require_run_api_scope(&agent, RunApiScope::TradeJournalWrite)?;
    } else if let Some(id) = conversation {
        let (allowed,): (bool,) = sqlx::query_as(
            "SELECT EXISTS(SELECT 1 FROM agent_conversation_tool_policies
            WHERE conversation_id=$1 AND tool_group='journal_writes' AND policy='allow')",
        )
        .bind(id)
        .fetch_one(&state.db_pool)
        .await
        .map_err(ApiError::from)?;
        if !allowed {
            return Err(ApiError::Forbidden(
                "conversation does not permit journal writing",
            ));
        }
    } else {
        return Err(ApiError::Forbidden(
            "journal writing requires a scoped run or permitted conversation",
        ));
    }
    if note.body.trim().is_empty() || note.body.len() > 4000 {
        return Err(ApiError::Validation("note must be 1..4000 bytes".into()));
    }
    let (account, env) = account(&state, &agent).await?;
    let row = trade_store::add_note(
        &state.db_pool,
        trade_store::JournalNoteInput {
            account: &account,
            environment: &env,
            kind: &kind,
            target: &target,
            author_kind: "agent",
            author_id: &agent.agent_key,
            body: &note.body,
            source_run_id: agent.run_provenance().map(|(id, _)| id),
            source_conversation_id: conversation,
        },
    )
    .await
    .map_err(ApiError::Internal)?
    .ok_or(ApiError::NotFound("journal target not found"))?;
    Ok(Json(row).into_response())
}
