//! Protocol handlers for different AWS API protocols
//!
//! Each handler implements the logic for building requests and parsing responses
//! for a specific AWS protocol type.

mod json;
mod query;
mod rest_json;
mod rest_xml;

pub use self::json::JsonProtocolHandler;
pub use self::query::QueryProtocolHandler;
pub use self::rest_json::RestJsonProtocolHandler;
pub use self::rest_xml::RestXmlProtocolHandler;

use super::field_mapper::apply_field_mappings;
use super::protocol::{ApiConfig, ApiProtocol, FieldMapping, ParsedResponse};
use crate::aws::client::AwsClients;
use anyhow::Result;
use serde_json::Value;
use std::collections::HashMap;

/// Trait for protocol-specific request/response handling
///
/// Note: We use a sync trait with separate async methods to maintain
/// dyn-compatibility. The actual async work is done by helper functions.
pub trait ProtocolHandler: Send + Sync {
    /// Parse raw response into items using the response_root path
    fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)>;
}

/// Unified protocol handler that dispatches to specific implementations
pub struct UnifiedProtocolHandler {
    protocol: ApiProtocol,
}

impl UnifiedProtocolHandler {
    pub fn new(protocol: ApiProtocol) -> Self {
        Self { protocol }
    }

    /// Execute the API request
    pub async fn execute(
        &self,
        clients: &AwsClients,
        service: &str,
        config: &ApiConfig,
        params: &Value,
    ) -> Result<String> {
        match self.protocol {
            ApiProtocol::Query => {
                QueryProtocolHandler
                    .execute_impl(clients, service, config, params)
                    .await
            }
            ApiProtocol::Json => {
                JsonProtocolHandler
                    .execute_impl(clients, service, config, params)
                    .await
            }
            ApiProtocol::RestJson => {
                RestJsonProtocolHandler
                    .execute_impl(clients, service, config, params)
                    .await
            }
            ApiProtocol::RestXml => {
                RestXmlProtocolHandler
                    .execute_impl(clients, service, config, params)
                    .await
            }
        }
    }

    /// Parse raw response into items
    pub fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)> {
        match self.protocol {
            ApiProtocol::Query => QueryProtocolHandler.parse_items(response, config),
            ApiProtocol::Json => JsonProtocolHandler.parse_items(response, config),
            ApiProtocol::RestJson => RestJsonProtocolHandler.parse_items(response, config),
            ApiProtocol::RestXml => RestXmlProtocolHandler.parse_items(response, config),
        }
    }

    /// Full pipeline: execute request, parse items, apply field mappings
    pub async fn invoke(
        &self,
        clients: &AwsClients,
        service: &str,
        config: &ApiConfig,
        params: &Value,
        field_mappings: &HashMap<String, FieldMapping>,
    ) -> Result<ParsedResponse> {
        // Execute the API call
        let response = self.execute(clients, service, config, params).await?;

        // Parse items from response
        let (items, next_token) = self.parse_items(&response, config)?;

        // Per-item enrichment: call describe for each sparse list item BEFORE
        // field mappings so the enriched full objects get properly mapped.
        let mut enriched = items;
        if let Some(ref enrich) = config.list_enrich {
            for item in &mut enriched {
                let id = item
                    .as_str()
                    .or_else(|| {
                        item.get("name")
                            .or_else(|| item.get("id"))
                            .and_then(|v| v.as_str())
                    })
                    .map(|s| s.to_string());

                if let Some(id) = id {
                    let mut enrich_path = enrich.path.clone();
                    enrich_path = enrich_path.replace("{resource_id}", &id);
                    if let Value::Object(map) = params {
                        for (key, value) in map {
                            if let Some(s) = value.as_str() {
                                enrich_path = enrich_path.replace(&format!("{{{}}}", key), s);
                            }
                        }
                    }

                    if let Ok(response) = clients
                        .http
                        .rest_json_request(service, &enrich.method, &enrich_path, None)
                        .await
                    {
                        if let Ok(json) = serde_json::from_str::<Value>(&response) {
                            let full = if let Some(ref rp) = enrich.response_path {
                                json.pointer(rp).cloned().unwrap_or(json)
                            } else {
                                json
                            };
                            *item = full;
                        }
                    }
                }
            }
        }

        // Apply field mappings if provided (after enrichment so rich items map)
        let mapped_items = if field_mappings.is_empty() {
            enriched
        } else {
            enriched
                .iter()
                .map(|item| apply_field_mappings(item, field_mappings))
                .collect()
        };

        Ok(ParsedResponse {
            items: mapped_items,
            next_token,
        })
    }
}

/// Get the appropriate protocol handler for the given protocol type
pub fn get_protocol_handler(protocol: ApiProtocol) -> UnifiedProtocolHandler {
    UnifiedProtocolHandler::new(protocol)
}
