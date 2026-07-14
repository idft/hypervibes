use std::sync::Arc;

use anyhow::{Result, bail};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};

use crate::{
    cache::asset::{AssetCachePolicy, validate_key},
    web::AppState,
};

use super::model_catalog::body_looks_like_svg;

fn parse_currency_logo_filename(file: &str) -> Result<&str> {
    validate_key(file)?;
    let coin = file
        .strip_suffix(".svg")
        .ok_or_else(|| anyhow::anyhow!("missing svg extension"))?;
    if coin.is_empty() {
        bail!("empty coin identifier");
    }
    validate_key(coin)?;
    if coin.contains('.') {
        bail!("currency identifier must not contain a dot");
    }
    Ok(coin)
}

pub(in crate::web::routes) async fn currency_logo(
    State(state): State<Arc<AppState>>,
    Path(file): Path<String>,
) -> impl IntoResponse {
    let coin = match parse_currency_logo_filename(&file) {
        Ok(coin) => coin,
        Err(_) => {
            return (
                StatusCode::BAD_REQUEST,
                [("Content-Type", "text/plain; charset=utf-8")],
                "invalid currency logo filename",
            )
                .into_response();
        }
    };

    let body = match state
        .asset_cache
        .get_or_fetch(
            "hyperliquid/logos",
            &format!("{coin}.svg"),
            &format!("https://app.hyperliquid.xyz/coins/{coin}.svg"),
            AssetCachePolicy {
                ttl: std::time::Duration::from_secs(30 * 24 * 60 * 60),
                max_bytes: 256 * 1024,
                allowed_content_types: vec![
                    "image/svg+xml",
                    "text/plain",
                    "application/octet-stream",
                ],
            },
        )
        .await
    {
        Ok(asset) if body_looks_like_svg(&asset.bytes) => asset.bytes,
        Ok(_) | Err(_) => fallback_currency_logo_svg(coin).into_bytes(),
    };

    (
        StatusCode::OK,
        [
            ("Content-Type", "image/svg+xml; charset=utf-8"),
            ("Cache-Control", "public, max-age=86400"),
            ("X-Content-Type-Options", "nosniff"),
            (
                "Content-Security-Policy",
                "sandbox; default-src 'none'; style-src 'unsafe-inline'; img-src 'none'; base-uri 'none'; form-action 'none'",
            ),
        ],
        body,
    )
        .into_response()
}

fn fallback_currency_logo_svg(coin: &str) -> String {
    let font_size = if coin.len() <= 3 { 11 } else { 8 };
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="{coin} logo"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="ui-sans-serif,system-ui" font-size="{font_size}" textLength="26" lengthAdjust="spacingAndGlyphs" fill="#fafafa">{coin}</text></svg>"##
    )
}

#[cfg(test)]
#[path = "currency_tests.rs"]
mod currency_tests;
