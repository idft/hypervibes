use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use reqwest::Url;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub bind_addr: String,
    pub agents_encryption_key: [u8; 32],
    pub agents_encryption_key_id: String,
    pub hermes_dashboard_url: String,
    pub hermes_dashboard_link_url: String,
    pub hermes_dashboard_session_token: Option<String>,
    pub opencode_workspaces_root: PathBuf,
    pub opencode_container_workspaces_root: String,
    pub vibetrading_agent_api_base_url: String,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        let hermes_dashboard_url = hermes_dashboard_url_from_env();
        let app_public_url = app_public_url_from_env()?;

        Ok(Self {
            database_url: database_url_from_env()?,
            bind_addr: bind_addr_from_env(),
            agents_encryption_key: agents_encryption_key_from_env()?,
            agents_encryption_key_id: agents_encryption_key_id_from_env()?,
            hermes_dashboard_url: hermes_dashboard_url.clone(),
            hermes_dashboard_link_url: hermes_dashboard_link_url(
                &hermes_dashboard_url,
                app_public_url.as_deref(),
            )?,
            hermes_dashboard_session_token: hermes_dashboard_session_token_from_env(),
            opencode_workspaces_root: opencode_workspaces_root_from_env()?,
            opencode_container_workspaces_root: opencode_container_workspaces_root_from_env()?,
            vibetrading_agent_api_base_url: vibetrading_agent_api_base_url_from_env()?,
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

fn app_public_url_from_env() -> Result<Option<String>> {
    match env::var("APP_PUBLIC_URL") {
        Ok(url) => {
            let url = url.trim();
            if url.is_empty() {
                return Ok(None);
            }
            validate_absolute_url("APP_PUBLIC_URL", url)?;
            Ok(Some(url.to_string()))
        }
        Err(_) => Ok(None),
    }
}

fn hermes_dashboard_link_url(
    hermes_dashboard_url: &str,
    app_public_url: Option<&str>,
) -> Result<String> {
    let Some(app_public_url) = app_public_url else {
        return Ok(hermes_dashboard_url.to_string());
    };

    let dashboard_url = Url::parse(hermes_dashboard_url)
        .with_context(|| format!("invalid Hermes dashboard URL: {hermes_dashboard_url}"))?;
    let mut browser_url = Url::parse(app_public_url)
        .with_context(|| format!("invalid APP_PUBLIC_URL: {app_public_url}"))?;

    browser_url.set_path("");
    browser_url.set_query(None);
    browser_url.set_fragment(None);
    browser_url
        .set_port(dashboard_url.port_or_known_default())
        .map_err(|_| anyhow::anyhow!("APP_PUBLIC_URL contains a host that cannot accept a port"))?;

    Ok(browser_url.to_string().trim_end_matches('/').to_string())
}

fn validate_absolute_url(name: &str, value: &str) -> Result<()> {
    let url = Url::parse(value).with_context(|| format!("{name} must be a valid absolute URL"))?;
    if url.scheme().is_empty() || url.host_str().is_none() {
        bail!("{name} must include a scheme and host");
    }
    Ok(())
}

fn hermes_dashboard_session_token_from_env() -> Option<String> {
    env::var("HERMES_DASHBOARD_SESSION_TOKEN")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

fn opencode_workspaces_root_from_env() -> Result<PathBuf> {
    let raw = env::var("OPENCODE_WORKSPACES_ROOT").unwrap_or_else(|_| "workspaces".to_string());
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("OPENCODE_WORKSPACES_ROOT must not be empty");
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .context("failed to resolve current working directory for OPENCODE_WORKSPACES_ROOT")?
            .join(path))
    }
}

fn opencode_container_workspaces_root_from_env() -> Result<String> {
    let root = env::var("OPENCODE_CONTAINER_WORKSPACES_ROOT")
        .unwrap_or_else(|_| "/workspaces".to_string());
    let root = root.trim();
    if root.is_empty() {
        bail!("OPENCODE_CONTAINER_WORKSPACES_ROOT must not be empty");
    }
    if !root.starts_with('/') {
        bail!("OPENCODE_CONTAINER_WORKSPACES_ROOT must be an absolute path");
    }
    Ok(root.trim_end_matches('/').to_string())
}

fn vibetrading_agent_api_base_url_from_env() -> Result<String> {
    let url = env::var("VIBETRADING_AGENT_API_BASE_URL")
        .unwrap_or_else(|_| "http://host.containers.internal:3003".to_string());
    let url = url.trim();
    if url.is_empty() {
        bail!("VIBETRADING_AGENT_API_BASE_URL must not be empty");
    }
    validate_absolute_url("VIBETRADING_AGENT_API_BASE_URL", url)?;
    Ok(url.to_string())
}

#[cfg(test)]
mod tests {
    use super::hermes_dashboard_link_url;

    #[test]
    fn hermes_dashboard_link_defaults_to_backend_url() {
        let link = hermes_dashboard_link_url("http://127.0.0.1:19119", None).unwrap();
        assert_eq!(link, "http://127.0.0.1:19119");
    }

    #[test]
    fn hermes_dashboard_link_uses_public_app_host() {
        let link = hermes_dashboard_link_url(
            "http://127.0.0.1:19119",
            Some("http://trading-box.local:3003"),
        )
        .unwrap();
        assert_eq!(link, "http://trading-box.local:19119");
    }

    #[test]
    fn hermes_dashboard_link_keeps_public_scheme() {
        let link = hermes_dashboard_link_url(
            "http://127.0.0.1:19119",
            Some("https://ops.example.com/vibetrading"),
        )
        .unwrap();
        assert_eq!(link, "https://ops.example.com:19119");
    }
}
