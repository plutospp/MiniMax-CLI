//! Automatic model discovery via provider Models list APIs.
//!
//! Queries the active provider's models endpoint (OpenAI-compatible
//! `GET /v1/models` or Anthropic-compatible `GET /v1/models`) so the model
//! picker can offer whatever the provider actually serves, instead of only
//! the built-in MiniMax catalog. Callers are expected to fall back to the
//! built-in catalog when discovery fails.

use std::time::Duration;

use anyhow::{Context, Result, bail};
use reqwest::header::{ACCEPT, AUTHORIZATION, CONTENT_TYPE, HeaderMap, HeaderValue};
use serde_json::Value;

use crate::config::{ActiveProvider, ProviderApi, is_minimax_host};
use crate::logging;

/// Upper bound for a discovery request so the TUI never hangs on a dead endpoint.
const DISCOVERY_TIMEOUT: Duration = Duration::from_secs(4);

/// A model entry returned by a provider's models endpoint.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiscoveredModel {
    /// Model ID to send in API requests.
    pub id: String,
    /// Human-facing name when the provider supplies one.
    pub display_name: Option<String>,
}

/// Discover the models offered by a provider via its models list endpoint.
///
/// Supports both OpenAI-shaped (`{"data": [{"id": ...}]}`) and
/// Anthropic-shaped (`{"data": [{"id", "display_name"}]}`) payloads, plus
/// plain string arrays returned by some proxies. Returns an error when the
/// endpoint is unreachable, responds with a non-success status, or yields no
/// usable IDs — callers should then fall back to the built-in catalog.
pub async fn discover_models(provider: &ActiveProvider) -> Result<Vec<DiscoveredModel>> {
    let (url, headers) = request_for(provider);
    logging::info(format!(
        "Discovering models for provider '{}' via {url}",
        provider.name
    ));

    let client = reqwest::Client::builder()
        .timeout(DISCOVERY_TIMEOUT)
        .default_headers(headers)
        .build()
        .context("Failed to build models discovery client")?;

    let response = client
        .get(&url)
        .send()
        .await
        .with_context(|| format!("Models request to {url} failed"))?;

    let status = response.status();
    if !status.is_success() {
        bail!("Models endpoint {url} returned {status}");
    }

    let body: Value = response
        .json()
        .await
        .with_context(|| format!("Models endpoint {url} returned invalid JSON"))?;

    let models = parse_models(&body);
    if models.is_empty() {
        bail!("Models endpoint {url} returned no usable model IDs");
    }

    logging::info(format!(
        "Discovered {} models for provider '{}'",
        models.len(),
        provider.name
    ));
    Ok(models)
}

/// Build the models URL and auth headers for a provider's API flavor.
fn request_for(provider: &ActiveProvider) -> (String, HeaderMap) {
    let mut headers = HeaderMap::new();
    headers.insert(ACCEPT, HeaderValue::from_static("application/json"));
    headers.insert(CONTENT_TYPE, HeaderValue::from_static("application/json"));

    match provider.api {
        ProviderApi::OpenAi => {
            headers.insert(
                AUTHORIZATION,
                HeaderValue::from_str(&format!("Bearer {}", provider.api_key))
                    .expect("API key contains invalid header characters"),
            );
            (provider.openai_models_url(), headers)
        }
        ProviderApi::Anthropic => {
            headers.insert(
                "x-api-key",
                HeaderValue::from_str(&provider.api_key)
                    .expect("API key contains invalid header characters"),
            );
            headers.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            if is_minimax_host(&provider.url) {
                // MiniMax's Anthropic-compatible surface also expects a bearer token.
                if let Ok(value) = HeaderValue::from_str(&format!("Bearer {}", provider.api_key)) {
                    headers.insert(AUTHORIZATION, value);
                }
            }
            (provider.anthropic_models_url(), headers)
        }
    }
}

/// Extract models from a models-endpoint payload.
///
/// Accepts `{"data": [...]}` wrappers (OpenAI/Anthropic), a bare top-level
/// array, and entries given either as objects with `id`/`display_name` or as
/// plain ID strings. Deduplicates (case-insensitive) and sorts by ID.
pub(crate) fn parse_models(body: &Value) -> Vec<DiscoveredModel> {
    let entries = body
        .get("data")
        .and_then(Value::as_array)
        .or_else(|| body.as_array())
        .map(Vec::as_slice)
        .unwrap_or_default();

    let mut models: Vec<DiscoveredModel> = entries
        .iter()
        .filter_map(|entry| match entry {
            Value::String(id) => Some(DiscoveredModel {
                id: id.trim().to_string(),
                display_name: None,
            }),
            Value::Object(map) => {
                let id = map.get("id")?.as_str()?.trim().to_string();
                let display_name = map
                    .get("display_name")
                    .and_then(Value::as_str)
                    .map(str::trim)
                    .filter(|name| !name.is_empty())
                    .map(str::to_string);
                Some(DiscoveredModel { id, display_name })
            }
            _ => None,
        })
        .filter(|model| !model.id.is_empty())
        .collect();

    models.sort_by_cached_key(|m| m.id.to_lowercase());
    models.dedup_by(|a, b| a.id.eq_ignore_ascii_case(&b.id));
    models
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use wiremock::matchers::{header, method, path};
    use wiremock::{Mock, MockServer, ResponseTemplate};

    fn openai_provider(base_url: String) -> ActiveProvider {
        ActiveProvider {
            name: "test-openai".to_string(),
            api: ProviderApi::OpenAi,
            url: base_url,
            api_key: "sk-test".to_string(),
            default_model: "gpt-4o".to_string(),
        }
    }

    fn anthropic_provider(base_url: String) -> ActiveProvider {
        ActiveProvider {
            name: "test-anthropic".to_string(),
            api: ProviderApi::Anthropic,
            url: base_url,
            api_key: "sk-ant".to_string(),
            default_model: "claude-3".to_string(),
        }
    }

    #[test]
    fn parse_openai_shape() {
        let body = json!({
            "object": "list",
            "data": [
                {"id": "gpt-4o", "object": "model"},
                {"id": "gpt-4o-mini", "object": "model"},
                {"id": "", "object": "model"},
            ]
        });
        let models = parse_models(&body);
        assert_eq!(
            models,
            vec![
                DiscoveredModel {
                    id: "gpt-4o".into(),
                    display_name: None
                },
                DiscoveredModel {
                    id: "gpt-4o-mini".into(),
                    display_name: None
                },
            ]
        );
    }

    #[test]
    fn parse_anthropic_shape_with_display_names() {
        let body = json!({
            "data": [
                {"id": "claude-3-5-sonnet", "display_name": "Claude 3.5 Sonnet"},
                {"id": "claude-3-haiku", "display_name": ""},
            ]
        });
        let models = parse_models(&body);
        assert_eq!(
            models,
            vec![
                DiscoveredModel {
                    id: "claude-3-5-sonnet".into(),
                    display_name: Some("Claude 3.5 Sonnet".into())
                },
                DiscoveredModel {
                    id: "claude-3-haiku".into(),
                    display_name: None
                },
            ]
        );
    }

    #[test]
    fn parse_plain_string_array_dedupes_and_sorts() {
        let body = json!(["zz-model", "Aa-model", "aa-model", "  ", "mm-model"]);
        let models = parse_models(&body);
        let ids: Vec<&str> = models.iter().map(|m| m.id.as_str()).collect();
        assert_eq!(ids, ["Aa-model", "mm-model", "zz-model"]);
    }

    #[test]
    fn parse_empty_or_unrecognized_payload() {
        assert!(parse_models(&json!({})).is_empty());
        assert!(parse_models(&json!({"data": []})).is_empty());
        assert!(parse_models(&json!({"error": "nope"})).is_empty());
    }

    #[tokio::test]
    async fn discover_openai_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("Authorization", "Bearer sk-test"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [{"id": "gpt-4o"}, {"id": "gpt-4o-mini"}]
            })))
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri());
        let models = discover_models(&provider).await.unwrap();
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].id, "gpt-4o");
    }

    #[tokio::test]
    async fn discover_anthropic_models_endpoint() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .and(header("x-api-key", "sk-ant"))
            .and(header("anthropic-version", "2023-06-01"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({
                "data": [
                    {"id": "claude-3-5-sonnet", "display_name": "Claude 3.5 Sonnet"}
                ]
            })))
            .mount(&server)
            .await;

        let provider = anthropic_provider(server.uri());
        let models = discover_models(&provider).await.unwrap();
        assert_eq!(models.len(), 1);
        assert_eq!(models[0].display_name.as_deref(), Some("Claude 3.5 Sonnet"));
    }

    #[tokio::test]
    async fn discover_errors_on_http_error() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri());
        let err = discover_models(&provider).await.unwrap_err().to_string();
        assert!(err.contains("404"), "unexpected error: {err}");
    }

    #[tokio::test]
    async fn discover_errors_on_empty_model_list() {
        let server = MockServer::start().await;
        Mock::given(method("GET"))
            .and(path("/v1/models"))
            .respond_with(ResponseTemplate::new(200).set_body_json(json!({"data": []})))
            .mount(&server)
            .await;

        let provider = openai_provider(server.uri());
        let err = discover_models(&provider).await.unwrap_err().to_string();
        assert!(
            err.contains("no usable model IDs"),
            "unexpected error: {err}"
        );
    }
}
