use crate::{
    cache::asset::{AssetCachePolicy, validate_key},
    web::AppState,
};
use axum::{
    extract::{Path, State},
    http::StatusCode,
    response::IntoResponse,
};
use std::sync::Arc;
pub(in crate::web::routes) async fn model_catalog_logo(
    State(state): State<Arc<AppState>>,
    Path(provider): Path<String>,
) -> impl IntoResponse {
    let key = format!("{provider}.svg");
    if validate_key(&key).is_err() {
        return (
            StatusCode::BAD_REQUEST,
            [("Content-Type", "text/plain; charset=utf-8")],
            "invalid provider",
        )
            .into_response();
    }

    let body = match state
        .asset_cache
        .get_or_fetch(
            "models-dev/logos",
            &key,
            &format!("https://models.dev/logos/{provider}.svg"),
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
        Ok(_) | Err(_) => fallback_logo_svg(&provider).into_bytes(),
    };

    (
        StatusCode::OK,
        [
            ("Content-Type", "image/svg+xml; charset=utf-8"),
            ("Cache-Control", "public, max-age=86400"),
        ],
        body,
    )
        .into_response()
}
pub(in crate::web::routes) fn body_looks_like_svg(body: &[u8]) -> bool {
    let text = String::from_utf8_lossy(body);
    let trimmed = text.trim_start();
    trimmed.starts_with("<svg") || (trimmed.starts_with("<?xml") && trimmed.contains("<svg"))
}
pub(in crate::web::routes) fn fallback_logo_svg(provider: &str) -> String {
    let initial = provider.chars().next().unwrap_or('M').to_ascii_uppercase();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="{provider}"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="ui-sans-serif,system-ui" font-size="14" fill="#fafafa">{initial}</text></svg>"##
    )
}
