//! JSON Protocol Handler
//!
//! Handles AWS JSON-RPC protocol (used by DynamoDB, ECS, etc.)
//! - Request: POST with X-Amz-Target header, JSON body
//! - Response: JSON

use super::ProtocolHandler;
use crate::aws::client::AwsClients;
use crate::resource::path_extractor::{extract_by_path, extract_list};
use crate::resource::protocol::ApiConfig;
use anyhow::Result;
use serde_json::Value;
use std::sync::{Mutex, OnceLock};

/// Cached AWS account ID from one GetCallerIdentity round-trip. Billing APIs
/// key their data by account (Budgets DescribeBudgets requires AccountId), and
/// the account is constant for the lifetime of a credentials check. The cache
/// is process-global, matching how a single orbit instance is one user/account.
static ACCOUNT_ID_CACHE: OnceLock<Mutex<Option<String>>> = OnceLock::new();

/// Resolve `{{account_id}}` and the `{{today}}` / `{{today-Nd}}` placeholders
/// inside a request body. The billing APIs (Cost Explorer DateInterval, Budgets
/// AccountId) cannot be expressed as JSON resource definitions otherwise: the
/// account is only discoverable at runtime and the date window must move with
/// each fetch. Values that carry no placeholder pass through untouched.
///
/// Resolution happens just before the request leaves, so a page token (which
/// always wins in this handler) and any freshly-resolved static params agree.
fn resolve_templates(value: &mut Value, account_id: &str) {
    match value {
        Value::String(s) => {
            if let Some(resolved) = resolve_template_string(s, account_id) {
                *value = Value::String(resolved);
            }
        }
        Value::Array(items) => {
            for item in items {
                resolve_templates(item, account_id);
            }
        }
        Value::Object(map) => {
            for item in map.values_mut() {
                resolve_templates(item, account_id);
            }
        }
        _ => {}
    }
}

fn resolve_template_string(template: &str, account_id: &str) -> Option<String> {
    if !template.contains("{{") {
        return None;
    }
    if template == "{{account_id}}" {
        return Some(account_id.to_string());
    }
    if let Some(rest) = template.strip_prefix("{{today") {
        if let Some(suffix) = rest.strip_suffix("}}") {
            // "{{today}}" or "{{today-30d}}"
            let days: i64 = if suffix.is_empty() {
                0
            } else if let Some(num) = suffix.strip_prefix('-') {
                num.trim_end_matches('d').parse().unwrap_or(0)
            } else {
                0
            };
            let now = chrono::Utc::now();
            let day = now.date_naive() - chrono::Duration::days(days);
            // Normalize to midnight UTC so a paginated fetch keeps a stable
            // window instead of drifting by a day's worth of requests.
            return Some(format!("{}T00:00:00Z", day));
        }
    }
    None
}

/// Fetch the AWS account id for the current credentials via GetCallerIdentity,
/// cached after the first call.
async fn resolve_account_id(clients: &AwsClients) -> Result<String> {
    let cache = ACCOUNT_ID_CACHE.get_or_init(|| Mutex::new(None));

    {
        if let Some(id) = cache.lock().unwrap().as_ref() {
            return Ok(id.clone());
        }
    }

    let xml = clients
        .http
        .query_request("sts", "GetCallerIdentity", &[])
        .await?;
    let json = crate::aws::http::xml_to_json(&xml)?;
    let id = json
        .pointer("/GetCallerIdentityResponse/GetCallerIdentityResult/Account")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_default();

    if id.is_empty() {
        return Err(anyhow::anyhow!(
            "GetCallerIdentity returned no account id; billing resources need it"
        ));
    }

    *cache.lock().unwrap() = Some(id.clone());
    Ok(id)
}

/// Resolve the runtime-only placeholders a resource JSON declares. $body is
/// the full request body after static + dynamic + pagination params are merged.
async fn resolve_body_templates(
    clients: &AwsClients,
    body: &mut serde_json::Map<String, Value>,
) -> Result<()> {
    let needs_resolution = body.values().any(|v| v.to_string().contains("{{"));
    if needs_resolution {
        let account_id = resolve_account_id(clients).await?;
        let mut root = Value::Object(std::mem::take(body));
        resolve_templates(&mut root, &account_id);
        let Value::Object(map) = root else {
            unreachable!("root was constructed as an object")
        };
        *body = map;
    }
    Ok(())
}

pub struct JsonProtocolHandler;

impl JsonProtocolHandler {
    /// Execute the API request (async implementation)
    pub async fn execute_impl(
        &self,
        clients: &AwsClients,
        service: &str,
        config: &ApiConfig,
        params: &Value,
    ) -> Result<String> {
        let action = config
            .action
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("JSON protocol requires 'action' field"))?;

        // Build request body
        let mut body = serde_json::Map::new();

        // Add static params from config
        for (key, value) in &config.static_params {
            body.insert(key.clone(), value.clone());
        }

        // Add dynamic params (skip internal params starting with '_')
        if let Value::Object(map) = params {
            for (key, value) in map {
                if !key.starts_with('_') {
                    // Apply param_mapping if defined (e.g., "log_group_name" -> "logGroupName")
                    let mapped_key = config
                        .param_mapping
                        .get(key)
                        .cloned()
                        .unwrap_or_else(|| key.clone());

                    // Unwrap single-element arrays to single values
                    // This is needed because filters are passed as arrays, but JSON protocol
                    // APIs typically expect single values (e.g., logGroupName: "name" not ["name"])
                    let unwrapped_value = match value {
                        Value::Array(arr) if arr.len() == 1 => arr[0].clone(),
                        _ => value.clone(),
                    };

                    body.insert(mapped_key, unwrapped_value);
                }
            }
        }

        // Add pagination params last so a page token always wins over static/dynamic params
        if let Some(pagination) = &config.pagination {
            if let Some(max_param) = &pagination.max_results_param {
                let max_value = pagination.max_results.unwrap_or(100);
                body.insert(max_param.clone(), Value::from(max_value));
            }
            if let Some(token) = params.get("_page_token").and_then(|v| v.as_str()) {
                if let Some(input_token) = &pagination.input_token {
                    body.insert(input_token.clone(), Value::String(token.to_string()));
                }
            }
        }

        let body_str = {
            resolve_body_templates(clients, &mut body).await?;
            serde_json::to_string(&Value::Object(body))?
        };
        clients.http.json_request(service, action, &body_str).await
    }
}

impl ProtocolHandler for JsonProtocolHandler {
    fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)> {
        let json: Value = serde_json::from_str(response)?;

        // Extract items using response_root path
        let items = if let Some(root) = &config.response_root {
            extract_list(&json, root)
        } else {
            // If no response_root, try common keys
            if let Some(arr) = json.as_array() {
                arr.clone()
            } else {
                vec![json.clone()]
            }
        };

        // Extract next token if pagination is configured
        let next_token = config
            .pagination
            .as_ref()
            .and_then(|p| p.output_token.as_ref())
            .and_then(|path| {
                let token = extract_by_path(&json, path);
                token.as_str().map(|s| s.to_string())
            });

        Ok((items, next_token))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_dynamodb_response() {
        let handler = JsonProtocolHandler;

        let response = r#"{
            "TableNames": ["table1", "table2", "table3"]
        }"#;

        let config = ApiConfig {
            response_root: Some("/TableNames".to_string()),
            ..Default::default()
        };

        let (items, _) = handler.parse_items(response, &config).unwrap();
        assert_eq!(items.len(), 3);
        assert_eq!(items[0], "table1");
    }

    #[test]
    fn test_parse_ecs_clusters_response() {
        let response = r#"{
            "clusters": [
                {"clusterArn": "arn:aws:ecs:us-east-1:123:cluster/default", "status": "ACTIVE"},
                {"clusterArn": "arn:aws:ecs:us-east-1:123:cluster/prod", "status": "ACTIVE"}
            ]
        }"#;

        let config = ApiConfig {
            response_root: Some("/clusters".to_string()),
            ..Default::default()
        };

        let handler = JsonProtocolHandler;
        let (items, _) = handler.parse_items(response, &config).unwrap();

        assert_eq!(items.len(), 2);
        assert_eq!(
            items[0]["clusterArn"],
            "arn:aws:ecs:us-east-1:123:cluster/default"
        );
    }

    #[test]
    fn test_parse_with_pagination() {
        let response = r#"{
            "clusters": [{"name": "test"}],
            "nextToken": "abc123"
        }"#;

        let config = ApiConfig {
            response_root: Some("/clusters".to_string()),
            pagination: Some(crate::resource::protocol::PaginationConfig {
                output_token: Some("/nextToken".to_string()),
                ..Default::default()
            }),
            ..Default::default()
        };

        let handler = JsonProtocolHandler;
        let (items, next_token) = handler.parse_items(response, &config).unwrap();

        assert_eq!(items.len(), 1);
        assert_eq!(next_token, Some("abc123".to_string()));
    }

    /// Billing resources declare date/account placeholders in static_params;
    /// they must resolve to concrete values before the body is signed.
    #[test]
    fn resolve_templates_replaces_account_and_dates() {
        let mut value = serde_json::json!({
            "AccountId": "{{account_id}}",
            "DateInterval": { "Start": "{{today-30d}}", "End": "{{today}}" },
            "Filter": ["{{today-7d}}", "plain"]
        });

        resolve_templates(&mut value, "123456789012");

        assert_eq!(value["AccountId"], serde_json::json!("123456789012"));
        let start = value["DateInterval"]["Start"].as_str().unwrap();
        let end = value["DateInterval"]["End"].as_str().unwrap();
        let filter = value["Filter"][0].as_str().unwrap();
        assert_eq!(value["Filter"][1], serde_json::json!("plain"));

        // All three resolve to midnight-UTC YYYY-MM-DD strings, so they are
        // valid Cost Explorer DateInterval bounds, and the window is 30 days.
        for d in [start, end, filter] {
            assert!(
                chrono::DateTime::parse_from_rfc3339(d).is_ok(),
                "{d} must be a parseable ISO-8601 instant"
            );
        }
        let start_day = chrono::NaiveDate::parse_from_str(&start[..10], "%Y-%m-%d").unwrap();
        let end_day = chrono::NaiveDate::parse_from_str(&end[..10], "%Y-%m-%d").unwrap();
        assert_eq!(
            (end_day - start_day).num_days(),
            30,
            "the anomaly window must span exactly 30 days"
        );
    }

    /// The date window must be stable across a paginated fetch: both pages of
    /// a 60-day anomaly scan should request the same Start, or late pages
    /// could overlap earlier ones.
    #[test]
    fn resolve_templates_stable_across_two_resolutions() {
        let mut first = serde_json::json!("{{today-30d}}");
        let mut second = serde_json::json!("{{today-30d}}");
        resolve_templates(&mut first, "1");
        resolve_templates(&mut second, "1");
        assert_eq!(first, second);
    }

    /// A value that carries no placeholder must pass through byte-for-byte, so
    /// ordinary JSON-RPC bodies never touch the resolver.
    #[test]
    fn resolve_templates_leaves_plain_values_alone() {
        let mut value = serde_json::json!({ "orderBy": "LastEventTime", "limit": 50 });
        resolve_templates(&mut value, "123");
        assert_eq!(
            value,
            serde_json::json!({ "orderBy": "LastEventTime", "limit": 50 })
        );
        assert_eq!(resolve_template_string("plain", "123"), None);
    }
}
