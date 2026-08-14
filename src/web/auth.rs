use std::{str::FromStr, sync::Arc};

use alloy::primitives::{Address, Signature, keccak256};
use axum::{
    body::{Body, to_bytes},
    extract::{FromRef, FromRequestParts, State},
    http::{HeaderMap, HeaderValue, StatusCode, request::Parts},
    middleware::Next,
    response::{Html, IntoResponse, Redirect, Response},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use chrono::{DateTime, Duration, Utc};
use rand::RngExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::FromRow;
use uuid::Uuid;

use crate::{agents::store::agent_belongs_to_user, db::DbPool, web::AppState};

const SESSION_COOKIE: &str = "vt_session";
const CSRF_COOKIE: &str = "vt_csrf";
const CHALLENGE_TTL: Duration = Duration::minutes(5);
const SESSION_TTL: Duration = Duration::days(7);

#[derive(Debug, Clone)]
pub struct AuthenticatedUser {
    pub id: Uuid,
    pub wallet_address: String,
}

/// The single Hyperliquid exchange signer owned by a HyperVibes user.
///
/// The private-key fields are intentionally only loaded by exchange-signing
/// code. UI callers should use the audit fields without exposing ciphertext.
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserApiWalletRow {
    pub user_id: Uuid,
    pub main_wallet_address: String,
    pub api_wallet_address: Option<String>,
    pub hyperliquid_private_key_ciphertext: Option<Vec<u8>>,
    pub hyperliquid_private_key_key_id: Option<String>,
    pub api_wallet_approved_at: Option<DateTime<Utc>>,
    pub api_wallet_expires_at: Option<DateTime<Utc>>,
    pub api_wallet_expiry_checked_at: Option<DateTime<Utc>>,
}

impl UserApiWalletRow {
    pub fn is_ready(&self) -> bool {
        self.api_wallet_address.is_some()
            && self.hyperliquid_private_key_ciphertext.is_some()
            && self.hyperliquid_private_key_key_id.is_some()
            && self.api_wallet_approved_at.is_some()
            && self
                .api_wallet_expires_at
                .is_none_or(|expires_at| expires_at > Utc::now())
            && (self.api_wallet_expires_at.is_none() || self.api_wallet_expiry_checked_at.is_some())
    }
}

pub async fn get_user_api_wallet(
    pool: &DbPool,
    user_id: Uuid,
) -> anyhow::Result<Option<UserApiWalletRow>> {
    Ok(sqlx::query_as(
        "SELECT id AS user_id, wallet_address AS main_wallet_address,
                api_wallet_address, hyperliquid_private_key_ciphertext,
                hyperliquid_private_key_key_id, api_wallet_approved_at,
                api_wallet_expires_at, api_wallet_expiry_checked_at
           FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?)
}

pub async fn get_user_api_wallet_for_agent(
    pool: &DbPool,
    agent_key: &str,
) -> anyhow::Result<Option<UserApiWalletRow>> {
    Ok(sqlx::query_as(
        "SELECT users.id AS user_id, users.wallet_address AS main_wallet_address,
                users.api_wallet_address, users.hyperliquid_private_key_ciphertext,
                users.hyperliquid_private_key_key_id, users.api_wallet_approved_at,
                users.api_wallet_expires_at, users.api_wallet_expiry_checked_at
           FROM agents JOIN users ON users.id = agents.user_id
          WHERE agents.agent_key = $1",
    )
    .bind(agent_key)
    .fetch_optional(pool)
    .await?)
}

pub async fn store_user_api_wallet(
    pool: &DbPool,
    user_id: Uuid,
    api_wallet_address: &str,
    ciphertext: &[u8],
    key_id: &str,
) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE users
            SET api_wallet_address = $2,
                hyperliquid_private_key_ciphertext = $3,
                hyperliquid_private_key_key_id = $4,
                api_wallet_approved_at = NULL,
                api_wallet_expires_at = NULL,
                api_wallet_expiry_checked_at = NULL,
                updated_at = now()
          WHERE id = $1 AND (api_wallet_address IS NULL OR api_wallet_address <> $2)",
    )
    .bind(user_id)
    .bind(api_wallet_address)
    .bind(ciphertext)
    .bind(key_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

pub async fn record_user_api_wallet_approval(pool: &DbPool, user_id: Uuid) -> anyhow::Result<bool> {
    let result = sqlx::query(
        "UPDATE users
            SET api_wallet_approved_at = now(),
                api_wallet_expires_at = now() + INTERVAL '6 months',
                api_wallet_expiry_checked_at = now(),
                updated_at = now()
          WHERE id = $1
            AND api_wallet_address IS NOT NULL
            AND hyperliquid_private_key_ciphertext IS NOT NULL
            AND hyperliquid_private_key_key_id IS NOT NULL",
    )
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() == 1)
}

impl<S> FromRequestParts<S> for AuthenticatedUser
where
    Arc<AppState>: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = StatusCode;

    async fn from_request_parts(parts: &mut Parts, _state: &S) -> Result<Self, Self::Rejection> {
        parts
            .extensions
            .get::<AuthenticatedUser>()
            .cloned()
            .ok_or(StatusCode::UNAUTHORIZED)
    }
}

#[derive(Serialize)]
pub(crate) struct ChallengeResponse {
    challenge_id: Uuid,
    message: String,
}

#[derive(Deserialize)]
pub(crate) struct VerifyLogin {
    challenge_id: Uuid,
    message: String,
    signature: String,
}

#[derive(Deserialize)]
pub(crate) struct ChallengeRequest {
    wallet_address: String,
}

#[derive(FromRow)]
struct ChallengeRow {
    wallet_address: String,
    nonce: String,
    uri: String,
    domain: String,
    chain_id: i64,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
    consumed_at: Option<DateTime<Utc>>,
}

#[derive(FromRow)]
struct SessionRow {
    user_id: Uuid,
    wallet_address: String,
    csrf_hash: Vec<u8>,
}

pub(crate) async fn login() -> Html<&'static str> {
    Html(include_str!(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/templates/auth/login.html"
    )))
}

pub(crate) async fn login_challenge(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<ChallengeRequest>,
) -> Result<impl IntoResponse, StatusCode> {
    let domain = request_domain(&headers).ok_or(StatusCode::BAD_REQUEST)?;
    let uri = format!("{}://{domain}/login", request_scheme(&headers));
    let wallet_address = Address::from_str(&input.wallet_address)
        .map_err(|_| StatusCode::BAD_REQUEST)?
        .to_string()
        .to_ascii_lowercase();
    let nonce = opaque_token();
    let issued_at = DateTime::<Utc>::from_timestamp_micros(Utc::now().timestamp_micros())
        .expect("current timestamp is representable at microsecond precision");
    let expires_at = issued_at + CHALLENGE_TTL;
    let id = Uuid::new_v4();
    let message = siwe_message(
        &domain,
        &wallet_address,
        &uri,
        &nonce,
        issued_at,
        expires_at,
    );

    sqlx::query("INSERT INTO login_challenges (id, wallet_address, nonce, uri, domain, chain_id, issued_at, expires_at) VALUES ($1, $2, $3, $4, $5, 1, $6, $7)")
        .bind(id).bind(&wallet_address).bind(&nonce).bind(&uri).bind(&domain).bind(issued_at).bind(expires_at)
        .execute(&state.db_pool).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(axum::Json(ChallengeResponse {
        challenge_id: id,
        message,
    }))
}

pub(crate) async fn login_verify(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    axum::Json(input): axum::Json<VerifyLogin>,
) -> Result<Response, StatusCode> {
    let mut tx = state
        .db_pool
        .begin()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let challenge = sqlx::query_as::<_, ChallengeRow>("SELECT wallet_address, nonce, uri, domain, chain_id, issued_at, expires_at, consumed_at FROM login_challenges WHERE id = $1 FOR UPDATE")
        .bind(input.challenge_id).fetch_optional(&mut *tx).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
        .ok_or(StatusCode::UNAUTHORIZED)?;
    if challenge.consumed_at.is_some()
        || challenge.expires_at <= Utc::now()
        || challenge.chain_id != 1
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let expected = siwe_message(
        &challenge.domain,
        &challenge.wallet_address,
        &challenge.uri,
        &challenge.nonce,
        challenge.issued_at,
        challenge.expires_at,
    );
    if input.message != expected
        || request_domain(&headers).as_deref() != Some(challenge.domain.as_str())
    {
        return Err(StatusCode::UNAUTHORIZED);
    }
    let address = recover_address(&input.message, &input.signature)
        .filter(|address| address == &challenge.wallet_address)
        .ok_or(StatusCode::UNAUTHORIZED)?;
    let user_id: Uuid = sqlx::query_scalar("INSERT INTO users (id, wallet_address) VALUES ($1, $2) ON CONFLICT (wallet_address) DO UPDATE SET updated_at = now() RETURNING id")
        .bind(Uuid::new_v4()).bind(address).fetch_one(&mut *tx).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    sqlx::query("UPDATE login_challenges SET consumed_at = now() WHERE id = $1")
        .bind(input.challenge_id)
        .execute(&mut *tx)
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let token = opaque_token();
    let csrf = opaque_token();
    sqlx::query("INSERT INTO user_sessions (token_hash, csrf_hash, user_id, issued_at, expires_at) VALUES ($1, $2, $3, now(), $4)")
        .bind(hash(&token)).bind(hash(&csrf)).bind(user_id).bind(Utc::now() + SESSION_TTL)
        .execute(&mut *tx).await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    tx.commit()
        .await
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;

    let has_agents: Option<(i32,)> =
        sqlx::query_as("SELECT 1 FROM agents WHERE user_id = $1 AND lifecycle = 'active' LIMIT 1")
            .bind(user_id)
            .fetch_optional(&state.db_pool)
            .await
            .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    let target = if has_agents.is_some() {
        "/agents"
    } else {
        "/account"
    };

    let body = serde_json::json!({ "status": "ok", "redirect": target });
    let mut response = (StatusCode::OK, axum::Json(body)).into_response();
    let secure = request_scheme(&headers) == "https";
    response
        .headers_mut()
        .append("set-cookie", cookie(SESSION_COOKIE, &token, true, secure));
    response
        .headers_mut()
        .append("set-cookie", cookie(CSRF_COOKIE, &csrf, false, secure));
    Ok(response)
}

pub(crate) async fn logout(State(state): State<Arc<AppState>>, headers: HeaderMap) -> Response {
    if let Some(token) = cookie_value(&headers, SESSION_COOKIE) {
        let _ = sqlx::query("UPDATE user_sessions SET revoked_at = now() WHERE token_hash = $1")
            .bind(hash(token))
            .execute(&state.db_pool)
            .await;
    }
    let secure = request_scheme(&headers) == "https";
    let mut response = Redirect::to("/login").into_response();
    response
        .headers_mut()
        .append("set-cookie", expired_cookie(SESSION_COOKIE, true, secure));
    response
        .headers_mut()
        .append("set-cookie", expired_cookie(CSRF_COOKIE, false, secure));
    response
}

pub async fn require_operator(
    State(state): State<Arc<AppState>>,
    mut request: axum::extract::Request,
    next: Next,
) -> Response {
    #[cfg(test)]
    if state._test_db_guard.is_some() && cookie_value(request.headers(), SESSION_COOKIE).is_none() {
        // Existing route tests exercise handlers with isolated test databases rather
        // than browser login. Production builds never include this path.
        request.extensions_mut().insert(AuthenticatedUser {
            id: crate::test_db::test_user_id(),
            wallet_address: "0x0000000000000000000000000000000000000000".to_string(),
        });
        return next.run(request).await;
    }
    let Some(token) = cookie_value(request.headers(), SESSION_COOKIE) else {
        return login_required(&request);
    };
    let session = sqlx::query_as::<_, SessionRow>("SELECT user_sessions.user_id, users.wallet_address, csrf_hash FROM user_sessions JOIN users ON users.id = user_sessions.user_id WHERE token_hash = $1 AND revoked_at IS NULL AND expires_at > now()")
        .bind(hash(token)).fetch_optional(&state.db_pool).await;
    let Ok(Some(session)) = session else {
        return login_required(&request);
    };

    if request.method() != axum::http::Method::GET && request.method() != axum::http::Method::HEAD {
        let (parts, body) = request.into_parts();
        let Ok(bytes) = to_bytes(body, 1024 * 1024).await else {
            return StatusCode::BAD_REQUEST.into_response();
        };
        let supplied = parts
            .headers
            .get("x-csrf-token")
            .and_then(|value| value.to_str().ok())
            .or_else(|| form_value(&bytes, "csrf_token"));
        if supplied.is_none_or(|value| hash(value) != session.csrf_hash) {
            return StatusCode::FORBIDDEN.into_response();
        }
        request = axum::extract::Request::from_parts(parts, Body::from(bytes));
    }

    if let Some(agent_key) = request
        .uri()
        .path()
        .strip_prefix("/agents/")
        .and_then(|path| path.split('/').next())
        && agent_key != "navigation"
        && agent_key != "new"
    {
        match agent_belongs_to_user(&state.db_pool, agent_key, session.user_id).await {
            Ok(true) => {}
            Ok(false) => {
                if request.method() == axum::http::Method::GET
                    || request.method() == axum::http::Method::HEAD
                {
                    return crate::web::error::not_found_response(request.uri().path());
                }
                return StatusCode::NOT_FOUND.into_response();
            }
            Err(_) => return StatusCode::INTERNAL_SERVER_ERROR.into_response(),
        }
    }
    request.extensions_mut().insert(AuthenticatedUser {
        id: session.user_id,
        wallet_address: session.wallet_address,
    });
    next.run(request).await
}

fn login_required(request: &axum::extract::Request) -> Response {
    if request.method() == axum::http::Method::GET {
        Redirect::to("/login").into_response()
    } else {
        StatusCode::UNAUTHORIZED.into_response()
    }
}

fn request_domain(headers: &HeaderMap) -> Option<String> {
    headers
        .get("host")
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}
fn request_scheme(headers: &HeaderMap) -> &str {
    headers
        .get("x-forwarded-proto")
        .and_then(|value| value.to_str().ok())
        .filter(|value| *value == "https")
        .unwrap_or("http")
}
fn opaque_token() -> String {
    URL_SAFE_NO_PAD.encode(rand::rng().random::<[u8; 32]>())
}
fn hash(value: &str) -> Vec<u8> {
    Sha256::digest(value.as_bytes()).to_vec()
}
fn cookie_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get("cookie")?
        .to_str()
        .ok()?
        .split(';')
        .map(str::trim)
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}
fn cookie(name: &str, value: &str, http_only: bool, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}={value}; Path=/; SameSite=Lax; Max-Age={};{}{}",
        SESSION_TTL.num_seconds(),
        if http_only { " HttpOnly;" } else { "" },
        secure
    ))
    .expect("cookie values are opaque URL-safe tokens")
}
fn expired_cookie(name: &str, http_only: bool, secure: bool) -> HeaderValue {
    let secure = if secure { "; Secure" } else { "" };
    HeaderValue::from_str(&format!(
        "{name}=; Path=/; SameSite=Lax; Max-Age=0;{}{}",
        if http_only { " HttpOnly;" } else { "" },
        secure
    ))
    .expect("cookie name is static")
}
fn form_value<'a>(body: &'a [u8], name: &str) -> Option<&'a str> {
    std::str::from_utf8(body)
        .ok()?
        .split('&')
        .find_map(|part| part.strip_prefix(&format!("{name}=")))
}
fn siwe_message(
    domain: &str,
    address: &str,
    uri: &str,
    nonce: &str,
    issued_at: DateTime<Utc>,
    expires_at: DateTime<Utc>,
) -> String {
    format!(
        "{domain} wants you to sign in with your Ethereum account:\n{address}\n\nURI: {uri}\nVersion: 1\nChain ID: 1\nNonce: {nonce}\nIssued At: {}\nExpiration Time: {}",
        issued_at.to_rfc3339(),
        expires_at.to_rfc3339()
    )
}
fn recover_address(message: &str, signature: &str) -> Option<String> {
    let signature = Signature::from_str(signature).ok()?;
    let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
    signature
        .recover_address_from_prehash(&keccak256([prefix.as_bytes(), message.as_bytes()].concat()))
        .ok()
        .map(|address| address.to_string().to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloy::{
        primitives::Address,
        signers::{Signer, local::PrivateKeySigner},
    };

    #[tokio::test]
    async fn siwe_signature_recovers_the_address_embedded_in_the_challenge() {
        let signer = PrivateKeySigner::from_str(
            "4c0883a69102937d6231471b5dbb6204fe5129617082795f9d3d2c7e2f9f3f5b",
        )
        .expect("test signer");
        let address = signer.address().to_string().to_ascii_lowercase();
        let now = Utc::now();
        let message = siwe_message(
            "example.test",
            &address,
            "https://example.test/login",
            "nonce",
            now,
            now + CHALLENGE_TTL,
        );
        let signature = signer
            .sign_message(message.as_bytes())
            .await
            .expect("sign challenge");
        assert_eq!(
            recover_address(&message, &signature.to_string()),
            Some(address)
        );
        assert!(message.contains("Chain ID: 1"));
        assert!(Address::from_str("0x8f0bb61c41988b44f623a0b5390fd2b52838d20e").is_ok());
    }

    #[test]
    fn siwe_timestamps_match_postgres_precision() {
        let issued_at = DateTime::<Utc>::from_timestamp_nanos(1_752_886_867_123_456_789);
        let stored_issued_at = DateTime::<Utc>::from_timestamp_micros(issued_at.timestamp_micros())
            .expect("test timestamp is representable at microsecond precision");
        assert_ne!(issued_at, stored_issued_at);
        assert_eq!(stored_issued_at.timestamp_subsec_nanos(), 123_456_000);

        let message = siwe_message(
            "example.test",
            "0x0000000000000000000000000000000000000000",
            "https://example.test/login",
            "nonce",
            stored_issued_at,
            stored_issued_at + CHALLENGE_TTL,
        );
        assert_eq!(
            message,
            siwe_message(
                "example.test",
                "0x0000000000000000000000000000000000000000",
                "https://example.test/login",
                "nonce",
                DateTime::<Utc>::from_timestamp_micros(stored_issued_at.timestamp_micros())
                    .expect("stored timestamp is representable at microsecond precision"),
                DateTime::<Utc>::from_timestamp_micros(
                    (stored_issued_at + CHALLENGE_TTL).timestamp_micros(),
                )
                .expect("stored expiration is representable at microsecond precision"),
            )
        );
    }

    #[test]
    fn csrf_cookie_is_not_http_only_but_session_cookie_is() {
        assert!(
            cookie(SESSION_COOKIE, "token", true, true)
                .to_str()
                .expect("header")
                .contains("HttpOnly")
        );
        assert!(
            !cookie(CSRF_COOKIE, "token", false, true)
                .to_str()
                .expect("header")
                .contains("HttpOnly")
        );
    }
}
