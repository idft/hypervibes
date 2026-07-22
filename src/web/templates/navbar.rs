use chrono::Utc;
use uuid::Uuid;

use crate::db::DbPool;

/// Server-rendered fragment included by every page via `{% include "navbar.html" %}`. Holds
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
}

impl Navbar {
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

/// Load the warning state for the navbar in a single query. The result
/// is shared by every authenticated page so the top bar can render the
/// status icon with one DB hit per request.
pub async fn load_navbar(pool: &DbPool, user_id: Uuid) -> Result<Navbar, sqlx::Error> {
    let row: NavbarRow = sqlx::query_as(
        "SELECT wallet_address, api_wallet_address, api_wallet_approved_at,
                 api_wallet_expires_at, api_wallet_expiry_checked_at,
                 builder_fee_approved_at
           FROM users WHERE id = $1",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;
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
}
