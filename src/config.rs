use std::env;

use anyhow::{Context, Result, bail};

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub bind_addr: String,
    pub agents_encryption_key: [u8; 32],
    pub agents_encryption_key_id: String,
    pub hermes_dashboard_url: String,
    pub hermes_dashboard_session_token: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: database_url_from_env()?,
            bind_addr: bind_addr_from_env(),
            agents_encryption_key: agents_encryption_key_from_env()?,
            agents_encryption_key_id: agents_encryption_key_id_from_env()?,
            hermes_dashboard_url: hermes_dashboard_url_from_env(),
            hermes_dashboard_session_token: hermes_dashboard_session_token_from_env(),
        })
    }
}

fn bind_addr_from_env() -> String {
    if let Ok(bind_addr) = env::var("APP_BIND_ADDR") {
        if !bind_addr.trim().is_empty() {
            return bind_addr;
        }
    }

    let host = env::var("APP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("APP_PORT").unwrap_or_else(|_| "3000".to_string());

    format!("{}:{}", host, port)
}

pub fn database_url_from_env() -> Result<String> {
    if let Ok(database_url) = env::var("DATABASE_URL") {
        if !database_url.trim().is_empty() {
            return Ok(database_url);
        }
    }

    let host = env::var("POSTGRES_HOST").unwrap_or_else(|_| "localhost".to_string());
    let port = env::var("POSTGRES_PORT").unwrap_or_else(|_| "5432".to_string());
    let user = env::var("POSTGRES_USER").unwrap_or_else(|_| "vibetrading".to_string());
    let password = env::var("POSTGRES_PASSWORD").unwrap_or_else(|_| "vibetrading".to_string());
    let database = env::var("POSTGRES_DB").unwrap_or_else(|_| "vibetrading".to_string());

    if host.trim().is_empty()
        || port.trim().is_empty()
        || user.trim().is_empty()
        || database.trim().is_empty()
    {
        bail!("postgres environment variables are incomplete")
    }

    Ok(format!(
        "postgres://{}:{}@{}:{}/{}",
        user, password, host, port, database
    ))
}

fn agents_encryption_key_from_env() -> Result<[u8; 32]> {
    let raw = env::var("AGENTS_ENCRYPTION_KEY")
        .context("missing AGENTS_ENCRYPTION_KEY environment variable")?;
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("AGENTS_ENCRYPTION_KEY must not be empty");
    }
    let bytes =
        hex::decode(raw).context("AGENTS_ENCRYPTION_KEY must be a valid hexadecimal string")?;
    if bytes.len() != 32 {
        bail!(
            "AGENTS_ENCRYPTION_KEY must decode to exactly 32 bytes, got {}",
            bytes.len()
        );
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Ok(key)
}

fn agents_encryption_key_id_from_env() -> Result<String> {
    env::var("AGENTS_ENCRYPTION_KEY_ID")
        .context("missing AGENTS_ENCRYPTION_KEY_ID environment variable")
}

fn hermes_dashboard_url_from_env() -> String {
    let host = env::var("HERMES_DASHBOARD_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("HERMES_DASHBOARD_PORT").unwrap_or_else(|_| "19119".to_string());
    format!("http://{host}:{port}")
}

fn hermes_dashboard_session_token_from_env() -> Option<String> {
    env::var("HERMES_DASHBOARD_SESSION_TOKEN")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}
