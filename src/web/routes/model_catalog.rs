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
    let Ok(text) = std::str::from_utf8(body) else {
        return false;
    };
    let Ok(document) = roxmltree::Document::parse(text) else {
        return false;
    };
    let root = document.root_element();
    if root.tag_name().name() != "svg"
        || root.tag_name().namespace() != Some("http://www.w3.org/2000/svg")
    {
        return false;
    }

    document
        .descendants()
        .filter(|node| node.is_element())
        .all(|element| {
            let name = element.tag_name().name();
            name != "script"
                && name != "foreignObject"
                && element.attributes().all(|attribute| {
                    let local_name = attribute.name();
                    !local_name
                        .get(..2)
                        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("on"))
                        && (!local_name.eq_ignore_ascii_case("href")
                            || attribute.value().starts_with('#'))
                })
        })
}
pub(in crate::web::routes) fn fallback_logo_svg(provider: &str) -> String {
    let initial = provider.chars().next().unwrap_or('M').to_ascii_uppercase();
    format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 32 32" role="img" aria-label="{provider}"><rect width="32" height="32" rx="8" fill="#18181b"/><text x="16" y="21" text-anchor="middle" font-family="ui-sans-serif,system-ui" font-size="14" fill="#fafafa">{initial}</text></svg>"##
    )
}
