use chrono::Utc;
use uuid::Uuid;

use crate::{
    agents::store::{list_agent_readiness_for_user, list_agents_for_user},
    db::DbPool,
};

/// Server-rendered fragment included by every page via `{% include "layouts/navbar.html" %}`. Holds
/// the authenticated wallet address and user-visible warning state (Hyperliquid API key,
/// builder fee approval) used by the top bar without an extra round trip.
///
/// The fragment is rendered by askama's `{% include %}` mechanism: pages that
/// extend `base.html` declare a `navbar: Navbar` field and the included
/// template reads the warnings through the parent's context.
#[derive(Debug, Clone, Default)]
pub struct Navbar {
    pub wallet_address: String,
    pub warnings: Vec<NavbarWarning>,
    pub agents: Vec<NavbarAgent>,
    pub can_create_agent: bool,
    pub selected_agent_key: Option<String>,
    pub selected_agent_name: Option<String>,
    pub selected_agent_enabled: bool,
    pub selected_agent_trading_job_enabled: Option<bool>,
}

#[derive(Debug, Clone)]
pub struct NavbarAgent {
    pub agent_key: String,
    pub display_name: String,
    pub enabled: bool,
    pub trading_job_enabled: bool,
}

impl Navbar {
    pub fn with_selected_agent(
        mut self,
        agent_key: String,
        display_name: String,
        enabled: bool,
    ) -> Self {
        self.selected_agent_key = Some(agent_key);
        self.selected_agent_name = Some(display_name);
        self.selected_agent_enabled = enabled;
        self.selected_agent_trading_job_enabled = self
            .agents
            .iter()
            .find(|agent| self.selected_agent_key.as_deref() == Some(agent.agent_key.as_str()))
            .map(|agent| agent.trading_job_enabled);
        self
    }

    pub fn agent_selector_label(&self) -> &str {
        self.selected_agent_name.as_deref().unwrap_or("Agents")
    }

    pub fn short_wallet_address(&self) -> String {
        let address = &self.wallet_address;
        if address.len() <= 10 {
            return address.clone();
        }
        format!("{}...{}", &address[..6], &address[address.len() - 4..])
    }

    pub fn identicon_data_uri(&self) -> String {
        let mut seed = 0_u32;
        for byte in self.wallet_address.bytes() {
            seed = (seed << 5).wrapping_sub(seed).wrapping_add(u32::from(byte));
        }
        let mut random = || {
            seed = ((u64::from(seed) * 9_301 + 49_297) % 233_280) as u32;
            f64::from(seed) / 233_280.0
        };
        let mut color = || {
            format!(
                "hsl({} {}% {}%)",
                (random() * 360.0).floor(),
                (random() * 60.0 + 40.0).floor(),
                ((random() + random() + random() + random()) * 25.0).floor(),
            )
        };
        let foreground = color();
        let background = color();
        let spot = color();
        let mut squares = String::new();
        for row in 0..8 {
            let mut values: Vec<u8> = (0..4).map(|_| (random() * 2.3).floor() as u8).collect();
            values.extend([values[3], values[2], values[1], values[0]]);
            for (column, value) in values.into_iter().enumerate() {
                if value != 0 {
                    let color = if value == 1 { &foreground } else { &spot };
                    squares.push_str(&format!(
                        r#"<rect x="{column}" y="{row}" width="1" height="1" fill="{color}"/>"#
                    ));
                }
            }
        }
        let svg = format!(
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 8 8"><rect width="8" height="8" fill="{background}"/>{squares}</svg>"#
        );
        format!("data:image/svg+xml,{}", encode_uri_component(&svg))
    }

    pub fn has_warnings(&self) -> bool {
        !self.warnings.is_empty()
    }

    /// Newline-separated warning labels, suitable for the icon's `title`
    /// tooltip so each warning lands on its own line on hover.
    pub fn tooltip(&self) -> String {
        self.warnings
            .iter()
            .map(|warning| warning.label())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Semicolon-separated labels, suitable for the `aria-label` where a
    /// single string is required.
    pub fn aria_label(&self) -> String {
        self.warnings
            .iter()
            .map(|warning| warning.label())
            .collect::<Vec<_>>()
            .join("; ")
    }
}

fn encode_uri_component(value: &str) -> String {
    let mut encoded = String::with_capacity(value.len());
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric()
            || matches!(
                byte,
                b'-' | b'_' | b'.' | b'!' | b'~' | b'*' | b'\'' | b'(' | b')'
            )
        {
            encoded.push(char::from(byte));
        } else {
            encoded.push('%');
            encoded.push_str(&format!("{byte:02X}"));
        }
    }
    encoded
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavbarWarning {
    ApiKeyNotSet,
    ApiKeyExpiringSoon,
    BuilderFeeNotApproved,
}

impl NavbarWarning {
    pub fn label(self) -> &'static str {
        match self {
            Self::ApiKeyNotSet => "Hyperliquid API key not set",
            Self::ApiKeyExpiringSoon => "Hyperliquid API key expires soon",
            Self::BuilderFeeNotApproved => "Builder fee not approved",
        }
    }
}

/// How many days until the API key expires. Returns `None` if no expiry is
/// recorded, or a negative number if the key is already past its expiry.
fn days_until_expiry(expires_at: Option<chrono::DateTime<Utc>>) -> Option<i64> {
    let expires_at = expires_at?;
    Some((expires_at - Utc::now()).num_days())
}

/// Load the state shared by every authenticated page's top bar.
pub async fn load_navbar(pool: &DbPool, user_id: Uuid) -> anyhow::Result<Navbar> {
    let (row, agents, readiness_by_agent) = tokio::try_join!(
        async {
            let row: NavbarRow = sqlx::query_as(
                "SELECT wallet_address, api_wallet_address, api_wallet_approved_at,
                  api_wallet_expires_at, api_wallet_expiry_checked_at,
                  builder_fee_approved_at,
                  hyperliquid_private_key_ciphertext IS NOT NULL AS has_api_wallet_private_key,
                  hyperliquid_private_key_key_id IS NOT NULL AS has_api_wallet_key_id
             FROM users WHERE id = $1",
            )
            .bind(user_id)
            .fetch_one(pool)
            .await?;
            Ok::<_, anyhow::Error>(row)
        },
        list_agents_for_user(pool, user_id),
        list_agent_readiness_for_user(pool, user_id),
    )?;
    let now = Utc::now();
    let api_key_ready = row.api_wallet_address.is_some()
        && row.api_wallet_approved_at.is_some()
        && row
            .api_wallet_expires_at
            .is_none_or(|expires_at| expires_at > now)
        && (row.api_wallet_expires_at.is_none() || row.api_wallet_expiry_checked_at.is_some());
    let mut warnings = Vec::new();
    if !api_key_ready {
        warnings.push(NavbarWarning::ApiKeyNotSet);
    } else if let Some(days) = days_until_expiry(row.api_wallet_expires_at)
        && days < 30
    {
        warnings.push(NavbarWarning::ApiKeyExpiringSoon);
    }
    if row.builder_fee_approved_at.is_none() {
        warnings.push(NavbarWarning::BuilderFeeNotApproved);
    }
    Ok(Navbar {
        wallet_address: row.wallet_address,
        warnings,
        agents: agents
            .into_iter()
            .map(|agent| {
                let trading_job_enabled = readiness_by_agent
                    .get(&agent.agent_key)
                    .is_some_and(|readiness| readiness.has_enabled_trading_job);
                NavbarAgent {
                    agent_key: agent.agent_key,
                    display_name: agent.display_name,
                    enabled: agent.enabled,
                    trading_job_enabled,
                }
            })
            .collect(),
        can_create_agent: api_key_ready
            && row.has_api_wallet_private_key
            && row.has_api_wallet_key_id,
        selected_agent_key: None,
        selected_agent_name: None,
        selected_agent_enabled: false,
        selected_agent_trading_job_enabled: None,
    })
}

#[derive(sqlx::FromRow)]
struct NavbarRow {
    wallet_address: String,
    api_wallet_address: Option<String>,
    api_wallet_approved_at: Option<chrono::DateTime<chrono::Utc>>,
    api_wallet_expires_at: Option<chrono::DateTime<chrono::Utc>>,
    api_wallet_expiry_checked_at: Option<chrono::DateTime<chrono::Utc>>,
    builder_fee_approved_at: Option<chrono::DateTime<chrono::Utc>>,
    has_api_wallet_private_key: bool,
    has_api_wallet_key_id: bool,
}

#[cfg(test)]
mod tests {
    use super::{Navbar, NavbarAgent};

    #[test]
    fn navbar_renders_a_short_wallet_address_and_safe_identicon_uri() {
        let navbar = Navbar {
            wallet_address: "0x1234567890abcdef1234567890abcdef12345678".to_string(),
            warnings: Vec::new(),
            ..Default::default()
        };

        assert_eq!(navbar.short_wallet_address(), "0x1234...5678");
        let identicon = navbar.identicon_data_uri();
        assert!(identicon.starts_with("data:image/svg+xml,"));
        assert!(!identicon.contains('<'));
        assert!(!identicon.contains('"'));
    }

    #[test]
    fn navbar_uses_the_selected_agent_label_and_status() {
        let navbar = Navbar {
            agents: vec![NavbarAgent {
                agent_key: "btc-agent".to_string(),
                display_name: "BTC Agent".to_string(),
                enabled: true,
                trading_job_enabled: false,
            }],
            ..Default::default()
        }
        .with_selected_agent("btc-agent".to_string(), "BTC Agent".to_string(), true);

        assert_eq!(navbar.agent_selector_label(), "BTC Agent");
        assert!(navbar.selected_agent_enabled);
        assert_eq!(navbar.selected_agent_trading_job_enabled, Some(false));
    }
}
