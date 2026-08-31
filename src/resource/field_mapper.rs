//! Field mapping and transformation
//!
//! This module transforms raw AWS API responses into normalized JSON
//! objects based on field mapping configuration.

use super::path_extractor::{extract_by_path, value_to_string};
use super::protocol::FieldMapping;
use serde_json::{json, Map, Value};
use std::collections::HashMap;

/// Apply field mappings to transform a raw API response item into normalized output
///
/// # Arguments
/// * `item` - Raw item from API response
/// * `mappings` - Map of target field name -> source field mapping
///
/// # Returns
/// Normalized JSON object with mapped fields
pub fn apply_field_mappings(item: &Value, mappings: &HashMap<String, FieldMapping>) -> Value {
    let mut result = Map::new();

    for (target_field, mapping) in mappings {
        // If source is empty or "/", use the item itself (for scalar arrays like DynamoDB table names)
        let value = if mapping.source.is_empty() || mapping.source == "/" {
            item.clone()
        } else {
            extract_by_path(item, &mapping.source)
        };

        // Apply transformation if specified
        let value = if let Some(transform) = &mapping.transform {
            apply_transform(&value, transform)
        } else {
            value
        };

        // Apply default if value is null
        let value = if value.is_null() {
            mapping
                .default
                .as_ref()
                .map(|d| Value::String(d.clone()))
                .unwrap_or(Value::String("-".to_string()))
        } else {
            // Convert non-string values to strings for consistency
            match value {
                Value::String(_) => value,
                Value::Number(n) => Value::String(n.to_string()),
                Value::Bool(b) => Value::String(if b { "Yes" } else { "No" }.to_string()),
                Value::Array(_) | Value::Object(_) => value, // Keep complex types as-is
                Value::Null => Value::String("-".to_string()),
            }
        };

        result.insert(target_field.clone(), value);
    }

    Value::Object(result)
}

/// Apply a named transformation to a value
pub fn apply_transform(value: &Value, transform: &str) -> Value {
    match transform {
        "tags_to_map" => transform_tags_to_map(value),
        "format_bytes" => transform_format_bytes(value),
        "format_epoch_millis" => transform_format_epoch_millis(value),
        "format_epoch_seconds" => transform_format_epoch_seconds(value),
        "bool_to_yes_no" => transform_bool_to_yes_no(value),
        "array_to_csv" => transform_array_to_csv(value),
        "first_item" => transform_first_item(value),
        "private_zone_to_type" => transform_private_zone_to_type(value),
        "route53_record_value" => transform_route53_record_value(value),
        "route53_record_id" => transform_route53_record_id(value),
        "ecr_visibility" => transform_ecr_visibility(value),
        "taskdef_arn_name" => transform_taskdef_arn_name(value),
        "taskdef_arn_family" => transform_taskdef_arn_family(value),
        "taskdef_arn_revision" => transform_taskdef_arn_revision(value),
        "cloudwatch_latest" => transform_cloudwatch_latest(value),
        _ => value.clone(),
    }
}

/// The newest datapoint of a CloudWatch GetMetricStatistics response, formatted
/// for its unit. CloudWatch makes no promise about datapoint order, so this
/// picks by timestamp; an empty window stays null rather than becoming a zero
/// that reads as a real reading.
fn transform_cloudwatch_latest(value: &Value) -> Value {
    let datapoints: Vec<&Value> = match value {
        Value::Array(items) => items.iter().collect(),
        Value::Object(_) => vec![value],
        _ => return Value::Null,
    };

    let latest = datapoints.into_iter().max_by(|a, b| {
        datapoint_timestamp(a)
            .partial_cmp(&datapoint_timestamp(b))
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    let Some(latest) = latest else {
        return Value::Null;
    };

    // Whichever statistic was asked for is the only one present.
    let Some(reading) = ["Average", "Sum", "Maximum", "Minimum", "SampleCount"]
        .iter()
        .find_map(|stat| latest.get(stat).and_then(datapoint_number))
    else {
        return Value::Null;
    };

    let unit = latest.get("Unit").and_then(|u| u.as_str()).unwrap_or("");

    Value::String(match unit {
        "Percent" => format!("{:.2}%", reading),
        "Count" => format!("{:.0}", reading),
        "Bytes" => {
            return transform_format_bytes(&json!(reading.max(0.0) as u64));
        }
        "" => format!("{:.2}", reading),
        other => format!("{:.2} {}", reading, other),
    })
}

/// Query mode returns ISO8601 strings, the JSON protocol returns epoch seconds;
/// both have to order the same way.
fn datapoint_timestamp(datapoint: &Value) -> f64 {
    match datapoint.get("Timestamp") {
        Some(Value::Number(n)) => n.as_f64().unwrap_or(0.0),
        Some(Value::String(s)) => chrono::DateTime::parse_from_rfc3339(s)
            .map(|dt| dt.timestamp() as f64)
            .unwrap_or(0.0),
        _ => 0.0,
    }
}

fn datapoint_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    }
}

/// Transform Route53 record to unique ID (Name#Type)
/// This creates a unique identifier since multiple records can have the same name with different types
/// Input: {"Name": "example.com", "Type": "A"} -> "example.com#A"
fn transform_route53_record_id(value: &Value) -> Value {
    let name = value.get("Name").and_then(|v| v.as_str()).unwrap_or("-");
    let record_type = value.get("Type").and_then(|v| v.as_str()).unwrap_or("-");

    Value::String(format!("{}#{}", name, record_type))
}

/// Transform Route53 PrivateZone boolean to "Public"/"Private"
fn transform_private_zone_to_type(value: &Value) -> Value {
    match value {
        Value::Bool(b) => Value::String(if *b { "Private" } else { "Public" }.to_string()),
        Value::String(s) => {
            let is_private = s == "true" || s == "True" || s == "TRUE";
            Value::String(if is_private { "Private" } else { "Public" }.to_string())
        }
        _ => Value::String("Public".to_string()),
    }
}

/// Transform Route53 record to value string
/// Handles both ResourceRecords and AliasTarget
/// ResourceRecords: [{"Value": "192.0.2.1"}] -> "192.0.2.1"
/// AliasTarget: {"DNSName": "example.com"} -> "example.com"
fn transform_route53_record_value(value: &Value) -> Value {
    // Check for AliasTarget first
    if let Some(alias_target) = value.get("AliasTarget") {
        let dns_name = alias_target
            .get("DNSName")
            .and_then(|v| v.as_str())
            .unwrap_or("-");
        return Value::String(dns_name.to_string());
    }

    // Check for ResourceRecords
    if let Some(resource_records) = value.get("ResourceRecords") {
        if let Some(records) = resource_records.get("ResourceRecord") {
            let arr = match records {
                Value::Array(a) => a.clone(),
                obj @ Value::Object(_) => vec![obj.clone()],
                _ => return Value::String("-".to_string()),
            };

            let values: Vec<String> = arr
                .iter()
                .filter_map(|item| {
                    item.get("Value")
                        .or_else(|| item.get("value"))
                        .and_then(|v| v.as_str())
                        .map(|s| s.to_string())
                })
                .collect();

            if !values.is_empty() {
                return Value::String(values.join(", "));
            }
        }
    }

    Value::String("-".to_string())
}

/// Transform AWS tag array to a key-value map
///
/// Input: [{"key": "Name", "value": "MyInstance"}, {"Key": "Env", "Value": "prod"}]
/// Output: {"Name": "MyInstance", "Env": "prod"}
pub fn transform_tags_to_map(value: &Value) -> Value {
    let mut tags = Map::new();

    let items = match value {
        Value::Array(arr) => arr.clone(),
        Value::Object(_) => vec![value.clone()], // Single tag
        _ => return Value::Object(tags),
    };

    for tag in items {
        // AWS uses both "key"/"value" (EC2 XML) and "Key"/"Value" (other services)
        let key = tag
            .get("key")
            .or_else(|| tag.get("Key"))
            .and_then(|v| v.as_str());
        let val = tag
            .get("value")
            .or_else(|| tag.get("Value"))
            .and_then(|v| v.as_str());

        if let (Some(k), Some(v)) = (key, val) {
            tags.insert(k.to_string(), Value::String(v.to_string()));
        }
    }

    Value::Object(tags)
}

/// Format bytes into human-readable format
pub fn transform_format_bytes(value: &Value) -> Value {
    let bytes = match value {
        Value::Number(n) => n.as_u64().unwrap_or(0),
        Value::String(s) => s.parse::<u64>().unwrap_or(0),
        _ => return Value::String("-".to_string()),
    };

    const KB: u64 = 1024;
    const MB: u64 = KB * 1024;
    const GB: u64 = MB * 1024;
    const TB: u64 = GB * 1024;

    let formatted = if bytes >= TB {
        format!("{:.1} TB", bytes as f64 / TB as f64)
    } else if bytes >= GB {
        format!("{:.1} GB", bytes as f64 / GB as f64)
    } else if bytes >= MB {
        format!("{:.1} MB", bytes as f64 / MB as f64)
    } else if bytes >= KB {
        format!("{:.1} KB", bytes as f64 / KB as f64)
    } else {
        format!("{} B", bytes)
    };

    Value::String(formatted)
}

/// Format epoch milliseconds to human-readable date string
pub fn transform_format_epoch_millis(value: &Value) -> Value {
    let millis = match value {
        Value::Number(n) => n.as_i64().unwrap_or(0),
        Value::String(s) => s.parse::<i64>().unwrap_or(0),
        _ => return Value::String("-".to_string()),
    };

    if millis <= 0 {
        return Value::String("-".to_string());
    }

    use chrono::{TimeZone, Utc};

    let formatted = Utc
        .timestamp_millis_opt(millis)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string());

    Value::String(formatted)
}

/// Format epoch seconds to human-readable date string
pub fn transform_format_epoch_seconds(value: &Value) -> Value {
    let secs = match value {
        Value::Number(n) => n.as_f64().unwrap_or(0.0) as i64,
        Value::String(s) => s.parse::<i64>().unwrap_or(0),
        _ => return Value::String("-".to_string()),
    };

    if secs <= 0 {
        return Value::String("-".to_string());
    }

    use chrono::{TimeZone, Utc};

    let formatted = Utc
        .timestamp_opt(secs, 0)
        .single()
        .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
        .unwrap_or_else(|| "-".to_string());

    Value::String(formatted)
}

/// Transform boolean to Yes/No string
pub fn transform_bool_to_yes_no(value: &Value) -> Value {
    match value {
        Value::Bool(b) => Value::String(if *b { "Yes" } else { "No" }.to_string()),
        Value::String(s) => {
            let yes = s == "true" || s == "True" || s == "TRUE" || s == "yes" || s == "Yes";
            Value::String(if yes { "Yes" } else { "No" }.to_string())
        }
        _ => Value::String("-".to_string()),
    }
}

/// Detect ECR repository visibility from its URI.
/// Private repos use .dkr.ecr.<region>.amazonaws.com;
/// public repos use public.ecr.aws.
pub fn transform_ecr_visibility(value: &Value) -> Value {
    let uri = value.as_str().unwrap_or("");
    let visibility = if uri.contains("public.ecr.aws") {
        "Public"
    } else {
        "Private"
    };
    Value::String(visibility.to_string())
}

/// Extract the family:revision suffix from a task-definition ARN.
/// "arn:aws:ecs:eu-west-1:123456789012:task-definition/TPayments:1" -> "TPayments:1"
pub fn transform_taskdef_arn_name(value: &Value) -> Value {
    match value {
        Value::String(s) => {
            let suffix = s.split("task-definition/").nth(1).unwrap_or("");
            Value::String(if suffix.is_empty() {
                s.clone()
            } else {
                suffix.to_string()
            })
        }
        _ => value.clone(),
    }
}

/// Extract just the family part of a task-definition ARN.
/// ".../TPayments:1" -> "TPayments"
pub fn transform_taskdef_arn_family(value: &Value) -> Value {
    match transform_taskdef_arn_name(value) {
        Value::String(s) => Value::String(s.split(':').next().unwrap_or(&s).to_string()),
        other => other,
    }
}

/// Extract just the revision part of a task-definition ARN.
/// ".../TPayments:1" -> "1"
pub fn transform_taskdef_arn_revision(value: &Value) -> Value {
    match transform_taskdef_arn_name(value) {
        Value::String(s) => Value::String(
            s.rsplit(':')
                .next()
                .filter(|r| *r != s)
                .unwrap_or("-")
                .to_string(),
        ),
        other => other,
    }
}

/// Transform array to comma-separated values
pub fn transform_array_to_csv(value: &Value) -> Value {
    match value {
        Value::Array(arr) => {
            let csv: Vec<String> = arr.iter().map(|v| value_to_string(v, "")).collect();
            Value::String(csv.join(", "))
        }
        _ => value.clone(),
    }
}

/// Extract first item from array
pub fn transform_first_item(value: &Value) -> Value {
    match value {
        Value::Array(arr) => arr.first().cloned().unwrap_or(Value::Null),
        _ => value.clone(),
    }
}

/// Build a normalized response with items under the specified key
pub fn build_response(items: Vec<Value>, response_key: &str, next_token: Option<String>) -> Value {
    let mut response = json!({
        response_key: items
    });

    if let Some(token) = next_token {
        response["_next_token"] = json!(token);
    }

    response
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CloudWatch does not promise datapoint order, and the panel shows one
    /// number, so the transform picks by timestamp rather than by position.
    #[test]
    fn cloudwatch_latest_picks_the_newest_datapoint_not_the_first() {
        let datapoints = json!([
            { "Timestamp": 1_787_000_000.0, "Average": 4.1, "Unit": "Percent" },
            { "Timestamp": 1_787_000_600.0, "Average": 6.34, "Unit": "Percent" },
            { "Timestamp": 1_787_000_300.0, "Average": 5.2, "Unit": "Percent" },
        ]);

        assert_eq!(
            apply_transform(&datapoints, "cloudwatch_latest"),
            json!("6.34%")
        );
    }

    /// The query-mode wire hands timestamps back as ISO8601 strings, so the
    /// comparison cannot assume numbers.
    #[test]
    fn cloudwatch_latest_orders_iso8601_timestamps_too() {
        let datapoints = json!([
            { "Timestamp": "2026-08-30T20:05:00Z", "Sum": 51.0, "Unit": "Count" },
            { "Timestamp": "2026-08-30T19:55:00Z", "Sum": 12.0, "Unit": "Count" },
        ]);

        assert_eq!(
            apply_transform(&datapoints, "cloudwatch_latest"),
            json!("51")
        );
    }

    /// A metric with no datapoints in the window (a stopped instance) must not
    /// invent a zero, which reads as a real reading.
    #[test]
    fn cloudwatch_latest_yields_nothing_for_an_empty_window() {
        assert!(apply_transform(&json!([]), "cloudwatch_latest").is_null());
        assert!(apply_transform(&Value::Null, "cloudwatch_latest").is_null());
    }

    /// Byte metrics (FreeableMemory) are unreadable in raw bytes.
    #[test]
    fn cloudwatch_latest_formats_byte_metrics_as_bytes() {
        let datapoints = json!([
            { "Timestamp": 1_787_000_000.0, "Average": 1_073_741_824.0, "Unit": "Bytes" },
        ]);

        assert_eq!(
            apply_transform(&datapoints, "cloudwatch_latest"),
            json!("1.0 GB")
        );
    }

    #[test]
    fn test_apply_field_mappings() {
        let item = json!({
            "instanceId": "i-123",
            "instanceState": {
                "name": "running"
            }
        });

        let mut mappings = HashMap::new();
        mappings.insert(
            "InstanceId".to_string(),
            FieldMapping {
                source: "/instanceId".to_string(),
                default: None,
                transform: None,
                array_item_path: None,
            },
        );
        mappings.insert(
            "State".to_string(),
            FieldMapping {
                source: "/instanceState/name".to_string(),
                default: None,
                transform: None,
                array_item_path: None,
            },
        );

        let result = apply_field_mappings(&item, &mappings);
        assert_eq!(result["InstanceId"], "i-123");
        assert_eq!(result["State"], "running");
    }

    #[test]
    fn test_apply_field_mappings_with_default() {
        let item = json!({
            "instanceId": "i-123"
        });

        let mut mappings = HashMap::new();
        mappings.insert(
            "PublicIp".to_string(),
            FieldMapping {
                source: "/publicIp".to_string(),
                default: Some("N/A".to_string()),
                transform: None,
                array_item_path: None,
            },
        );

        let result = apply_field_mappings(&item, &mappings);
        assert_eq!(result["PublicIp"], "N/A");
    }

    #[test]
    fn test_transform_tags_to_map() {
        let tags = json!([
            {"key": "Name", "value": "MyInstance"},
            {"key": "Env", "value": "prod"}
        ]);

        let result = transform_tags_to_map(&tags);
        assert_eq!(result["Name"], "MyInstance");
        assert_eq!(result["Env"], "prod");
    }

    #[test]
    fn test_transform_tags_capital_case() {
        let tags = json!([
            {"Key": "Name", "Value": "MyInstance"}
        ]);

        let result = transform_tags_to_map(&tags);
        assert_eq!(result["Name"], "MyInstance");
    }

    #[test]
    fn test_transform_format_bytes() {
        assert_eq!(transform_format_bytes(&json!(0)), json!("0 B"));
        assert_eq!(transform_format_bytes(&json!(1024)), json!("1.0 KB"));
        assert_eq!(transform_format_bytes(&json!(1048576)), json!("1.0 MB"));
        assert_eq!(transform_format_bytes(&json!(1073741824)), json!("1.0 GB"));
    }

    #[test]
    fn test_transform_bool_to_yes_no() {
        assert_eq!(transform_bool_to_yes_no(&json!(true)), json!("Yes"));
        assert_eq!(transform_bool_to_yes_no(&json!(false)), json!("No"));
        assert_eq!(transform_bool_to_yes_no(&json!("true")), json!("Yes"));
        assert_eq!(transform_bool_to_yes_no(&json!("false")), json!("No"));
    }

    #[test]
    fn test_transform_format_epoch_seconds() {
        assert_eq!(
            transform_format_epoch_seconds(&json!(1687351280)),
            json!("2023-06-21 12:41:20")
        );
        assert_eq!(transform_format_epoch_seconds(&json!(0)), json!("-"));
    }

    #[test]
    fn test_build_response() {
        let items = vec![json!({"id": "1"}), json!({"id": "2"})];

        let response = build_response(items, "instances", Some("token123".to_string()));
        assert_eq!(response["instances"].as_array().unwrap().len(), 2);
        assert_eq!(response["_next_token"], "token123");
    }

    #[test]
    fn test_transform_route53_record_value_with_single_resource_record() {
        let record = json!({
            "ResourceRecords": {
                "ResourceRecord": {
                    "Value": "192.0.2.1"
                }
            }
        });

        let result = transform_route53_record_value(&record);
        assert_eq!(result, json!("192.0.2.1"));
    }

    #[test]
    fn test_transform_route53_record_value_with_multiple_resource_records() {
        let record = json!({
            "ResourceRecords": {
                "ResourceRecord": [
                    {"Value": "192.0.2.1"},
                    {"Value": "192.0.2.2"},
                    {"Value": "192.0.2.3"}
                ]
            }
        });

        let result = transform_route53_record_value(&record);
        assert_eq!(result, json!("192.0.2.1, 192.0.2.2, 192.0.2.3"));
    }

    #[test]
    fn test_transform_route53_record_value_with_alias_target() {
        let record = json!({
            "AliasTarget": {
                "DNSName": "elb-123.us-east-1.elb.amazonaws.com",
                "HostedZoneId": "Z35SXDOTRQ7X7K",
                "EvaluateTargetHealth": "false"
            }
        });

        let result = transform_route53_record_value(&record);
        assert_eq!(result, json!("elb-123.us-east-1.elb.amazonaws.com"));
    }

    #[test]
    fn test_transform_route53_record_value_with_empty_records() {
        let record = json!({
            "ResourceRecords": {
                "ResourceRecord": []
            }
        });

        let result = transform_route53_record_value(&record);
        assert_eq!(result, json!("-"));
    }

    #[test]
    fn test_transform_route53_record_value_with_no_value() {
        let record = json!({});

        let result = transform_route53_record_value(&record);
        assert_eq!(result, json!("-"));
    }

    #[test]
    fn test_transform_route53_record_id() {
        let record = json!({
            "Name": "example.com.",
            "Type": "A"
        });

        let result = transform_route53_record_id(&record);
        assert_eq!(result, json!("example.com.#A"));
    }

    #[test]
    fn test_transform_route53_record_id_with_different_types() {
        let a_record = json!({"Name": "example.com.", "Type": "A"});
        let aaaa_record = json!({"Name": "example.com.", "Type": "AAAA"});
        let mx_record = json!({"Name": "example.com.", "Type": "MX"});

        assert_eq!(
            transform_route53_record_id(&a_record),
            json!("example.com.#A")
        );
        assert_eq!(
            transform_route53_record_id(&aaaa_record),
            json!("example.com.#AAAA")
        );
        assert_eq!(
            transform_route53_record_id(&mx_record),
            json!("example.com.#MX")
        );
    }

    #[test]
    fn test_transform_route53_record_id_with_missing_fields() {
        let record = json!({});

        let result = transform_route53_record_id(&record);
        assert_eq!(result, json!("-#-"));
    }

    #[test]
    fn test_transform_taskdef_arn_parts() {
        let arn = json!("arn:aws:ecs:eu-west-1:123456789012:task-definition/TPayments:3");
        assert_eq!(transform_taskdef_arn_name(&arn), json!("TPayments:3"));
        assert_eq!(transform_taskdef_arn_family(&arn), json!("TPayments"));
        assert_eq!(transform_taskdef_arn_revision(&arn), json!("3"));
    }

    #[test]
    fn test_transform_taskdef_arn_unparseable_passes_through() {
        let not_an_arn = json!("just-a-string");
        assert_eq!(
            transform_taskdef_arn_name(&not_an_arn),
            json!("just-a-string")
        );
        assert_eq!(
            transform_taskdef_arn_family(&not_an_arn),
            json!("just-a-string")
        );
        assert_eq!(transform_taskdef_arn_revision(&not_an_arn), json!("-"));
    }
}
