//! AWS API Dispatcher
//!
//! This module handles all AWS API dispatching:
//! - List operations via JSON config
//! - Actions (write operations like start/stop/delete)
//! - Describe (single resource details)
//!
//! API operations are configured in JSON files under src/resources/.
//! Special cases (S3 objects, STS) have dedicated handlers.

use super::field_mapper::build_response;
use super::handlers::get_protocol_handler;
use super::protocol::ApiProtocol;
use super::registry::get_resource;
use crate::aws::client::AwsClients;
use crate::aws::http::xml_to_json;
use anyhow::{anyhow, Result};
use serde_json::{json, Value};
use std::collections::HashMap;
use tracing::{debug, warn};

// =============================================================================
// Helper Functions
// =============================================================================

/// Extract a single string parameter from Value
fn extract_param(params: &Value, key: &str) -> String {
    params
        .get(key)
        .and_then(|v| {
            v.as_str().map(|s| s.to_string()).or_else(|| {
                v.as_array()
                    .and_then(|a| a.first())
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string())
            })
        })
        .unwrap_or_default()
}

/// Format bytes into human-readable format
fn format_bytes(bytes: u64) -> String {
    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    }
}

/// Format epoch milliseconds to human-readable date string
fn format_epoch_millis(millis: i64) -> String {
    use chrono::{TimeZone, Utc};

    if millis <= 0 {
        return "-".to_string();
    }

    Utc.timestamp_millis_opt(millis)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string())
}

/// Resolve template variables in static param values: {resource_id}, {timestamp}
fn resolve_static_param_template(template: &str, resource_id: &str, timestamp: &str) -> String {
    template
        .replace("{resource_id}", resource_id)
        .replace("{timestamp}", timestamp)
}

/// Format epoch milliseconds to human-readable date string (public for log tail UI)
pub fn format_log_timestamp(millis: i64) -> String {
    format_epoch_millis(millis)
}

// =============================================================================
// Data-Driven List Operations
// =============================================================================

/// Invoke an AWS list API using JSON configuration
///
/// This function reads the API configuration from the resource definition
/// and uses the appropriate protocol handler to execute the request.
pub async fn invoke_list(
    resource_key: &str,
    clients: &AwsClients,
    params: &Value,
) -> Result<Value> {
    let resource_def =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    let api_config = resource_def
        .api_config
        .as_ref()
        .ok_or_else(|| anyhow!("Resource {} does not have api_config", resource_key))?;

    let handler = get_protocol_handler(api_config.protocol);

    let service = api_config
        .service_name
        .as_deref()
        .unwrap_or(&resource_def.service);

    let parsed = handler
        .invoke(
            clients,
            service,
            api_config,
            params,
            &resource_def.field_mappings,
        )
        .await?;

    Ok(build_response(
        parsed.items,
        &resource_def.response_path,
        parsed.next_token,
    ))
}

// =============================================================================
// Legacy List Operations (special cases)
// =============================================================================

/// Invoke an AWS API method for special cases (S3 objects, STS, CloudWatch logs).
/// Most list operations should use invoke_list instead.
pub async fn invoke_sdk(
    service: &str,
    method: &str,
    clients: &AwsClients,
    params: &Value,
) -> Result<Value> {
    match (service, method) {
        // S3 list_objects_v2 - requires bucket region resolution and complex folder handling
        ("s3", "list_objects_v2") => {
            let bucket = extract_param(params, "bucket_names");
            if bucket.is_empty() {
                return Err(anyhow!("Bucket name required"));
            }

            let prefix = params
                .get("prefix")
                .map(|v| {
                    if let Some(s) = v.as_str() {
                        s.to_string()
                    } else if let Some(arr) = v.as_array() {
                        arr.first()
                            .and_then(|v| v.as_str())
                            .unwrap_or("")
                            .to_string()
                    } else {
                        String::new()
                    }
                })
                .unwrap_or_default();

            let bucket_region = clients.http.get_bucket_region(&bucket).await?;
            debug!("Bucket {} is in region {}", bucket, bucket_region);

            let path = if prefix.is_empty() {
                "?list-type=2&delimiter=/".to_string()
            } else {
                format!(
                    "?list-type=2&delimiter=/&prefix={}",
                    urlencoding::encode(&prefix)
                )
            };

            let xml = clients
                .http
                .rest_xml_request_s3_bucket("GET", &bucket, &path, None, &bucket_region)
                .await?;
            let json = xml_to_json(&xml)?;

            let mut objects: Vec<Value> = vec![];

            // Add common prefixes (folders)
            if let Some(prefixes) = json.pointer("/ListBucketResult/CommonPrefixes") {
                let prefix_list = match prefixes {
                    Value::Array(arr) => arr.clone(),
                    obj @ Value::Object(_) => vec![obj.clone()],
                    _ => vec![],
                };
                for p in prefix_list {
                    let prefix_val = p.pointer("/Prefix").and_then(|v| v.as_str()).unwrap_or("-");
                    let display_name = prefix_val
                        .trim_end_matches('/')
                        .rsplit('/')
                        .next()
                        .unwrap_or(prefix_val);
                    objects.push(json!({
                        "Key": prefix_val,
                        "DisplayName": format!("{}/", display_name),
                        "Size": "-",
                        "LastModified": "-",
                        "StorageClass": "FOLDER",
                        "IsFolder": true
                    }));
                }
            }

            // Add objects (files)
            if let Some(contents) = json.pointer("/ListBucketResult/Contents") {
                let content_list = match contents {
                    Value::Array(arr) => arr.clone(),
                    obj @ Value::Object(_) => vec![obj.clone()],
                    _ => vec![],
                };
                for obj in content_list {
                    let key = obj.pointer("/Key").and_then(|v| v.as_str()).unwrap_or("-");
                    if key == prefix {
                        continue;
                    }
                    let display_name = key.rsplit('/').next().unwrap_or(key);
                    let size = obj.pointer("/Size").and_then(|v| v.as_str()).unwrap_or("0");
                    let size_bytes = size.parse::<u64>().unwrap_or(0);
                    let size_formatted = format_bytes(size_bytes);
                    objects.push(json!({
                        "Key": key,
                        "DisplayName": display_name,
                        "Size": size_formatted,
                        // Raw byte count kept alongside the display string so the
                        // download size guard doesn't have to parse "1.2 KB" back
                        "SizeBytes": size_bytes,
                        "LastModified": obj.pointer("/LastModified").and_then(|v| v.as_str()).unwrap_or("-"),
                        "StorageClass": obj.pointer("/StorageClass").and_then(|v| v.as_str()).unwrap_or("STANDARD"),
                        "IsFolder": false
                    }));
                }
            }

            Ok(json!({ "objects": objects }))
        }

        // STS get_caller_identity - returns single item, not a list
        ("sts", "get_caller_identity") => {
            let xml = clients
                .http
                .query_request("sts", "GetCallerIdentity", &[])
                .await?;
            let json = xml_to_json(&xml)?;

            let result_path = json.pointer("/GetCallerIdentityResponse/GetCallerIdentityResult");
            let identity = json!({
                "Account": result_path.and_then(|r| r.pointer("/Account")).and_then(|v| v.as_str()).unwrap_or("-"),
                "UserId": result_path.and_then(|r| r.pointer("/UserId")).and_then(|v| v.as_str()).unwrap_or("-"),
                "Arn": result_path.and_then(|r| r.pointer("/Arn")).and_then(|v| v.as_str()).unwrap_or("-"),
            });

            Ok(json!({ "identity": [identity] }))
        }

        // CloudWatch Logs - tail_logs (streaming operation)
        ("cloudwatchlogs", "tail_logs") => {
            let log_group = extract_param(params, "log_group_name");
            let log_stream = extract_param(params, "log_stream_name");

            if log_group.is_empty() || log_stream.is_empty() {
                return Err(anyhow!("Log group and stream names required"));
            }

            let request_body = json!({
                "logGroupName": log_group,
                "logStreamName": log_stream,
                "startFromHead": false,
                "limit": 100
            })
            .to_string();

            let response = clients
                .http
                .json_request("logs", "GetLogEvents", &request_body)
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            let events = json
                .get("events")
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let result: Vec<Value> = events
                .iter()
                .map(|e| {
                    let timestamp = e.get("timestamp").and_then(|v| v.as_i64()).unwrap_or(0);
                    json!({
                        "timestamp": format_epoch_millis(timestamp),
                        "message": e.get("message").and_then(|v| v.as_str()).unwrap_or("-"),
                    })
                })
                .collect();

            Ok(json!({ "events": result }))
        }

        // CloudWatch Logs - get_log_events (for log tailing UI)
        ("cloudwatchlogs", "get_log_events") => {
            let log_group = extract_param(params, "log_group_name");
            let log_stream = extract_param(params, "log_stream_name");

            if log_group.is_empty() || log_stream.is_empty() {
                return Err(anyhow!("Log group and stream names required"));
            }

            let mut request = json!({
                "logGroupName": log_group,
                "logStreamName": log_stream,
                "startFromHead": false,
                "limit": 100
            });

            // Add next token if provided
            if let Some(token) = params.get("next_forward_token").and_then(|v| v.as_str()) {
                request["nextToken"] = json!(token);
            }

            let response = clients
                .http
                .json_request("logs", "GetLogEvents", &request.to_string())
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            Ok(json)
        }

        _ => Err(anyhow!(
            "Operation not handled: service='{}', method='{}'. Configure it in the resource JSON.",
            service,
            method
        )),
    }
}

// =============================================================================
// Data-Driven Action Execution
// =============================================================================

/// Execute an action using JSON configuration
async fn invoke_action(
    resource_key: &str,
    action_id: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<()> {
    let resource_def =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    let action_config = resource_def
        .action_configs
        .get(action_id)
        .ok_or_else(|| anyhow!("Action '{}' not configured for {}", action_id, resource_key))?;

    let service = action_config
        .service_name
        .as_deref()
        .unwrap_or(&resource_def.service);

    debug!(
        "Executing action: {} on {} (service: {}, protocol: {:?})",
        action_id, resource_key, service, action_config.protocol
    );

    match action_config.protocol {
        ApiProtocol::Query => {
            let action_name = action_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("Query action requires 'action' field"))?;

            let mut params_owned: Vec<(String, String)> = Vec::new();

            // Handle special formats
            if action_config.special_handling.as_deref() == Some("parse_pipe_format_tg_target") {
                // Format: target_group_arn|target_id
                let parts: Vec<&str> = resource_id.split('|').collect();
                if parts.len() != 2 {
                    return Err(anyhow!(
                        "Invalid target format, expected target_group_arn|target_id"
                    ));
                }
                params_owned.push(("TargetGroupArn".to_string(), parts[0].to_string()));
                params_owned.push(("Targets.member.1.Id".to_string(), parts[1].to_string()));
            } else {
                // Add resource ID parameter
                if let Some(ref id_param) = action_config.id_param {
                    params_owned.push((id_param.clone(), resource_id.to_string()));
                }
            }

            // Add static parameters
            // Resolve template variables in static params: {resource_id}, {timestamp}
            let current_timestamp = chrono::Utc::now().format("%Y%m%dT%H%M%S").to_string();

            for (key, value) in &action_config.static_params {
                if let Some(template) = value.as_str() {
                    let resolved =
                        resolve_static_param_template(template, resource_id, &current_timestamp);
                    params_owned.push((key.clone(), resolved));
                }
            }

            let params_ref: Vec<(&str, &str)> = params_owned
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            clients
                .http
                .query_request(service, action_name, &params_ref)
                .await?;
            Ok(())
        }

        ApiProtocol::Json => {
            let action_name = action_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("JSON action requires 'action' field"))?;

            let body = if let Some(ref template) = action_config.body_template {
                // Handle special ARN parsing if needed
                let actual_id =
                    if action_config.special_handling.as_deref() == Some("parse_arn_for_cluster") {
                        // Extract cluster from ARN like arn:aws:ecs:region:account:service/cluster/service-name
                        let parts: Vec<&str> = resource_id.split('/').collect();
                        if parts.len() >= 2 {
                            parts[parts.len() - 2].to_string()
                        } else {
                            resource_id.to_string()
                        }
                    } else {
                        resource_id.to_string()
                    };

                template
                    .replace("{resource_id}", &actual_id)
                    .replace("{cluster}", {
                        let parts: Vec<&str> = resource_id.split('/').collect();
                        if parts.len() >= 2 {
                            parts[parts.len() - 2]
                        } else {
                            resource_id
                        }
                    })
            } else {
                // Build body from id_param
                let id_param = action_config.id_param.as_deref().unwrap_or("id");
                json!({ id_param: resource_id }).to_string()
            };

            clients
                .http
                .json_request(service, action_name, &body)
                .await?;
            Ok(())
        }

        ApiProtocol::RestJson => {
            let method = action_config.method.as_deref().unwrap_or("DELETE");
            let path_template = action_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-JSON action requires 'path' field"))?;

            let path = path_template.replace("{resource_id}", resource_id);
            let body = action_config.body_template.as_deref();

            clients
                .http
                .rest_json_request(service, method, &path, body)
                .await?;
            Ok(())
        }

        ApiProtocol::RestXml => {
            let method = action_config.method.as_deref().unwrap_or("DELETE");
            let path_template = action_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-XML action requires 'path' field"))?;

            let path = path_template.replace("{resource_id}", resource_id);

            clients
                .http
                .rest_xml_request(service, method, &path, None)
                .await?;
            Ok(())
        }
    }
}

// =============================================================================
// Data-Driven Describe
// =============================================================================

/// Fill a describe `body_template`.
///
/// `{resource_id}` is the row's id field. `{arn_name}` and `{arn_id}` are the last
/// two segments of an ARN, which is how to reach an API that wants a name and an id
/// together: WAFv2's GetWebACL takes Name, Id and Scope, and the list call hands back
/// only an ARN carrying both.
fn render_describe_body(template: &str, resource_id: &str) -> Result<String> {
    let mut body = template.replace("{resource_id}", resource_id);

    if body.contains("{arn_name}") || body.contains("{arn_id}") {
        let (name, id) = arn_name_and_id(resource_id)?;
        body = body.replace("{arn_name}", name).replace("{arn_id}", id);
    }

    Ok(body)
}

/// Name and id out of an ARN shaped like
/// `arn:aws:wafv2:eu-west-1:123456789012:regional/webacl/<name>/<id>`.
///
/// Checks the whole shape rather than just counting back two segments, so a bare id
/// or a truncated ARN is refused instead of yielding two nonsense values.
fn arn_name_and_id(arn: &str) -> Result<(&str, &str)> {
    let segments: Vec<&str> = arn.split('/').collect();

    if !arn.starts_with("arn:") || segments.len() < 4 || segments.iter().any(|s| s.is_empty()) {
        return Err(anyhow!(
            "cannot read a name and id out of {:?}: expected an ARN ending in /<name>/<id>",
            arn
        ));
    }

    Ok((segments[segments.len() - 2], segments[segments.len() - 1]))
}

/// Describe a single resource using JSON configuration
async fn invoke_describe(
    resource_key: &str,
    clients: &AwsClients,
    resource_id: &str,
    parent_params: &HashMap<String, String>,
) -> Result<Value> {
    let resource_def =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    let describe_config = resource_def
        .describe_config
        .as_ref()
        .ok_or_else(|| anyhow!("Describe not configured for {}", resource_key))?;

    let service = describe_config
        .service_name
        .as_deref()
        .unwrap_or(&resource_def.service);

    debug!(
        "Describing resource: {} with id: {} (service: {}, protocol: {:?})",
        resource_key, resource_id, service, describe_config.protocol
    );

    let mut result = match describe_config.protocol {
        ApiProtocol::Query => {
            let action_name = describe_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("Query describe requires 'action' field"))?;

            let id_param = describe_config.id_param.as_deref().unwrap_or("Id");
            let xml = clients
                .http
                .query_request(service, action_name, &[(id_param, resource_id)])
                .await?;
            let json = xml_to_json(&xml)?;

            // Extract from response path
            if let Some(ref path) = describe_config.response_path {
                extract_single_item(&json, path)?
            } else {
                json
            }
        }

        ApiProtocol::Json => {
            let action_name = describe_config
                .action
                .as_ref()
                .ok_or_else(|| anyhow!("JSON describe requires 'action' field"))?;

            let body = if let Some(ref template) = describe_config.body_template {
                render_describe_body(template, resource_id)?
            } else {
                let id_param = describe_config.id_param.as_deref().unwrap_or("id");
                json!({ id_param: resource_id }).to_string()
            };

            let response = clients
                .http
                .json_request(service, action_name, &body)
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            if let Some(ref path) = describe_config.response_path {
                json.pointer(path).cloned().unwrap_or(json)
            } else {
                json
            }
        }

        ApiProtocol::RestJson => {
            let method = describe_config.method.as_deref().unwrap_or("GET");
            let path_template = describe_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-JSON describe requires 'path' field"))?;

            let mut path = path_template.replace("{resource_id}", resource_id);
            for (key, value) in parent_params {
                path = path.replace(&format!("{{{}}}", key), value);
            }
            let response = clients
                .http
                .rest_json_request(service, method, &path, None)
                .await?;
            let json: Value = serde_json::from_str(&response)?;

            if let Some(ref resp_path) = describe_config.response_path {
                json.pointer(resp_path).cloned().unwrap_or(json)
            } else {
                json
            }
        }

        ApiProtocol::RestXml => {
            let method = describe_config.method.as_deref().unwrap_or("GET");
            let path_template = describe_config
                .path
                .as_ref()
                .ok_or_else(|| anyhow!("REST-XML describe requires 'path' field"))?;

            let mut path = path_template.replace("{resource_id}", resource_id);
            for (key, value) in parent_params {
                path = path.replace(&format!("{{{}}}", key), value);
            }
            let xml = clients
                .http
                .rest_xml_request(service, method, &path, None)
                .await?;
            let json = xml_to_json(&xml)?;

            if let Some(ref resp_path) = describe_config.response_path {
                json.pointer(resp_path).cloned().unwrap_or(json)
            } else {
                json
            }
        }
    };

    // Handle enrich calls (additional API calls to add more data). Each one is
    // templated against the primary response, so it sees the describe as it
    // arrived and never a field an earlier enrich call added.
    let primary = result.clone();
    for enrich in &describe_config.enrich_calls {
        let enrich_result = execute_enrich_call(
            clients,
            service,
            resource_id,
            &primary,
            enrich,
            describe_config.protocol,
        )
        .await;
        match enrich_result {
            Ok(value) => {
                if let Value::Object(ref mut map) = result {
                    map.insert(enrich.result_field.clone(), value);
                }
            }
            Err(err) => {
                // A swallowed enrich failure renders as an empty row that looks
                // like "AWS says there are none of these". Say so in the log.
                warn!(
                    "Describe enrichment {} for {} failed: {}",
                    enrich.result_field, resource_id, err
                );
                if let Some(ref default) = enrich.default_value {
                    if let Value::Object(ref mut map) = result {
                        map.insert(enrich.result_field.clone(), json!(default));
                    }
                }
            }
        }
    }

    Ok(result)
}

/// Resolve one enrich param or body template.
///
/// `{resource_id}` is the row's id. `{/Json/Pointer}` reads the describe
/// response that has already been fetched, which is the only place the ids these
/// calls need appear: ListTagsForResource wants an ARN, EC2 wants the attached
/// security group ids, and the row carries neither. `{now}` and `{now-15m}` are
/// ISO8601 UTC, for CloudWatch windows.
///
/// An unresolvable token is an error, not an empty string: sending a literal
/// `{/DBInstanceArn}` to AWS earns a rejection that reads like the resource's
/// fault.
fn resolve_enrich_template(
    template: &str,
    resource_id: &str,
    primary: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String> {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;

    while let Some(start) = rest.find('{') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after
            .find('}')
            .ok_or_else(|| anyhow!("unterminated template token in {:?}", template))?;
        out.push_str(&resolve_enrich_token(
            &after[..end],
            resource_id,
            primary,
            now,
        )?);
        rest = &after[end + 1..];
    }
    out.push_str(rest);

    Ok(out)
}

fn resolve_enrich_token(
    token: &str,
    resource_id: &str,
    primary: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<String> {
    if token == "resource_id" {
        return Ok(resource_id.to_string());
    }

    if token.starts_with('/') {
        let value = super::path_extractor::extract_by_path(primary, token);
        return enrich_scalar(&value)
            .ok_or_else(|| anyhow!("describe response carries no value at {}", token));
    }

    if let Some(offset) = token.strip_prefix("now") {
        let at = now - parse_enrich_offset(offset, token)?;
        return Ok(at.to_rfc3339_opts(chrono::SecondsFormat::Secs, true));
    }

    Err(anyhow!("unknown enrich template token {{{}}}", token))
}

/// `""` is now; `-15m`, `-2h`, `-1d`, `-30s` are offsets back from it.
fn parse_enrich_offset(offset: &str, token: &str) -> Result<chrono::Duration> {
    if offset.is_empty() {
        return Ok(chrono::Duration::zero());
    }

    let spec = offset.strip_prefix('-').ok_or_else(|| {
        anyhow!(
            "enrich template token {{{}}} must offset backwards, e.g. {{now-15m}}",
            token
        )
    })?;
    let (digits, unit) =
        spec.split_at(spec.find(|c: char| !c.is_ascii_digit()).ok_or_else(|| {
            anyhow!(
                "enrich template token {{{}}} needs a unit: s, m, h or d",
                token
            )
        })?);
    let amount: i64 = digits
        .parse()
        .map_err(|_| anyhow!("enrich template token {{{}}} has no leading number", token))?;

    match unit {
        "s" => Ok(chrono::Duration::seconds(amount)),
        "m" => Ok(chrono::Duration::minutes(amount)),
        "h" => Ok(chrono::Duration::hours(amount)),
        "d" => Ok(chrono::Duration::days(amount)),
        other => Err(anyhow!(
            "enrich template token {{{}}} has unknown unit {:?}, expected s, m, h or d",
            token,
            other
        )),
    }
}

/// A single param value out of an extracted path. XML→JSON collapses a
/// one-element list to a bare object, so an array here means the path genuinely
/// matched several items and the first is the one a scalar param can carry.
fn enrich_scalar(value: &Value) -> Option<String> {
    match value {
        Value::String(s) => Some(s.clone()),
        Value::Number(n) => Some(n.to_string()),
        Value::Bool(b) => Some(b.to_string()),
        Value::Array(items) => items.iter().find_map(enrich_scalar),
        Value::Object(_) | Value::Null => None,
    }
}

/// Query params for an enrich call: templated params first, then any filters in
/// EC2's `Filter.N.Name` / `Filter.N.Value.M` form.
fn build_enrich_query_params(
    enrich: &super::protocol::EnrichCall,
    resource_id: &str,
    primary: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Vec<(String, String)>> {
    let mut params: Vec<(String, String)> = Vec::new();

    for (key, template) in &enrich.params {
        params.push((
            key.clone(),
            resolve_enrich_template(template, resource_id, primary, now)?,
        ));
    }

    for (index, filter) in enrich.filters.iter().enumerate() {
        let values = enrich_filter_values(primary, &filter.values_source);
        // Dropping an empty filter would widen the call to the whole account and
        // present the result as this resource's own.
        if values.is_empty() {
            return Err(anyhow!(
                "filter {} has no values at {} in the describe response",
                filter.name,
                filter.values_source
            ));
        }

        let n = index + 1;
        params.push((format!("Filter.{}.Name", n), filter.name.clone()));
        for (i, value) in values.iter().enumerate() {
            params.push((format!("Filter.{}.Value.{}", n, i + 1), value.clone()));
        }
    }

    Ok(params)
}

/// Resolve the template tokens in every string leaf of a JSON enrich body.
fn resolve_enrich_body(
    body: &Value,
    resource_id: &str,
    primary: &Value,
    now: chrono::DateTime<chrono::Utc>,
) -> Result<Value> {
    match body {
        Value::String(s) => Ok(Value::String(resolve_enrich_template(
            s,
            resource_id,
            primary,
            now,
        )?)),
        Value::Array(items) => items
            .iter()
            .map(|item| resolve_enrich_body(item, resource_id, primary, now))
            .collect::<Result<Vec<Value>>>()
            .map(Value::Array),
        Value::Object(fields) => fields
            .iter()
            .map(|(key, value)| {
                resolve_enrich_body(value, resource_id, primary, now).map(|v| (key.clone(), v))
            })
            .collect::<Result<serde_json::Map<String, Value>>>()
            .map(Value::Object),
        other => Ok(other.clone()),
    }
}

fn enrich_filter_values(primary: &Value, source: &str) -> Vec<String> {
    match super::path_extractor::extract_by_path(primary, source) {
        Value::Array(items) => items.iter().filter_map(enrich_scalar).collect(),
        other => enrich_scalar(&other).into_iter().collect(),
    }
}

/// Execute an enrichment call for describe
async fn execute_enrich_call(
    clients: &AwsClients,
    default_service: &str,
    resource_id: &str,
    primary: &Value,
    enrich: &super::protocol::EnrichCall,
    default_protocol: ApiProtocol,
) -> Result<Value> {
    let service = enrich.service.as_deref().unwrap_or(default_service);
    let protocol = enrich.protocol.unwrap_or(default_protocol);
    let now = chrono::Utc::now();

    match protocol {
        ApiProtocol::Query => {
            let action = enrich.action.as_deref().ok_or_else(|| {
                anyhow!(
                    "query enrich call {} requires 'action'",
                    enrich.result_field
                )
            })?;
            let params = build_enrich_query_params(enrich, resource_id, primary, now)?;
            let param_refs: Vec<(&str, &str)> = params
                .iter()
                .map(|(k, v)| (k.as_str(), v.as_str()))
                .collect();

            let xml = clients
                .http
                .query_request(service, action, &param_refs)
                .await?;
            let json = xml_to_json(&xml)?;

            // extract_by_path, not pointer: XML lists collapse to a bare object
            // when they hold one item, and pointer indexing cannot see through
            // that.
            extract_enrich_result(
                super::path_extractor::extract_by_path(
                    &json,
                    enrich.extract_path.as_deref().unwrap_or("/"),
                ),
                enrich,
            )
        }

        ApiProtocol::Json => {
            let action = enrich.action.as_deref().ok_or_else(|| {
                anyhow!("json enrich call {} requires 'action'", enrich.result_field)
            })?;
            let body = enrich.body.as_ref().ok_or_else(|| {
                anyhow!("json enrich call {} requires 'body'", enrich.result_field)
            })?;
            let body = resolve_enrich_body(body, resource_id, primary, now)?.to_string();

            let response = clients.http.json_request(service, action, &body).await?;
            let json: Value = serde_json::from_str(&response)?;

            extract_enrich_result(
                pointer_or_whole(&json, enrich.extract_path.as_deref()),
                enrich,
            )
        }

        ApiProtocol::RestJson | ApiProtocol::RestXml => {
            let path_template = enrich.path.as_ref().ok_or_else(|| {
                anyhow!("REST enrich call {} requires 'path'", enrich.result_field)
            })?;
            let path = resolve_enrich_template(path_template, resource_id, primary, now)?;
            let method = enrich.method.as_deref().unwrap_or("GET");

            let json = if protocol == ApiProtocol::RestJson {
                let response = clients
                    .http
                    .rest_json_request(service, method, &path, None)
                    .await?;
                serde_json::from_str(&response)?
            } else {
                let xml = clients
                    .http
                    .rest_xml_request(service, method, &path, None)
                    .await?;
                xml_to_json(&xml)?
            };

            extract_enrich_result(
                pointer_or_whole(&json, enrich.extract_path.as_deref()),
                enrich,
            )
        }
    }
}

fn pointer_or_whole(json: &Value, extract_path: Option<&str>) -> Value {
    match extract_path {
        Some(path) => json.pointer(path).cloned().unwrap_or(Value::Null),
        None => json.clone(),
    }
}

/// A missing enrichment is an error so the caller can fall back to
/// `default_value` and log, rather than storing a null that renders as blank.
fn extract_enrich_result(value: Value, enrich: &super::protocol::EnrichCall) -> Result<Value> {
    if value.is_null() {
        return Err(anyhow!(
            "response carries nothing at {:?} for {}",
            enrich.extract_path.as_deref().unwrap_or("/"),
            enrich.result_field
        ));
    }
    Ok(value)
}

/// Extract a single item from a response that may be array or object
fn extract_single_item(json: &Value, path: &str) -> Result<Value> {
    let value = json
        .pointer(path)
        .ok_or_else(|| anyhow!("Response path not found: {}", path))?;

    match value {
        Value::Array(arr) => arr
            .first()
            .cloned()
            .ok_or_else(|| anyhow!("Empty response")),
        obj @ Value::Object(_) => Ok(obj.clone()),
        _ => Ok(value.clone()),
    }
}

// =============================================================================
// Unified Action Execution
// =============================================================================

/// Execute an action on a resource (start, stop, terminate, etc.)
/// Uses JSON config to execute the action.
pub async fn execute_action(
    service: &str,
    action: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<()> {
    let (resource_key, _) = find_resource_with_action(service, action).ok_or_else(|| {
        anyhow!(
            "Action '{}' not configured for service '{}'. Add action_configs to the resource JSON.",
            action,
            service
        )
    })?;

    invoke_action(&resource_key, action, clients, resource_id).await
}

/// Execute an action that returns data to display (e.g., get_secret_value)
/// These are read-only operations that retrieve and display data.
pub async fn execute_action_with_result(
    service: &str,
    action: &str,
    clients: &AwsClients,
    resource_id: &str,
) -> Result<Value> {
    match (service, action) {
        // Secrets Manager - Get Secret Value
        ("secretsmanager", "get_secret_value") => {
            let response = clients
                .http
                .json_request(
                    "secretsmanager",
                    "GetSecretValue",
                    &json!({
                        "SecretId": resource_id
                    })
                    .to_string(),
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json)
        }

        // SSM - Get Parameter Value (with decryption for SecureString)
        ("ssm", "get_parameter") => {
            let response = clients
                .http
                .json_request(
                    "ssm",
                    "GetParameter",
                    &json!({
                        "Name": resource_id,
                        "WithDecryption": true
                    })
                    .to_string(),
                )
                .await?;
            let json: Value = serde_json::from_str(&response)?;
            Ok(json)
        }

        _ => Err(anyhow!(
            "Unknown action with result: {}.{}",
            service,
            action
        )),
    }
}

/// Find a resource that has the given action configured
fn find_resource_with_action(
    service: &str,
    action_id: &str,
) -> Option<(String, &'static super::registry::ResourceDef)> {
    use super::registry::get_registry;

    for (key, resource) in &get_registry().resources {
        if resource.service == service && resource.action_configs.contains_key(action_id) {
            return Some((key.clone(), resource));
        }
    }
    None
}

// =============================================================================
// Describe Function
// =============================================================================

/// Fetch full details for a single resource by ID
/// Uses JSON config, with special handling for S3 buckets.
pub async fn describe_resource(
    resource_key: &str,
    clients: &AwsClients,
    resource_id: &str,
    parent_params: &HashMap<String, String>,
) -> Result<Value> {
    // S3 buckets need special handling for region resolution
    if resource_key == "s3-buckets" {
        return describe_s3_bucket(clients, resource_id).await;
    }

    let resource =
        get_resource(resource_key).ok_or_else(|| anyhow!("Unknown resource: {}", resource_key))?;

    if resource.describe_config.is_none() {
        return Err(anyhow!(
            "Describe not configured for '{}'. Add describe_config to the resource JSON.",
            resource_key
        ));
    }

    invoke_describe(resource_key, clients, resource_id, parent_params).await
}

/// Special handler for S3 bucket describe (needs region resolution)
async fn describe_s3_bucket(clients: &AwsClients, bucket_name: &str) -> Result<Value> {
    let mut result = json!({
        "BucketName": bucket_name,
    });

    // Get bucket location first (this determines the region for other calls)
    let bucket_region = clients
        .http
        .get_bucket_region(bucket_name)
        .await
        .unwrap_or_else(|_| "us-east-1".to_string());
    result["Region"] = json!(&bucket_region);

    // Get bucket versioning
    if let Ok(xml) = clients
        .http
        .rest_xml_request_s3_bucket("GET", bucket_name, "?versioning", None, &bucket_region)
        .await
    {
        if let Ok(json) = xml_to_json(&xml) {
            let status = json
                .pointer("/VersioningConfiguration/Status")
                .and_then(|v| v.as_str())
                .unwrap_or("Disabled");
            result["Versioning"] = json!(status);
        }
    }

    // Get bucket encryption
    if let Ok(xml) = clients
        .http
        .rest_xml_request_s3_bucket("GET", bucket_name, "?encryption", None, &bucket_region)
        .await
    {
        if let Ok(json) = xml_to_json(&xml) {
            if let Some(rules) = json.pointer("/ServerSideEncryptionConfiguration/Rule") {
                result["Encryption"] = rules.clone();
            }
        }
    } else {
        result["Encryption"] = json!("None");
    }

    Ok(result)
}

// =============================================================================
// Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::resource::protocol::{EnrichCall, EnrichFilter};

    #[test]
    fn test_unknown_resource_has_no_api_config() {
        assert!(get_resource("nonexistent-resource").is_none());
    }

    #[test]
    fn test_dynamodb_tables_has_api_config() {
        let resource = get_resource("dynamodb-tables").unwrap();
        assert!(resource.has_api_config());
    }

    #[test]
    fn test_ec2_instances_has_api_config() {
        let resource = get_resource("ec2-instances").unwrap();
        assert!(resource.has_api_config());
    }

    #[test]
    fn test_lambda_functions_has_api_config() {
        let resource = get_resource("lambda-functions").unwrap();
        assert!(resource.has_api_config());
    }

    #[test]
    fn test_iam_users_has_api_config() {
        let resource = get_resource("iam-users").unwrap();
        assert!(resource.has_api_config());
    }

    #[test]
    fn test_redshift_clusters_has_api_config() {
        let resource = get_resource("redshift-clusters").unwrap();
        assert!(resource.has_api_config());
    }

    #[test]
    fn test_resolve_static_param_template_replaces_all_placeholders() {
        let out = resolve_static_param_template(
            "orbit-{resource_id}-{timestamp}",
            "test-cluster",
            "20260309T143000",
        );
        assert_eq!(out, "orbit-test-cluster-20260309T143000");
    }

    #[test]
    fn test_resolve_static_param_template_keeps_plain_text() {
        let out = resolve_static_param_template("fixed-value", "x", "y");
        assert_eq!(out, "fixed-value");
    }

    #[test]
    fn describe_body_fills_the_resource_id() {
        let body = render_describe_body("{\"TableName\": \"{resource_id}\"}", "orders").unwrap();
        assert_eq!(body, "{\"TableName\": \"orders\"}");
    }

    /// WAFv2's GetWebACL wants Name, Id and Scope together, and the list call only
    /// hands back an ARN holding both.
    #[test]
    fn describe_body_splits_a_wafv2_arn_into_name_and_id() {
        let arn = "arn:aws:wafv2:eu-west-1:123456789012:regional/webacl/prod-edge/0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0";
        let body = render_describe_body(
            "{\"Name\": \"{arn_name}\", \"Id\": \"{arn_id}\", \"Scope\": \"REGIONAL\"}",
            arn,
        )
        .unwrap();
        assert_eq!(
            body,
            "{\"Name\": \"prod-edge\", \"Id\": \"0f1e2d3c-4b5a-6978-8796-a5b4c3d2e1f0\", \"Scope\": \"REGIONAL\"}"
        );
    }

    /// Substituting nothing would send a literal "{arn_name}" to AWS and report its
    /// confusing rejection as the resource's problem. Fail here instead, naming the
    /// id we could not split.
    #[test]
    fn describe_body_rejects_an_id_that_is_not_an_arn_with_a_name_and_id() {
        for id in [
            "prod-edge",
            "arn:aws:wafv2:eu-west-1:123456789012:regional/webacl",
            "",
        ] {
            let err = render_describe_body("{\"Name\": \"{arn_name}\"}", id)
                .expect_err("should refuse to guess a name");
            assert!(
                err.to_string().contains(id) || id.is_empty(),
                "error {:?} should name the id {:?}",
                err.to_string(),
                id
            );
        }
    }

    /// An enrich call's params are templated against the describe response that
    /// was *already* fetched, because the ids those calls need (an ARN for
    /// ListTagsForResource, the parent cluster id, the attached security groups)
    /// only appear in that response, not in the row's id.
    #[test]
    fn enrich_templates_read_the_row_id_and_the_primary_describe_response() {
        use serde_json::json;

        let primary = json!({
            "DBInstanceIdentifier": "prod-docdb-1",
            "DBInstanceArn": "arn:aws:rds:eu-west-1:123456789012:db:prod-docdb-1",
            "DBClusterIdentifier": "prod-docdb",
            "Endpoint": { "Port": 27017 },
        });
        let now = fixed_now();

        assert_eq!(
            resolve_enrich_template("{resource_id}", "prod-docdb-1", &primary, now).unwrap(),
            "prod-docdb-1"
        );
        assert_eq!(
            resolve_enrich_template("{/DBInstanceArn}", "prod-docdb-1", &primary, now).unwrap(),
            "arn:aws:rds:eu-west-1:123456789012:db:prod-docdb-1"
        );
        assert_eq!(
            resolve_enrich_template("{/Endpoint/Port}", "prod-docdb-1", &primary, now).unwrap(),
            "27017"
        );
        assert_eq!(
            resolve_enrich_template(
                "cluster={/DBClusterIdentifier};id={resource_id}",
                "prod-docdb-1",
                &primary,
                now
            )
            .unwrap(),
            "cluster=prod-docdb;id=prod-docdb-1"
        );
    }

    /// CloudWatch's GetMetricStatistics needs a window, and a window has to be
    /// relative to now or the panel would show a frozen point in the past.
    #[test]
    fn enrich_templates_render_a_relative_utc_clock_for_metric_windows() {
        use serde_json::json;

        let primary = json!({});
        let now = fixed_now();

        assert_eq!(
            resolve_enrich_template("{now}", "irrelevant", &primary, now).unwrap(),
            "2026-08-30T20:15:00Z"
        );
        assert_eq!(
            resolve_enrich_template("{now-15m}", "irrelevant", &primary, now).unwrap(),
            "2026-08-30T20:00:00Z"
        );
        assert_eq!(
            resolve_enrich_template("{now-2h}", "irrelevant", &primary, now).unwrap(),
            "2026-08-30T18:15:00Z"
        );
        assert_eq!(
            resolve_enrich_template("{now-1d}", "irrelevant", &primary, now).unwrap(),
            "2026-08-29T20:15:00Z"
        );
        assert_eq!(
            resolve_enrich_template("{now-30s}", "irrelevant", &primary, now).unwrap(),
            "2026-08-30T20:14:30Z"
        );
    }

    /// Sending a literal "{/DBInstanceArn}" to AWS earns a rejection that reads
    /// like the resource's fault. Refuse here, naming the path that was missing.
    #[test]
    fn enrich_templates_refuse_a_path_the_describe_response_does_not_carry() {
        use serde_json::json;

        let primary = json!({ "DBInstanceIdentifier": "prod-docdb-1" });

        for template in ["{/DBInstanceArn}", "{/Endpoint/Address}", "{unknown_token}"] {
            let err = resolve_enrich_template(template, "prod-docdb-1", &primary, fixed_now())
                .expect_err("should refuse to send an unresolved template");
            let message = err.to_string();
            assert!(
                template.contains(message.split_whitespace().last().unwrap_or("?"))
                    || message.contains(template.trim_matches(['{', '}'])),
                "error {:?} should name the token from {:?}",
                message,
                template
            );
        }
    }

    /// EC2's DescribeSecurityGroupRules only filters by group-id, and an instance
    /// can sit in several groups, so the values come out of the describe response
    /// as a list and have to expand into Filter.1.Value.1..N.
    #[test]
    fn enrich_filters_expand_into_the_ec2_filter_form() {
        use serde_json::json;

        let primary = json!({
            "VpcSecurityGroups": {
                "VpcSecurityGroupMembership": [
                    { "VpcSecurityGroupId": "sg-0f83a0604e72e34c1", "Status": "active" },
                    { "VpcSecurityGroupId": "sg-0fe51082fe28f1fe4", "Status": "active" },
                ]
            }
        });

        let enrich = EnrichCall {
            action: Some("DescribeSecurityGroupRules".to_string()),
            result_field: "SecurityGroupRules".to_string(),
            filters: vec![EnrichFilter {
                name: "group-id".to_string(),
                values_source: "/VpcSecurityGroups/VpcSecurityGroupMembership/VpcSecurityGroupId"
                    .to_string(),
            }],
            ..Default::default()
        };

        let params =
            build_enrich_query_params(&enrich, "prod-docdb-1", &primary, fixed_now()).unwrap();

        assert_eq!(
            params,
            vec![
                ("Filter.1.Name".to_string(), "group-id".to_string()),
                (
                    "Filter.1.Value.1".to_string(),
                    "sg-0f83a0604e72e34c1".to_string()
                ),
                (
                    "Filter.1.Value.2".to_string(),
                    "sg-0fe51082fe28f1fe4".to_string()
                ),
            ]
        );
    }

    /// XML→JSON collapses a one-element list to a bare object, so the same filter
    /// config must still produce one value instead of silently producing none.
    #[test]
    fn enrich_filters_survive_a_single_element_xml_list_collapsing() {
        use serde_json::json;

        let primary = json!({
            "VpcSecurityGroups": {
                "VpcSecurityGroupMembership": {
                    "VpcSecurityGroupId": "sg-0f83a0604e72e34c1", "Status": "active"
                }
            }
        });

        let enrich = EnrichCall {
            result_field: "SecurityGroupRules".to_string(),
            filters: vec![EnrichFilter {
                name: "group-id".to_string(),
                values_source: "/VpcSecurityGroups/VpcSecurityGroupMembership/VpcSecurityGroupId"
                    .to_string(),
            }],
            ..Default::default()
        };

        let params =
            build_enrich_query_params(&enrich, "prod-docdb-1", &primary, fixed_now()).unwrap();

        assert_eq!(
            params,
            vec![
                ("Filter.1.Name".to_string(), "group-id".to_string()),
                (
                    "Filter.1.Value.1".to_string(),
                    "sg-0f83a0604e72e34c1".to_string()
                ),
            ]
        );
    }

    /// A filter with nothing behind it would ask EC2 for every rule in the
    /// account, which reads as "these are this instance's rules". Refuse instead.
    #[test]
    fn enrich_filters_refuse_to_run_unfiltered_when_the_source_is_empty() {
        use serde_json::json;

        let enrich = EnrichCall {
            result_field: "SecurityGroupRules".to_string(),
            filters: vec![EnrichFilter {
                name: "group-id".to_string(),
                values_source: "/VpcSecurityGroups/VpcSecurityGroupMembership/VpcSecurityGroupId"
                    .to_string(),
            }],
            ..Default::default()
        };

        let err = build_enrich_query_params(&enrich, "prod-docdb-1", &json!({}), fixed_now())
            .expect_err("an unfilterable filter must not become a full-account query");
        assert!(
            err.to_string().contains("group-id"),
            "error {:?} should name the filter",
            err.to_string()
        );
    }

    /// CloudWatch's GetMetricStatistics body is nested JSON whose own braces
    /// would be read as template tokens if the body were a string, so it stays
    /// JSON and only its string leaves are templated.
    #[test]
    fn enrich_json_bodies_template_their_string_leaves_only() {
        use serde_json::json;

        let body = json!({
            "Namespace": "AWS/DocDB",
            "MetricName": "CPUUtilization",
            "Dimensions": [{ "Name": "DBInstanceIdentifier", "Value": "{resource_id}" }],
            "StartTime": "{now-15m}",
            "EndTime": "{now}",
            "Period": 300,
            "Statistics": ["Average"],
        });

        let resolved = resolve_enrich_body(&body, "prod-docdb-1", &json!({}), fixed_now()).unwrap();

        assert_eq!(
            resolved,
            json!({
                "Namespace": "AWS/DocDB",
                "MetricName": "CPUUtilization",
                "Dimensions": [{ "Name": "DBInstanceIdentifier", "Value": "prod-docdb-1" }],
                "StartTime": "2026-08-30T20:00:00Z",
                "EndTime": "2026-08-30T20:15:00Z",
                "Period": 300,
                "Statistics": ["Average"],
            })
        );
    }

    fn fixed_now() -> chrono::DateTime<chrono::Utc> {
        use chrono::TimeZone;
        chrono::Utc
            .with_ymd_and_hms(2026, 8, 30, 20, 15, 0)
            .single()
            .expect("valid test timestamp")
    }

    #[test]
    fn test_extract_param_variants() {
        use serde_json::json;

        // Test single string value
        let params_str = json!({ "bucket": "my-bucket" });
        assert_eq!(extract_param(&params_str, "bucket"), "my-bucket");

        // Test array with single string
        let params_single_arr = json!({ "bucket": ["only-bucket"] });
        assert_eq!(extract_param(&params_single_arr, "bucket"), "only-bucket");

        // Test array of strings (takes first)
        let params_arr = json!({ "bucket": ["first-bucket", "second-bucket"] });
        assert_eq!(extract_param(&params_arr, "bucket"), "first-bucket");

        // Test missing key
        assert_eq!(extract_param(&params_str, "nonexistent"), "");
    }
}
