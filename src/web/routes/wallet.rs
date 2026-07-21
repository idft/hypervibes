use std::{str::FromStr, sync::Arc};

use alloy::primitives::{Address, Signature, keccak256};
use askama::Template;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use tracing::warn;

use crate::{
    agents::store::list_agents_for_user,
    hyperliquid::orders::gateway::BUILDER_RECIPIENT,
    web::{
        AppState,
        auth::{
            AuthenticatedUser, get_user_api_wallet, record_user_api_wallet_approval,
            store_user_api_wallet,
        },
        error::AppError,
        templates::{
            SubaccountChoiceView, TradingAccountChoicesView, WalletAgentView, WalletPageTemplate,
        },
    },
};

use crate::agents::{
    crypto::encrypt,
    keys::{derive_wallet_address, generate_wallet_private_key},
};

const ZERO_ADDRESS: &str = "0x0000000000000000000000000000000000000000";
const SIGNATURE_CHAIN_ID: u64 = 42_161;
const MIN_BUILDER_FEE_BPS: i16 = 1;
const MAX_BUILDER_FEE_BPS: i16 = 10;
const DEFAULT_BUILDER_FEE_BPS: i16 = 5;
const BUILDER_FEE_BPS_TO_TENTHS: i16 = 10;

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web::routes) struct SignedAction {
    action: Value,
    signature: String,
}

#[derive(Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub(in crate::web::routes) struct ApiWalletForm {
    wallet_source: String,
    hyperliquid_private_key: String,
}

pub(in crate::web::routes) const USER_API_WALLET_NAME: &str = "Vibetrading";

#[derive(Debug, Clone)]
pub(in crate::web::routes) struct TradingAccountChoices {
    main_address: String,
    main_assigned_to: Option<String>,
    subaccounts: Vec<SubaccountChoiceView>,
    subaccount_capacity: Option<String>,
    lookup_error: Option<String>,
}

impl From<TradingAccountChoices> for TradingAccountChoicesView {
    fn from(value: TradingAccountChoices) -> Self {
        Self {
            main_address: value.main_address,
            main_assigned_to: value.main_assigned_to,
            subaccounts: value.subaccounts,
            subaccount_capacity: value.subaccount_capacity,
            lookup_error: value.lookup_error,
        }
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web::routes) struct BuilderFeeRequest {
    action: Value,
    signature: String,
    /// Builder fee in whole basis points (1–10 bps). Stored as
    /// `fee_bps * 10` in the `users.builder_fee_tenths_of_bp` column.
    fee_bps: i16,
}

#[derive(Serialize)]
pub(in crate::web::routes) struct WalletAddressResponse {
    wallet_address: String,
}

pub(in crate::web::routes) async fn wallet_address(
    user: AuthenticatedUser,
) -> Json<WalletAddressResponse> {
    Json(WalletAddressResponse {
        wallet_address: user.wallet_address,
    })
}

pub(in crate::web::routes) async fn wallet_index(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Html<String>, AppError> {
    let row: (i16, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT builder_fee_tenths_of_bp, builder_fee_approved_at FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_one(&state.db_pool)
    .await?;
    let api_wallet = get_user_api_wallet(&state.db_pool, user.id).await?;
    let wallet_address = api_wallet
        .as_ref()
        .filter(|wallet| wallet.user_id == user.id)
        .map(|wallet| wallet.main_wallet_address.clone())
        .unwrap_or(user.wallet_address);
    let agents = list_agents_for_user(&state.db_pool, user.id)
        .await?
        .into_iter()
        .map(|agent| WalletAgentView {
            agent_key: agent.agent_key,
            display_name: agent.display_name,
            lifecycle: "active".to_string(),
            trading_account_address: agent.trading_account_address,
            balance: "Balance unavailable".to_string(),
            expiry: "Expiry unavailable".to_string(),
        })
        .collect();
    let navbar = crate::web::templates::load_navbar(&state.db_pool, user.id).await?;
    let fee_bps = if row.1.is_some()
        && row.0 > 0
        && row.0 % BUILDER_FEE_BPS_TO_TENTHS == 0
    {
        (row.0 / BUILDER_FEE_BPS_TO_TENTHS)
            .clamp(MIN_BUILDER_FEE_BPS, MAX_BUILDER_FEE_BPS)
    } else {
        DEFAULT_BUILDER_FEE_BPS
    };
    Ok(Html(
        WalletPageTemplate {
            wallet_address,
            fee_bps,
            min_fee_bps: MIN_BUILDER_FEE_BPS,
            max_fee_bps: MAX_BUILDER_FEE_BPS,
            builder_fee_approved: row.1.is_some(),
            builder_recipient: BUILDER_RECIPIENT,
            current_path: "/wallet".to_string(),
            agents,
            api_wallet_address: api_wallet
                .as_ref()
                .and_then(|wallet| wallet.api_wallet_address.clone()),
            api_wallet_state: api_wallet_state(api_wallet.as_ref()),
            api_wallet_expires_at: api_wallet
                .as_ref()
                .and_then(|wallet| wallet.api_wallet_expires_at),
            api_wallet_expiry_class: api_wallet_expiry_class(
                api_wallet.as_ref().and_then(|wallet| wallet.api_wallet_expires_at),
            ),
            api_wallet_show_expired: api_wallet_show_expired(api_wallet.as_ref()),
            navbar,
        }
        .render()?,
    ))
}

/// Tailwind text color class for the API wallet expiry line, based on how
/// far the expiry is from now. > 5 months: emerald. < 1 month: amber.
/// Anything in between (or no expiry): zinc.
fn api_wallet_expiry_class(expires_at: Option<chrono::DateTime<chrono::Utc>>) -> &'static str {
    let Some(expires_at) = expires_at else {
        return "text-zinc-400";
    };
    let days = (expires_at - chrono::Utc::now()).num_days();
    if days >= 150 {
        "text-emerald-400"
    } else if days < 30 {
        "text-amber-400"
    } else {
        "text-zinc-400"
    }
}

/// True when the API key exists and is approved but its expiry is in the
/// past. The wallet page uses this to render the address with an "Expired"
/// label in red, instead of treating the key as fully missing.
fn api_wallet_show_expired(wallet: Option<&crate::web::auth::UserApiWalletRow>) -> bool {
    let Some(wallet) = wallet else { return false; };
    if wallet.api_wallet_address.is_none() || wallet.api_wallet_approved_at.is_none() {
        return false;
    }
    if let Some(expires_at) = wallet.api_wallet_expires_at {
        expires_at <= chrono::Utc::now()
    } else {
        false
    }
}

fn api_wallet_state(wallet: Option<&crate::web::auth::UserApiWalletRow>) -> String {
    let Some(wallet) = wallet else {
        return "Not set up".to_string();
    };
    if wallet.api_wallet_address.is_none() {
        "Not set up".to_string()
    } else if wallet.api_wallet_approved_at.is_none() {
        "Approval required".to_string()
    } else if !wallet.is_ready() {
        "Expired or unavailable".to_string()
    } else {
        "Approved".to_string()
    }
}

pub(in crate::web::routes) async fn approve_builder_fee(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Json(request): Json<BuilderFeeRequest>,
) -> Result<Response, AppError> {
    let fee_tenths_of_bp = request.fee_bps.checked_mul(BUILDER_FEE_BPS_TO_TENTHS);
    if !(MIN_BUILDER_FEE_BPS..=MAX_BUILDER_FEE_BPS).contains(&request.fee_bps)
        || !valid_builder_fee_action(&request.action, request.fee_bps)
        || !signature_matches(
            &request.action,
            &request.signature,
            &user.wallet_address,
            ActionKind::BuilderFee,
        )
    {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    }
    let exchange = relay(&request.action, &request.signature).await;
    if !exchange_success(&exchange) {
        return Ok((StatusCode::BAD_GATEWAY, Json(exchange)).into_response());
    }
    let Some(fee_tenths_of_bp) = fee_tenths_of_bp else {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    };
    sqlx::query("UPDATE users SET builder_fee_tenths_of_bp = $2, builder_fee_approved_at = now(), updated_at = now() WHERE id = $1")
        .bind(user.id).bind(fee_tenths_of_bp).execute(&state.db_pool).await?;
    Ok(Json(json!({"status":"ok", "redirect":"/agents/new"})).into_response())
}

pub(in crate::web::routes) async fn setup_user_api_wallet(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Json(form): Json<ApiWalletForm>,
) -> Result<Response, AppError> {
    let private_key = match form.wallet_source.as_str() {
        "generate" => generate_wallet_private_key(),
        "import" if !form.hyperliquid_private_key.trim().is_empty() => {
            form.hyperliquid_private_key.trim().to_string()
        }
        "import" => {
            return Ok(Json(json!({"error":"Private key is required."})).into_response());
        }
        _ => {
            return Ok(Json(json!({"error":"Select an API wallet option."})).into_response());
        }
    };
    let api_wallet_address = match derive_wallet_address(&private_key) {
        Ok(address) => address,
        Err(_) => {
            return Ok(Json(json!({"error":"Private key is invalid."})).into_response());
        }
    };
    let ciphertext = encrypt(&state.encryption_key, &private_key)?;
    if !store_user_api_wallet(
        &state.db_pool,
        user.id,
        &api_wallet_address,
        &ciphertext,
        &state.encryption_key.key_id,
    )
    .await?
    {
        return Ok(Json(json!({"error":"Use a fresh trading signer address."})).into_response());
    }
    Ok(Json(json!({"status":"ok","api_wallet_address":api_wallet_address})).into_response())
}

pub(in crate::web::routes) async fn load_trading_account_choices(
    state: &Arc<AppState>,
    user: &AuthenticatedUser,
) -> TradingAccountChoices {
    let main_address = user.wallet_address.to_ascii_lowercase();
    let assignments: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT trading_account_address, agent_key, display_name FROM agents
          WHERE trading_account_address IS NOT NULL AND environment = 'live'",
    )
    .fetch_all(&state.db_pool)
    .await
    .unwrap_or_default();
    let assignment_for = |address: &str| {
        assignments
            .iter()
            .find(|row| row.0.eq_ignore_ascii_case(address))
            .map(|row| crate::web::templates::SubaccountAssignmentView {
                agent_key: row.1.clone(),
                display_name: row.2.clone(),
            })
    };
    let main_assigned_to = assignment_for(&main_address).map(|assigned| assigned.display_name);
    let result = reqwest::Client::new()
        .post("https://api.hyperliquid.xyz/info")
        .json(&json!({"type":"subAccounts", "user": main_address}))
        .send()
        .await;
    let response = match result {
        Ok(response) => response.error_for_status(),
        Err(error) => {
            return TradingAccountChoices {
                main_address,
                main_assigned_to,
                subaccounts: Vec::new(),
                subaccount_capacity: None,
                lookup_error: Some(error.to_string()),
            };
        }
    };
    let value = match response {
        Ok(response) => response.json::<Value>().await,
        Err(error) => Err(error),
    };
    match value {
        Ok(value) => {
            let subaccounts: Vec<SubaccountChoiceView> = value
                .as_array()
                .map(|rows| {
                    rows.iter()
                        .filter_map(|row| {
                            let address = row
                                .get("subAccountUser")
                                .or_else(|| row.get("address"))?
                                .as_str()?
                                .to_ascii_lowercase();
                            Some(SubaccountChoiceView {
                                name: row
                                    .get("name")
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned),
                                assigned_to: assignment_for(&address),
                                address,
                                balance: row
                                    .get("clearinghouseState")
                                    .and_then(|state| state.get("marginSummary"))
                                    .and_then(|summary| summary.get("accountValue"))
                                    .and_then(Value::as_str)
                                    .map(ToOwned::to_owned),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            let subaccount_capacity =
                load_subaccount_capacity(&main_address, subaccounts.len()).await;
            TradingAccountChoices {
                main_address,
                main_assigned_to,
                subaccounts,
                subaccount_capacity,
                lookup_error: None,
            }
        }
        Err(error) => TradingAccountChoices {
            main_address,
            main_assigned_to,
            subaccounts: Vec::new(),
            subaccount_capacity: None,
            lookup_error: Some(error.to_string()),
        },
    }
}

async fn load_subaccount_capacity(owner: &str, existing_subaccounts: usize) -> Option<String> {
    let response = reqwest::Client::new()
        .post("https://api.hyperliquid.xyz/info")
        .json(&json!({"type":"userRateLimit", "user": owner}))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    let limit = subaccount_limit(&response)?;
    let remaining = limit.saturating_sub(existing_subaccounts);
    Some(format!("{remaining} remaining of {limit} total"))
}

fn subaccount_limit(response: &Value) -> Option<usize> {
    let cumulative_volume = response.get("cumVlm")?.as_str()?.parse::<f64>().ok()?;
    if cumulative_volume < 100_000.0 {
        return Some(0);
    }

    let additional_accounts = ((cumulative_volume - 100_000.0) / 100_000_000.0).floor() as usize;
    Some((10 + additional_accounts).min(50))
}

pub(in crate::web::routes) fn selected_trading_account(
    selection: &str,
    choices: &TradingAccountChoices,
) -> Result<String, &'static str> {
    if selection == "main" && choices.main_assigned_to.is_none() {
        return Ok(choices.main_address.clone());
    }

    if choices.lookup_error.is_none()
        && choices
            .subaccounts
            .iter()
            .any(|item| item.address == selection && item.assigned_to.is_none())
    {
        return Ok(selection.to_string());
    }

    Err("Select a valid trading account.")
}

pub(in crate::web::routes) fn created_subaccount_address(
    created_name: &str,
    choices: &TradingAccountChoices,
) -> Option<String> {
    choices
        .subaccounts
        .iter()
        .find(|subaccount| subaccount.name.as_deref() == Some(created_name))
        .map(|subaccount| subaccount.address.clone())
}

pub(in crate::web::routes) async fn approve_user_api_wallet(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Json(request): Json<SignedAction>,
) -> Result<Response, AppError> {
    let Some(wallet) = get_user_api_wallet(&state.db_pool, user.id).await? else {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    };
    if !valid_user_api_wallet_action(
        &request.action,
        wallet.api_wallet_address.as_deref().unwrap_or_default(),
    ) || !signature_matches(
        &request.action,
        &request.signature,
        &user.wallet_address,
        ActionKind::Agent,
    ) {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    }
    let exchange = relay(&request.action, &request.signature).await;
    if !exchange_success(&exchange) {
        return Ok((StatusCode::BAD_GATEWAY, Json(exchange)).into_response());
    }
    if !record_user_api_wallet_approval(&state.db_pool, user.id).await? {
        return Ok(StatusCode::CONFLICT.into_response());
    }
    Ok(Json(json!({"status":"ok", "redirect":"/wallet"})).into_response())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::web::routes) struct NewAgentSubaccountRequest {
    display_name: String,
}

/// Create a new Hyperliquid sub-account owned by the signed-in user. The
/// Create Agent page calls this so the new account is listed immediately and
/// can be selected for the agent. Retries reconcile the exact named account
/// instead of creating a duplicate.
pub(in crate::web::routes) async fn create_user_subaccount(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Json(request): Json<NewAgentSubaccountRequest>,
) -> Result<Response, AppError> {
    let display_name = request.display_name.trim();
    if display_name.is_empty() || crate::agents::model::slugify_agent_key(display_name).is_empty() {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "Enter an agent name first."})),
        )
            .into_response());
    }
    let expected_name = agent_subaccount_name(display_name);
    let signer = match ready_user_signer(&state, &user).await {
        Ok(signer) => signer,
        Err(response) => return Ok(response),
    };
    let (address, created) =
        if let Some(address) = discover_subaccount(&user.wallet_address, &expected_name).await {
            (address, false)
        } else {
            match relay_create_subaccount(&signer, &user.wallet_address, &expected_name).await {
                Ok(address) => (address, true),
                Err(response) => return Ok(response),
            }
        };
    Ok(
        Json(json!({"status":"ok", "name": expected_name, "address": address, "created": created}))
            .into_response(),
    )
}

/// Load and decrypt the user's ready trading signer, verifying the stored
/// address matches the derived key. The error variant is the response to
/// return to the caller.
async fn ready_user_signer(
    state: &Arc<AppState>,
    user: &AuthenticatedUser,
) -> Result<hypersdk::hypercore::PrivateKeySigner, Response> {
    let to_response = |error: anyhow::Error| AppError(error).into_response();
    let wallet = get_user_api_wallet(&state.db_pool, user.id)
        .await
        .map_err(to_response)?;
    let Some(wallet) = wallet.filter(|wallet| wallet.is_ready()) else {
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": "The user's trading signer needs attention."})),
        )
            .into_response());
    };
    let ciphertext = wallet
        .hyperliquid_private_key_ciphertext
        .ok_or_else(|| to_response(anyhow::anyhow!("approved user signer has no ciphertext")))?;
    let key_id = wallet
        .hyperliquid_private_key_key_id
        .ok_or_else(|| to_response(anyhow::anyhow!("approved user signer has no key id")))?;
    if key_id != state.encryption_key.key_id {
        return Err(to_response(anyhow::anyhow!(
            "user signer key id does not match server key"
        )));
    }
    let private_key =
        crate::agents::crypto::decrypt(&state.encryption_key, &ciphertext).map_err(to_response)?;
    let signer: hypersdk::hypercore::PrivateKeySigner = private_key
        .parse()
        .map_err(|error| to_response(anyhow::anyhow!("invalid stored user signer: {error}")))?;
    if signer.address().to_string().to_ascii_lowercase()
        != wallet.api_wallet_address.as_deref().unwrap_or_default()
    {
        return Err(to_response(anyhow::anyhow!(
            "stored user signer address does not match its database address"
        )));
    }
    Ok(signer)
}

/// Sign and relay a `createSubAccount` action, then reconcile the created
/// sub-account by its exact name. Returns the sub-account address. The error
/// variant is the response to return to the caller.
async fn relay_create_subaccount(
    signer: &hypersdk::hypercore::PrivateKeySigner,
    owner: &str,
    expected_name: &str,
) -> Result<String, Response> {
    let nonce = chrono::Utc::now().timestamp_millis() as u64;
    let (_hash, signature) =
        crate::hyperliquid::signing::sign_create_subaccount(signer, expected_name, nonce)
            .await
            .map_err(|error| AppError(error).into_response())?;
    let action = json!({"type": "createSubAccount", "name": expected_name});
    let signature =
        serde_json::to_value(signature).map_err(|error| AppError(error.into()).into_response())?;
    let exchange = relay_with_signature(&action, nonce, signature).await;
    if !exchange_success(&exchange) {
        let message = exchange_error_message(&exchange);
        warn!(response = ?exchange, "Hyperliquid rejected new Sub-Account creation");
        return Err((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": message})),
        )
            .into_response());
    }
    let Some(address) = discover_subaccount(owner, expected_name).await else {
        warn!(owner = %owner, subaccount_name = %expected_name, "Hyperliquid accepted new Sub-Account creation but it could not be reconciled");
        return Err((
            StatusCode::BAD_GATEWAY,
            Json(json!({"error": "Sub-Account creation was accepted but could not be confirmed. Refresh the account list before retrying."})),
        )
            .into_response());
    };
    Ok(address)
}

/// The Hyperliquid sub-account name Vibetrading creates for an agent.
/// Hyperliquid limits sub-account names to 16 characters, so the display
/// name is truncated to fit the `vt-` prefix that marks Vibetrading-managed
/// accounts.
pub(in crate::web::routes) fn agent_subaccount_name(display_name: &str) -> String {
    let suffix = display_name
        .trim()
        .chars()
        .take(13)
        .collect::<String>()
        .trim_end()
        .to_string();
    format!("vt-{suffix}")
}

async fn discover_subaccount(owner: &str, expected_name: &str) -> Option<String> {
    let response = reqwest::Client::new()
        .post("https://api.hyperliquid.xyz/info")
        .json(&json!({"type":"subAccounts", "user": owner}))
        .send()
        .await
        .ok()?
        .error_for_status()
        .ok()?
        .json::<Value>()
        .await
        .ok()?;
    response.as_array()?.iter().find_map(|entry| {
        (entry.get("name").and_then(Value::as_str) == Some(expected_name)).then(|| {
            entry
                .get("subAccountUser")
                .or_else(|| entry.get("address"))
                .and_then(Value::as_str)
                .map(str::to_ascii_lowercase)
        })?
    })
}

async fn relay(action: &Value, signature: &str) -> Value {
    let signature = signature_parts(signature).unwrap_or_else(|| json!({}));
    let nonce = action["nonce"].as_u64().unwrap_or_default();
    relay_with_signature(action, nonce, signature).await
}

async fn relay_with_signature(action: &Value, nonce: u64, signature: Value) -> Value {
    let response = reqwest::Client::new()
        .post("https://api.hyperliquid.xyz/exchange")
        .json(&json!({"action": action, "nonce": nonce, "signature": signature}))
        .send()
        .await;
    match response {
        Ok(response) => response.json().await.unwrap_or_else(
            |_| json!({"status":"error", "message":"Invalid Hyperliquid response"}),
        ),
        Err(_) => json!({"status":"error", "message":"Hyperliquid exchange unavailable"}),
    }
}

fn exchange_success(response: &Value) -> bool {
    response.get("status").and_then(Value::as_str) == Some("ok")
}

fn exchange_error_message(response: &Value) -> String {
    response
        .get("response")
        .and_then(Value::as_str)
        .or_else(|| response.get("message").and_then(Value::as_str))
        .unwrap_or("Hyperliquid rejected Sub-Account creation.")
        .to_string()
}
fn signature_parts(signature: &str) -> Option<Value> {
    let raw = hex::decode(signature.strip_prefix("0x")?).ok()?;
    if raw.len() != 65 {
        return None;
    }
    Some(
        json!({"r": format!("0x{}", hex::encode(&raw[..32])), "s": format!("0x{}", hex::encode(&raw[32..64])), "v": raw[64]}),
    )
}

fn valid_common(action: &Value) -> bool {
    action.get("hyperliquidChain").and_then(Value::as_str) == Some("Mainnet")
        && action.get("signatureChainId").and_then(Value::as_str) == Some("0xa4b1")
        && action
            .get("nonce")
            .and_then(Value::as_u64)
            .is_some_and(|nonce| nonce > 0)
}
fn valid_lowercase_address(value: &str) -> bool {
    Address::from_str(value).is_ok() && value == value.to_ascii_lowercase()
}
fn valid_user_api_wallet_action(action: &Value, expected_address: &str) -> bool {
    valid_common(action)
        && action.get("type").and_then(Value::as_str) == Some("approveAgent")
        && action.get("agentName").and_then(Value::as_str) == Some(USER_API_WALLET_NAME)
        && action
            .get("agentAddress")
            .and_then(Value::as_str)
            .is_some_and(|address| valid_lowercase_address(address) && address == expected_address)
}
fn valid_builder_fee_action(action: &Value, fee_bps: i16) -> bool {
    valid_common(action)
        && action.get("type").and_then(Value::as_str) == Some("approveBuilderFee")
        && action.get("builder").and_then(Value::as_str) == Some(BUILDER_RECIPIENT)
        && action.get("maxFeeRate").and_then(Value::as_str)
            == Some(&format!("{:.2}%", fee_bps as f64 / 100.0))
}

#[derive(Clone, Copy)]
enum ActionKind {
    Agent,
    BuilderFee,
}
fn signature_matches(action: &Value, signature: &str, expected: &str, kind: ActionKind) -> bool {
    let Some(digest) = typed_action_hash(action, kind) else {
        return false;
    };
    Signature::from_str(signature)
        .ok()
        .and_then(|signature| signature.recover_address_from_prehash(&digest).ok())
        .map(|address| address.to_string().to_ascii_lowercase() == expected)
        .unwrap_or(false)
}
fn typed_action_hash(action: &Value, kind: ActionKind) -> Option<alloy::primitives::B256> {
    let chain = action.get("hyperliquidChain")?.as_str()?;
    let nonce = action.get("nonce")?.as_u64()?;
    let type_hash = match kind {
        ActionKind::Agent => keccak256(
            "HyperliquidTransaction:ApproveAgent(string hyperliquidChain,address agentAddress,string agentName,uint64 nonce)",
        ),
        ActionKind::BuilderFee => keccak256(
            "HyperliquidTransaction:ApproveBuilderFee(string hyperliquidChain,string maxFeeRate,address builder,uint64 nonce)",
        ),
    };
    let mut encoded = Vec::with_capacity(128);
    encoded.extend_from_slice(type_hash.as_slice());
    encoded.extend_from_slice(keccak256(chain).as_slice());
    match kind {
        ActionKind::Agent => {
            encoded.extend_from_slice(&address_word(action.get("agentAddress")?.as_str()?));
            encoded.extend_from_slice(keccak256(action.get("agentName")?.as_str()?).as_slice());
        }
        ActionKind::BuilderFee => {
            encoded.extend_from_slice(keccak256(action.get("maxFeeRate")?.as_str()?).as_slice());
            encoded.extend_from_slice(&address_word(action.get("builder")?.as_str()?));
        }
    }
    encoded.extend_from_slice(&uint_word(nonce));
    let domain_type = keccak256(
        "EIP712Domain(string name,string version,uint256 chainId,address verifyingContract)",
    );
    let mut domain = Vec::new();
    domain.extend_from_slice(domain_type.as_slice());
    domain.extend_from_slice(keccak256("HyperliquidSignTransaction").as_slice());
    domain.extend_from_slice(keccak256("1").as_slice());
    domain.extend_from_slice(&uint_word(SIGNATURE_CHAIN_ID));
    domain.extend_from_slice(&address_word(ZERO_ADDRESS));
    Some(keccak256(
        [
            b"\x19\x01".as_slice(),
            keccak256(domain).as_slice(),
            keccak256(encoded).as_slice(),
        ]
        .concat(),
    ))
}
fn address_word(value: &str) -> [u8; 32] {
    let mut word = [0; 32];
    if let Ok(address) = Address::from_str(value) {
        word[12..].copy_from_slice(address.as_slice());
    }
    word
}
fn uint_word(value: u64) -> [u8; 32] {
    let mut word = [0; 32];
    word[24..].copy_from_slice(&value.to_be_bytes());
    word
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::signers::{Signer, local::PrivateKeySigner};
    #[tokio::test]
    async fn validates_official_approve_agent_typed_signature() {
        let signer = PrivateKeySigner::from_str(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("signer");
        let action = json!({"type":"approveAgent","hyperliquidChain":"Mainnet","signatureChainId":"0xa4b1","agentAddress":"0x0000000000000000000000000000000000000001","agentName":"Vibetrading","nonce":1});
        let signature = signer
            .sign_hash(&typed_action_hash(&action, ActionKind::Agent).expect("hash"))
            .await
            .expect("sign");
        assert!(valid_user_api_wallet_action(
            &action,
            "0x0000000000000000000000000000000000000001"
        ));
        assert!(signature_matches(
            &action,
            &signature.to_string(),
            &signer.address().to_string().to_ascii_lowercase(),
            ActionKind::Agent
        ));
    }

    #[test]
    fn selected_trading_account_rejects_assigned_accounts() {
        let subaccount = |address: &str, assigned_to: Option<&str>| SubaccountChoiceView {
            name: None,
            address: address.to_string(),
            balance: None,
            assigned_to: assigned_to.map(|agent_key| {
                crate::web::templates::SubaccountAssignmentView {
                    agent_key: agent_key.to_string(),
                    display_name: "Other Agent".to_string(),
                }
            }),
        };
        let choices = TradingAccountChoices {
            main_address: "0xmain".to_string(),
            main_assigned_to: None,
            subaccounts: vec![
                subaccount("0xfree", None),
                subaccount("0xused", Some("other-agent")),
            ],
            subaccount_capacity: None,
            lookup_error: None,
        };
        assert_eq!(
            selected_trading_account("main", &choices).expect("main selectable"),
            "0xmain"
        );
        assert_eq!(
            selected_trading_account("0xfree", &choices).expect("free sub-account selectable"),
            "0xfree"
        );
        assert!(selected_trading_account("0xused", &choices).is_err());
        assert!(selected_trading_account("0xunknown", &choices).is_err());

        let main_assigned = TradingAccountChoices {
            main_assigned_to: Some("Other Agent".to_string()),
            ..choices
        };
        assert!(selected_trading_account("main", &main_assigned).is_err());
    }

    #[test]
    fn subaccount_name_fits_hyperliquid_length_limit() {
        assert_eq!(agent_subaccount_name("BTC"), "vt-BTC");
        assert_eq!(agent_subaccount_name("  BTC Momentum  "), "vt-BTC Momentum");
        let truncated = agent_subaccount_name("A Very Long Agent Display Name");
        assert_eq!(truncated, "vt-A Very Long A");
        assert!(truncated.chars().count() <= 16);
    }

    #[test]
    fn calculates_subaccount_limit_from_reported_cumulative_volume() {
        assert_eq!(subaccount_limit(&json!({"cumVlm":"99999.99"})), Some(0));
        assert_eq!(subaccount_limit(&json!({"cumVlm":"100000"})), Some(10));
        assert_eq!(subaccount_limit(&json!({"cumVlm":"100100000"})), Some(11));
        assert_eq!(
            subaccount_limit(&json!({"cumVlm":"100000000000"})),
            Some(50)
        );
    }

    #[test]
    fn extracts_exchange_rejection_message() {
        assert_eq!(
            exchange_error_message(&json!({"status":"err","response":"Need to deposit"})),
            "Need to deposit"
        );
        assert_eq!(
            exchange_error_message(&json!({"status":"error"})),
            "Hyperliquid rejected Sub-Account creation."
        );
    }
}
