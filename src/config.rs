use std::{env, path::PathBuf};

use anyhow::{Context, Result, bail};
use reqwest::Url;
#[derive(Debug, Clone)]
pub struct AppConfig {
    pub database_url: String,
    pub bind_addr: String,
    pub app_cache_dir: PathBuf,
    pub agents_encryption_key: [u8; 32],
    pub agents_encryption_key_id: String,
    pub opencode_container_workspaces_root: String,
    pub vibetrading_agent_api_base_url: String,
    pub workspace_control_base_url: String,
    pub workspace_control_api_key: String,
    pub opencode_base_url: String,
    pub opencode_server_username: String,
    pub opencode_server_password: Option<String>,
}

impl AppConfig {
    pub fn from_env() -> Result<Self> {
        Ok(Self {
            database_url: database_url_from_env()?,
            bind_addr: bind_addr_from_env(),
            app_cache_dir: app_cache_dir_from_env()?,
            agents_encryption_key: agents_encryption_key_from_env()?,
            agents_encryption_key_id: agents_encryption_key_id_from_env()?,
            opencode_container_workspaces_root: opencode_container_workspaces_root_from_env()?,
            vibetrading_agent_api_base_url: vibetrading_agent_api_base_url_from_env()?,
            workspace_control_base_url: workspace_control_base_url_from_env()?,
            workspace_control_api_key: workspace_control_api_key_from_env()?,
            opencode_base_url: opencode_base_url_from_env()?,
            opencode_server_username: opencode_server_username_from_env(),
            opencode_server_password: opencode_server_password_from_env(),
        })
    }
}

fn app_cache_dir_from_env() -> Result<PathBuf> {
    let raw = env::var("APP_CACHE_DIR").unwrap_or_else(|_| "cache".to_string());
    let raw = raw.trim();
    if raw.is_empty() {
        bail!("APP_CACHE_DIR must not be empty");
    }

    let path = PathBuf::from(raw);
    if path.is_absolute() {
        Ok(path)
    } else {
        Ok(env::current_dir()
            .context("failed to resolve current working directory for APP_CACHE_DIR")?
            .join(path))
    }
}

fn bind_addr_from_env() -> String {
    if let Ok(bind_addr) = env::var("APP_BIND_ADDR")
        && !bind_addr.trim().is_empty()
    {
        return bind_addr;
    }

    let host = env::var("APP_HOST").unwrap_or_else(|_| "127.0.0.1".to_string());
    let port = env::var("APP_PORT").unwrap_or_else(|_| "3000".to_string());

    format!("{}:{}", host, port)
}

pub fn database_url_from_env() -> Result<String> {
    if let Ok(database_url) = env::var("DATABASE_URL")
        && !database_url.trim().is_empty()
    {
        return Ok(database_url);
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

fn workspace_control_base_url_from_env() -> Result<String> {
    let url = env::var("WORKSPACE_CONTROL_BASE_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:14097".to_string());
    let url = url.trim();
    if url.is_empty() {
        bail!("WORKSPACE_CONTROL_BASE_URL must not be empty");
    }
    validate_absolute_url("WORKSPACE_CONTROL_BASE_URL", url)?;
    Ok(url.trim_end_matches('/').to_string())
}

fn workspace_control_api_key_from_env() -> Result<String> {
    let key = env::var("WORKSPACE_CONTROL_API_KEY")
        .context("missing WORKSPACE_CONTROL_API_KEY environment variable")?;
    let key = key.trim();
    if key.is_empty() {
        bail!("WORKSPACE_CONTROL_API_KEY must not be empty");
    }
    Ok(key.to_string())
}

fn opencode_base_url_from_env() -> Result<String> {
    let url =
        env::var("OPENCODE_BASE_URL").unwrap_or_else(|_| "http://localhost:14096".to_string());
    let url = url.trim();
    if url.is_empty() {
        bail!("OPENCODE_BASE_URL must not be empty");
    }
    validate_absolute_url("OPENCODE_BASE_URL", url)?;
    Ok(url.trim_end_matches('/').to_string())
}

fn validate_absolute_url(name: &str, value: &str) -> Result<()> {
    let url = Url::parse(value).with_context(|| format!("{name} must be a valid absolute URL"))?;
    if url.scheme().is_empty() || url.host_str().is_none() {
        bail!("{name} must include a scheme and host");
    }
    Ok(())
}

fn opencode_server_username_from_env() -> String {
    let raw = env::var("OPENCODE_SERVER_USERNAME").unwrap_or_else(|_| "opencode".to_string());
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        "opencode".to_string()
    } else {
        trimmed.to_string()
    }
}

fn opencode_server_password_from_env() -> Option<String> {
    env::var("OPENCODE_SERVER_PASSWORD")
        .ok()
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
}

#[cfg(test)]
mod tests {
    use std::env;

    use super::app_cache_dir_from_env;

    #[test]
    fn app_cache_dir_defaults_relative_to_current_directory() {
        let previous = env::var_os("APP_CACHE_DIR");
        unsafe {
            env::remove_var("APP_CACHE_DIR");
        }

        let result = app_cache_dir_from_env().unwrap();

        match previous {
            Some(value) => unsafe { env::set_var("APP_CACHE_DIR", value) },
            None => unsafe { env::remove_var("APP_CACHE_DIR") },
        }

        assert_eq!(result, env::current_dir().unwrap().join("cache"));
    }

    #[test]
    fn app_cache_dir_resolves_relative_env_value() {
        let previous = env::var_os("APP_CACHE_DIR");
        unsafe {
            env::set_var("APP_CACHE_DIR", "tmp/cache-dir");
        }

        let result = app_cache_dir_from_env().unwrap();

        match previous {
            Some(value) => unsafe { env::set_var("APP_CACHE_DIR", value) },
            None => unsafe { env::remove_var("APP_CACHE_DIR") },
        }

        assert_eq!(result, env::current_dir().unwrap().join("tmp/cache-dir"));
    }
}
