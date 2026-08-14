use std::{
    collections::{HashMap, HashSet},
    str::FromStr,
    sync::Arc,
};

use alloy::primitives::{Address, Signature, keccak256};
use askama::Template;
use axum::{
    Json,
    extract::State,
    http::StatusCode,
    response::{Html, IntoResponse, Response},
};
use rust_decimal::Decimal;
use serde::{Deserialize, de::DeserializeOwned};
use serde_json::{Value, json};
use tracing::warn;

use crate::{
    agents::store::list_agents_for_user,
    hyperliquid::builder_fee::{BUILDER_RECIPIENT, MAX_BUILDER_FEE_TENTHS_OF_BP},
    web::{
        AppState,
        auth::{
            AuthenticatedUser, get_user_api_wallet, record_user_api_wallet_approval,
            store_user_api_wallet,
        },
        error::AppError,
        templates::{
            AccountPageTemplate, AccountRowView, AccountTableBalanceView, SubaccountChoiceView,
            TradingAccountChoicesView,
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
const REVOKED_BUILDER_FEE_RATE: &str = "0.00%";
/// Hyperliquid's canonical mainnet USDC token from the `spotMeta` response.
const PERPETUAL_USDC_TOKEN: &str = "USDC:0x6d1e7cde53ba9467b783cb7c530ce054";
const UNIFIED_ACCOUNT_DEX: &str = "spot";
const MAX_TRANSFER_DECIMAL_PLACES: u32 = 8;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
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

pub(in crate::web::routes) const USER_API_WALLET_NAME: &str = "HyperVibes";

#[derive(Debug, Clone)]
pub(in crate::web::routes) struct TradingAccountChoices {
    main_address: String,
    main_balance: Option<String>,
    main_assigned_to: Option<String>,
    subaccounts: Vec<SubaccountChoiceView>,
    subaccount_capacity: Option<String>,
    lookup_error: Option<String>,
    account_mode_supported: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotClearinghouseStateResponse {
    #[serde(default)]
    balances: Vec<SpotBalance>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotState {
    #[serde(default)]
    balances: Vec<SpotBalance>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SpotBalance {
    coin: String,
    total: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
struct SubaccountResponse {
    #[serde(alias = "address")]
    sub_account_user: Option<String>,
    name: Option<String>,
    spot_state: Option<SpotState>,
}

#[derive(Debug, Clone)]
struct OwnedSubaccount {
    name: Option<String>,
    address: String,
    balance: Option<Decimal>,
}

#[derive(Debug, Clone)]
struct OwnedTradingAccounts {
    main_address: String,
    main_balance: Option<Decimal>,
    subaccounts: Vec<OwnedSubaccount>,
    subaccount_capacity: Option<String>,
    lookup_error: Option<String>,
    account_mode: AccountMode,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum AccountMode {
    Unified,
    Unsupported,
    Unavailable,
}

fn parse_spot_usdc_balance(balances: &[SpotBalance]) -> Option<Decimal> {
    let Some(balance) = balances
        .iter()
        .find(|balance| balance.coin.eq_ignore_ascii_case("USDC"))
    else {
        return Some(Decimal::ZERO);
    };
    let total = balance.total.as_deref()?;
    let value: Decimal = total.parse().ok()?;
    (!value.is_sign_negative()).then_some(value)
}

fn parse_subaccounts(response: Vec<SubaccountResponse>) -> Vec<OwnedSubaccount> {
    response
        .into_iter()
        .filter_map(|subaccount| {
            let address = subaccount.sub_account_user?.to_ascii_lowercase();
            Address::from_str(&address).ok()?;
            Some(OwnedSubaccount {
                name: subaccount.name.filter(|name| !name.trim().is_empty()),
                address,
                balance: subaccount
                    .spot_state
                    .as_ref()
                    .map(|state| parse_spot_usdc_balance(&state.balances))
                    .unwrap_or(Some(Decimal::ZERO)),
            })
        })
        .collect()
}

async fn post_info<T: DeserializeOwned>(payload: Value) -> Result<T, String> {
    reqwest::Client::new()
        .post("https://api.hyperliquid.xyz/info")
        .json(&payload)
        .send()
        .await
        .map_err(|_| "Hyperliquid account data is temporarily unavailable.".to_string())?
        .error_for_status()
        .map_err(|_| "Hyperliquid account data is temporarily unavailable.".to_string())?
        .json()
        .await
        .map_err(|_| "Hyperliquid account data is temporarily unavailable.".to_string())
}

async fn load_owned_trading_accounts(
    owner: &str,
    include_capacity: bool,
    check_mode: bool,
) -> OwnedTradingAccounts {
    let main_address = owner.to_ascii_lowercase();
    let abstraction_request = async {
        if check_mode {
            post_info::<String>(json!({"type": "userAbstraction", "user": main_address})).await
        } else {
            Ok("unifiedAccount".to_string())
        }
    };
    let (main_result, subaccounts_result, abstraction_result) = tokio::join!(
        post_info::<SpotClearinghouseStateResponse>(
            json!({"type": "spotClearinghouseState", "user": main_address}),
        ),
        post_info::<Vec<SubaccountResponse>>(json!({"type": "subAccounts", "user": main_address}),),
        abstraction_request,
    );

    let main_balance = main_result
        .ok()
        .and_then(|response| parse_spot_usdc_balance(&response.balances));
    let main_lookup_failed = main_balance.is_none();
    let (subaccounts, subaccount_lookup_error) = match subaccounts_result {
        Ok(response) => (parse_subaccounts(response), None),
        Err(error) => (Vec::new(), Some(error)),
    };
    let lookup_error = if main_lookup_failed {
        Some("Hyperliquid account data is temporarily unavailable.".to_string())
    } else {
        subaccount_lookup_error
    };
    let account_mode = match abstraction_result {
        Ok(mode) if mode == "unifiedAccount" => AccountMode::Unified,
        Ok(_) => AccountMode::Unsupported,
        Err(_) => AccountMode::Unavailable,
    };
    let subaccount_capacity = if include_capacity {
        load_subaccount_capacity(&main_address, subaccounts.len()).await
    } else {
        None
    };
    OwnedTradingAccounts {
        main_address,
        main_balance,
        subaccounts,
        subaccount_capacity,
        lookup_error,
        account_mode,
    }
}

fn account_rows(
    accounts: &OwnedTradingAccounts,
    assignments: &HashMap<String, (String, String)>,
) -> Vec<AccountRowView> {
    let row = |name: String, address: String, balance: Option<Decimal>| {
        let assignment = assignments.get(&address);
        let balance_view = account_balance_view(balance);
        AccountRowView {
            name,
            address: address.clone(),
            agent_key: assignment.map(|value| value.0.clone()),
            agent_display_name: assignment.map(|value| value.1.clone()),
            balance: balance_view.formatted.clone(),
            balance_view,
            transfer_value: address,
        }
    };
    let mut rows = vec![row(
        "Main Account".to_string(),
        accounts.main_address.clone(),
        accounts.main_balance,
    )];
    rows.extend(accounts.subaccounts.iter().map(|subaccount| {
        row(
            subaccount
                .name
                .clone()
                .unwrap_or_else(|| "Unnamed subaccount".to_string()),
            subaccount.address.clone(),
            subaccount.balance,
        )
    }));
    rows
}

fn account_balance_view(balance: Option<Decimal>) -> AccountTableBalanceView {
    let Some(balance) = balance else {
        return AccountTableBalanceView {
            formatted: "Unavailable".to_string(),
            whole: "Unavailable".to_string(),
            decimals: None,
            is_zero: false,
        };
    };
    let formatted = crate::web::templates::shared::format_money_text(Some(balance));
    let (whole, decimals) = match formatted.split_once('.') {
        Some((whole, decimals)) => (whole.to_string(), Some(decimals.to_string())),
        None => (formatted.clone(), None),
    };
    AccountTableBalanceView {
        formatted,
        whole,
        decimals,
        is_zero: balance.is_zero(),
    }
}

fn total_account_balance_value(accounts: &OwnedTradingAccounts) -> Option<Decimal> {
    std::iter::once(accounts.main_balance)
        .chain(
            accounts
                .subaccounts
                .iter()
                .map(|subaccount| subaccount.balance),
        )
        .try_fold(Decimal::ZERO, |total, balance| {
            balance.map(|balance| total + balance)
        })
}

fn account_mode_error(mode: AccountMode) -> Option<String> {
    (mode != AccountMode::Unified).then(|| account_mode_message(mode).to_string())
}

fn account_mode_message(mode: AccountMode) -> &'static str {
    match mode {
        AccountMode::Unified => "",
        AccountMode::Unsupported => {
            "HyperVibes currently supports Unified Accounts only. Enable Unified Account in Hyperliquid before transferring or creating agents."
        }
        AccountMode::Unavailable => {
            "Hyperliquid account mode could not be verified. Transfers are temporarily unavailable."
        }
    }
}

fn canonical_transfer_amount(value: &str) -> Option<String> {
    if value.is_empty() || value.trim() != value || value.contains('e') || value.contains('E') {
        return None;
    }
    let (integer, fraction) = value.split_once('.').unwrap_or((value, ""));
    if integer.is_empty() || !integer.chars().all(|character| character.is_ascii_digit()) {
        return None;
    }
    if (value.contains('.') && fraction.is_empty())
        || (!fraction.is_empty() && !fraction.chars().all(|character| character.is_ascii_digit()))
    {
        return None;
    }
    let amount = Decimal::from_str(value).ok()?;
    if amount.is_zero() || amount.is_sign_negative() || amount.scale() > MAX_TRANSFER_DECIMAL_PLACES
    {
        return None;
    }
    Some(amount.normalize().to_string())
}

fn valid_send_asset_action(
    action: &Value,
    main_address: &str,
    owned_addresses: &HashSet<String>,
) -> bool {
    let Some(action_object) = action.as_object() else {
        return false;
    };
    let expected_fields = [
        "type",
        "hyperliquidChain",
        "signatureChainId",
        "destination",
        "sourceDex",
        "destinationDex",
        "token",
        "amount",
        "fromSubAccount",
        "nonce",
    ];
    if action_object.len() != expected_fields.len()
        || expected_fields
            .iter()
            .any(|field| !action_object.contains_key(*field))
    {
        return false;
    }
    if action.get("type").and_then(Value::as_str) != Some("sendAsset")
        || !valid_common(action)
        || action.get("sourceDex").and_then(Value::as_str) != Some(UNIFIED_ACCOUNT_DEX)
        || action.get("destinationDex").and_then(Value::as_str) != Some(UNIFIED_ACCOUNT_DEX)
        || action.get("token").and_then(Value::as_str) != Some(PERPETUAL_USDC_TOKEN)
    {
        return false;
    }
    let Some(destination) = action.get("destination").and_then(Value::as_str) else {
        return false;
    };
    let Some(from_subaccount) = action.get("fromSubAccount").and_then(Value::as_str) else {
        return false;
    };
    let Some(amount) = action.get("amount").and_then(Value::as_str) else {
        return false;
    };
    if !valid_lowercase_address(destination)
        || !owned_addresses.contains(destination)
        || canonical_transfer_amount(amount).as_deref() != Some(amount)
    {
        return false;
    }
    let source = if from_subaccount.is_empty() {
        main_address
    } else {
        if !valid_lowercase_address(from_subaccount)
            || from_subaccount == main_address
            || !owned_addresses.contains(from_subaccount)
        {
            return false;
        }
        from_subaccount
    };
    source != destination
}

async fn relay_transfer(action: &Value, signature: &str) -> Result<Value, ()> {
    let signature = signature_parts(signature).ok_or(())?;
    reqwest::Client::new()
        .post("https://api.hyperliquid.xyz/exchange")
        .json(&json!({
            "action": action,
            "nonce": action.get("nonce").and_then(Value::as_u64).ok_or(())?,
            "signature": signature,
        }))
        .send()
        .await
        .map_err(|_| ())?
        .error_for_status()
        .map_err(|_| ())?
        .json()
        .await
        .map_err(|_| ())
}

pub(in crate::web::routes) async fn transfer_between_accounts(
    State(_state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Json(request): Json<SignedAction>,
) -> Result<Response, AppError> {
    let accounts = load_owned_trading_accounts(&user.wallet_address, false, true).await;
    let owned_addresses: HashSet<String> = std::iter::once(accounts.main_address.clone())
        .chain(
            accounts
                .subaccounts
                .iter()
                .map(|subaccount| subaccount.address.clone()),
        )
        .collect();
    match accounts.account_mode {
        AccountMode::Unified => {}
        AccountMode::Unsupported => {
            return Ok((
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(json!({"error": account_mode_message(accounts.account_mode)})),
            )
                .into_response());
        }
        AccountMode::Unavailable => {
            return Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({"error": account_mode_message(accounts.account_mode)})),
            )
                .into_response());
        }
    }
    if !valid_send_asset_action(&request.action, &accounts.main_address, &owned_addresses)
        || !signature_matches(
            &request.action,
            &request.signature,
            &accounts.main_address,
            ActionKind::SendAsset,
        )
    {
        return Ok(StatusCode::BAD_REQUEST.into_response());
    }
    let exchange = match relay_transfer(&request.action, &request.signature).await {
        Ok(response) => response,
        Err(()) => {
            return Ok((
                StatusCode::BAD_GATEWAY,
                Json(json!({"message": "Hyperliquid exchange unavailable."})),
            )
                .into_response());
        }
    };
    if !exchange_success(&exchange) {
        return Ok((
            StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": transfer_error_message(&exchange)})),
        )
            .into_response());
    }
    Ok(Json(json!({"status": "ok"})).into_response())
}

impl From<TradingAccountChoices> for TradingAccountChoicesView {
    fn from(value: TradingAccountChoices) -> Self {
        Self {
            main_address: value.main_address,
            main_balance: value.main_balance,
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

pub(in crate::web::routes) async fn account_index(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
) -> Result<Html<String>, AppError> {
    let mut row: (i16, Option<chrono::DateTime<chrono::Utc>>) = sqlx::query_as(
        "SELECT builder_fee_tenths_of_bp, builder_fee_approved_at FROM users WHERE id = $1",
    )
    .bind(user.id)
    .fetch_one(&state.db_pool)
    .await?;
    let builder_fee_approved = match state
        .builder_fee_cache
        .max_builder_fee(&user.wallet_address, BUILDER_RECIPIENT)
        .await
    {
        Ok(remote_max) => {
            let covered = u32::try_from(row.0)
                .ok()
                .is_some_and(|saved_fee| saved_fee > 0 && remote_max >= saved_fee);
            if let Some(remote_fee) = remote_max
                .min(MAX_BUILDER_FEE_TENTHS_OF_BP)
                .try_into()
                .ok()
                .filter(|fee: &i16| *fee > 0)
                && row.0 != remote_fee
            {
                sqlx::query(
                    "UPDATE users SET builder_fee_tenths_of_bp = $2, updated_at = now()
                       WHERE id = $1",
                )
                .bind(user.id)
                .bind(remote_fee)
                .execute(&state.db_pool)
                .await?;
                row.0 = remote_fee;
            }
            if covered && row.1.is_none() {
                sqlx::query(
                    "UPDATE users SET builder_fee_approved_at = now(), updated_at = now()
                       WHERE id = $1 AND builder_fee_approved_at IS NULL",
                )
                .bind(user.id)
                .execute(&state.db_pool)
                .await?;
                row.1 = Some(chrono::Utc::now());
            } else if !covered && row.1.is_some() {
                sqlx::query(
                    "UPDATE users SET builder_fee_approved_at = NULL, updated_at = now()
                       WHERE id = $1 AND builder_fee_approved_at IS NOT NULL",
                )
                .bind(user.id)
                .execute(&state.db_pool)
                .await?;
                row.1 = None;
            }
            covered
        }
        Err(error) => {
            warn!(user_id = %user.id, error = %error, "builder fee approval lookup failed");
            row.1.is_some()
        }
    };
    let api_wallet = get_user_api_wallet(&state.db_pool, user.id).await?;
    let wallet_address = api_wallet
        .as_ref()
        .filter(|wallet| wallet.user_id == user.id)
        .map(|wallet| wallet.main_wallet_address.clone())
        .unwrap_or_else(|| user.wallet_address.clone());
    let agents = list_agents_for_user(&state.db_pool, user.id).await?;
    let assignments: HashMap<String, (String, String)> = agents
        .iter()
        .map(|agent| {
            (
                agent.trading_account_address.to_ascii_lowercase(),
                (agent.agent_key.clone(), agent.display_name.clone()),
            )
        })
        .collect();
    let owned_accounts = load_owned_trading_accounts(&user.wallet_address, false, true).await;
    let accounts = account_rows(&owned_accounts, &assignments);
    let total_balance = (accounts.len() > 1)
        .then(|| account_balance_view(total_account_balance_value(&owned_accounts)));
    let transfers_enabled =
        owned_accounts.account_mode == AccountMode::Unified && accounts.len() >= 2;
    let navbar = crate::web::templates::load_navbar(&state.db_pool, user.id).await?;
    let fee_bps = if row.0 > 0 && row.0 % BUILDER_FEE_BPS_TO_TENTHS == 0 {
        (row.0 / BUILDER_FEE_BPS_TO_TENTHS).clamp(MIN_BUILDER_FEE_BPS, MAX_BUILDER_FEE_BPS)
    } else {
        DEFAULT_BUILDER_FEE_BPS
    };
    Ok(Html(
        AccountPageTemplate {
            wallet_address,
            fee_bps,
            min_fee_bps: MIN_BUILDER_FEE_BPS,
            max_fee_bps: MAX_BUILDER_FEE_BPS,
            builder_recipient: BUILDER_RECIPIENT,
            current_path: "/account".to_string(),
            accounts,
            total_balance,
            account_lookup_error: owned_accounts.lookup_error,
            account_mode_error: account_mode_error(owned_accounts.account_mode),
            transfers_enabled,
            api_wallet_address: api_wallet
                .as_ref()
                .and_then(|wallet| wallet.api_wallet_address.clone()),
            api_wallet_state: api_wallet_state(api_wallet.as_ref()),
            api_wallet_expires_at: api_wallet
                .as_ref()
                .and_then(|wallet| wallet.api_wallet_expires_at),
            api_wallet_expiry_class: api_wallet_expiry_class(
                api_wallet
                    .as_ref()
                    .and_then(|wallet| wallet.api_wallet_expires_at),
            ),
            api_wallet_show_expired: api_wallet_show_expired(api_wallet.as_ref()),
            builder_fee_approved,
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
    let Some(wallet) = wallet else {
        return false;
    };
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
        || !valid_builder_fee_action(
            &request.action,
            &format!("{:.2}%", request.fee_bps as f64 / 100.0),
        )
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
    state
        .builder_fee_cache
        .record_approval(
            &user.wallet_address,
            BUILDER_RECIPIENT,
            u32::try_from(fee_tenths_of_bp).expect("validated builder fee fits u32"),
        )
        .await;
    Ok(Json(json!({"status":"ok", "redirect":"/account"})).into_response())
}

pub(in crate::web::routes) async fn cancel_builder_fee(
    State(state): State<Arc<AppState>>,
    user: AuthenticatedUser,
    Json(request): Json<SignedAction>,
) -> Result<Response, AppError> {
    if !valid_builder_fee_action(&request.action, REVOKED_BUILDER_FEE_RATE)
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
    sqlx::query(
        "UPDATE users SET builder_fee_approved_at = NULL, updated_at = now() WHERE id = $1",
    )
    .bind(user.id)
    .execute(&state.db_pool)
    .await?;
    state
        .builder_fee_cache
        .record_revocation(&user.wallet_address, BUILDER_RECIPIENT)
        .await;
    Ok(Json(json!({"status":"ok", "redirect":"/account"})).into_response())
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
    let accounts = load_owned_trading_accounts(&main_address, true, !cfg!(test)).await;
    let subaccounts = accounts
        .subaccounts
        .iter()
        .map(|subaccount| SubaccountChoiceView {
            name: subaccount.name.clone(),
            assigned_to: assignment_for(&subaccount.address),
            address: subaccount.address.clone(),
            balance: subaccount
                .balance
                .map(|value| crate::web::templates::shared::format_money_text(Some(value))),
        })
        .collect();
    TradingAccountChoices {
        main_address,
        main_balance: accounts
            .main_balance
            .map(|value| crate::web::templates::shared::format_money_text(Some(value))),
        main_assigned_to,
        subaccounts,
        subaccount_capacity: accounts.subaccount_capacity,
        lookup_error: accounts
            .lookup_error
            .or_else(|| account_mode_error(accounts.account_mode)),
        account_mode_supported: accounts.account_mode == AccountMode::Unified,
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
    if !choices.account_mode_supported {
        return Err("HyperVibes currently supports Unified Accounts only.");
    }
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
    Ok(Json(json!({"status":"ok", "redirect":"/account"})).into_response())
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

/// The Hyperliquid sub-account name HyperVibes creates for an agent.
/// Hyperliquid limits sub-account names to 16 characters, so the display
/// name is truncated to fit the `vt-` prefix that marks HyperVibes-managed
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

fn transfer_error_message(response: &Value) -> String {
    response
        .get("response")
        .and_then(Value::as_str)
        .or_else(|| response.get("message").and_then(Value::as_str))
        .unwrap_or("Hyperliquid rejected the transfer.")
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
fn valid_builder_fee_action(action: &Value, max_fee_rate: &str) -> bool {
    valid_common(action)
        && action.get("type").and_then(Value::as_str) == Some("approveBuilderFee")
        && action.get("builder").and_then(Value::as_str) == Some(BUILDER_RECIPIENT)
        && action.get("maxFeeRate").and_then(Value::as_str) == Some(max_fee_rate)
}

#[derive(Clone, Copy)]
enum ActionKind {
    Agent,
    BuilderFee,
    SendAsset,
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
        ActionKind::SendAsset => keccak256(
            "HyperliquidTransaction:SendAsset(string hyperliquidChain,string destination,string sourceDex,string destinationDex,string token,string amount,string fromSubAccount,uint64 nonce)",
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
        ActionKind::SendAsset => {
            for field in [
                "destination",
                "sourceDex",
                "destinationDex",
                "token",
                "amount",
                "fromSubAccount",
            ] {
                encoded.extend_from_slice(keccak256(action.get(field)?.as_str()?).as_slice());
            }
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
        let action = json!({"type":"approveAgent","hyperliquidChain":"Mainnet","signatureChainId":"0xa4b1","agentAddress":"0x0000000000000000000000000000000000000001","agentName":"HyperVibes","nonce":1});
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
    fn validates_builder_fee_revocation_rate() {
        let action = json!({
            "type": "approveBuilderFee",
            "hyperliquidChain": "Mainnet",
            "signatureChainId": "0xa4b1",
            "maxFeeRate": REVOKED_BUILDER_FEE_RATE,
            "builder": BUILDER_RECIPIENT,
            "nonce": 1,
        });
        assert!(valid_builder_fee_action(&action, REVOKED_BUILDER_FEE_RATE));
        assert!(!valid_builder_fee_action(&action, "0"));
    }

    #[test]
    fn parses_unified_spot_balances_without_fabricating_values() {
        let response: SpotClearinghouseStateResponse = serde_json::from_value(json!({
            "balances": [{"coin": "USDC", "total": "12.50"}]
        }))
        .expect("spot clearinghouse response");
        assert_eq!(
            parse_spot_usdc_balance(&response.balances),
            Some(Decimal::new(1250, 2))
        );

        assert_eq!(parse_spot_usdc_balance(&[]), Some(Decimal::ZERO));
        assert_eq!(
            parse_spot_usdc_balance(&[SpotBalance {
                coin: "USDT".to_string(),
                total: Some("12.50".to_string()),
            }]),
            Some(Decimal::ZERO)
        );
        for balances in [
            vec![SpotBalance {
                coin: "USDC".to_string(),
                total: Some("not-a-number".to_string()),
            }],
            vec![SpotBalance {
                coin: "USDC".to_string(),
                total: Some("-1".to_string()),
            }],
        ] {
            assert_eq!(parse_spot_usdc_balance(&balances), None);
        }
    }

    #[test]
    fn parses_named_and_unnamed_subaccounts_in_exchange_order() {
        let response: Vec<SubaccountResponse> = serde_json::from_value(json!([
            {
                "name": "Alpha",
                "subAccountUser": "0x2222222222222222222222222222222222222222",
                "spotState": {"balances": [{"coin": "USDC", "total": "3.5"}]}
            },
            {
                "subAccountUser": "0x3333333333333333333333333333333333333333",
                "spotState": {"balances": [{"coin": "USDC", "total": "bad"}]}
            },
            {
                "subAccountUser": "0x4444444444444444444444444444444444444444"
            }
        ]))
        .expect("subaccount response");
        let subaccounts = parse_subaccounts(response);
        assert_eq!(subaccounts.len(), 3);
        assert_eq!(subaccounts[0].name.as_deref(), Some("Alpha"));
        assert_eq!(subaccounts[0].balance, Some(Decimal::new(35, 1)));
        assert!(subaccounts[1].name.is_none());
        assert_eq!(subaccounts[1].balance, None);
        assert_eq!(subaccounts[2].balance, Some(Decimal::ZERO));
    }

    #[test]
    fn account_rows_keep_main_first_and_only_use_supplied_assignments() {
        let main = "0x1111111111111111111111111111111111111111".to_string();
        let named = "0x2222222222222222222222222222222222222222".to_string();
        let unnamed = "0x3333333333333333333333333333333333333333".to_string();
        let mut accounts = OwnedTradingAccounts {
            main_address: main.clone(),
            main_balance: Some(Decimal::new(100, 0)),
            subaccounts: vec![
                OwnedSubaccount {
                    name: Some("Trading subaccount".to_string()),
                    address: named.clone(),
                    balance: None,
                },
                OwnedSubaccount {
                    name: None,
                    address: unnamed.clone(),
                    balance: Some(Decimal::new(2, 0)),
                },
            ],
            subaccount_capacity: None,
            lookup_error: None,
            account_mode: AccountMode::Unified,
        };
        let assignments = HashMap::from([(
            named.clone(),
            ("named-agent".to_string(), "Named Agent".to_string()),
        )]);

        let rows = account_rows(&accounts, &assignments);
        assert_eq!(rows[0].name, "Main Account");
        assert_eq!(rows[0].address, main);
        assert_eq!(rows[0].balance, "100.0000");
        assert_eq!(rows[1].name, "Trading subaccount");
        assert_eq!(rows[1].agent_key.as_deref(), Some("named-agent"));
        assert_eq!(rows[1].balance, "Unavailable");
        assert_eq!(rows[2].name, "Unnamed subaccount");
        assert!(rows[2].agent_key.is_none());
        assert_eq!(rows[2].balance, "2.0000");
        assert_eq!(total_account_balance_value(&accounts), None);
        accounts.subaccounts[0].balance = Some(Decimal::ZERO);
        assert_eq!(
            account_balance_view(total_account_balance_value(&accounts)).formatted,
            "102.0000"
        );
    }

    #[tokio::test]
    async fn send_asset_signature_and_account_directions_are_valid() {
        let signer = PrivateKeySigner::from_str(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("signer");
        let main = signer.address().to_string().to_ascii_lowercase();
        let first = "0x1111111111111111111111111111111111111111";
        let second = "0x2222222222222222222222222222222222222222";
        let owned = HashSet::from([main.clone(), first.to_string(), second.to_string()]);

        for (source, destination) in [("", first), (first, main.as_str()), (first, second)] {
            let action = json!({
                "type": "sendAsset",
                "hyperliquidChain": "Mainnet",
                "signatureChainId": "0xa4b1",
                "destination": destination,
                "sourceDex": "spot",
                "destinationDex": "spot",
                "token": PERPETUAL_USDC_TOKEN,
                "amount": "1.25",
                "fromSubAccount": source,
                "nonce": 42,
            });
            let signature = signer
                .sign_hash(&typed_action_hash(&action, ActionKind::SendAsset).expect("hash"))
                .await
                .expect("sign");
            assert!(valid_send_asset_action(&action, &main, &owned));
            assert!(signature_matches(
                &action,
                &signature.to_string(),
                &main,
                ActionKind::SendAsset
            ));
        }
    }

    #[test]
    fn send_asset_validation_rejects_unsafe_fields_and_amounts() {
        let main = "0x1111111111111111111111111111111111111111";
        let sub = "0x2222222222222222222222222222222222222222";
        let owned = HashSet::from([main.to_string(), sub.to_string()]);
        let mut action = json!({
            "type": "sendAsset",
            "hyperliquidChain": "Mainnet",
            "signatureChainId": "0xa4b1",
            "destination": sub,
            "sourceDex": "spot",
            "destinationDex": "spot",
            "token": PERPETUAL_USDC_TOKEN,
            "amount": "1.25",
            "fromSubAccount": "",
            "nonce": 42,
        });
        assert!(valid_send_asset_action(&action, main, &owned));
        for amount in ["0", "-1", "1.", "1e-2", "1.123456789", "01.25", "1.250"] {
            action["amount"] = json!(amount);
            assert!(!valid_send_asset_action(&action, main, &owned));
        }
        action["amount"] = json!("1.25");
        action["fromSubAccount"] = json!(main);
        assert!(!valid_send_asset_action(&action, main, &owned));
        action["fromSubAccount"] = json!(sub);
        action["destination"] = json!(sub);
        assert!(!valid_send_asset_action(&action, main, &owned));
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
            main_balance: None,
            main_assigned_to: None,
            subaccounts: vec![
                subaccount("0xfree", None),
                subaccount("0xused", Some("other-agent")),
            ],
            subaccount_capacity: None,
            lookup_error: None,
            account_mode_supported: true,
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
