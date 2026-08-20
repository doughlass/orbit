//! REST-XML Protocol Handler
//!
//! Handles AWS REST-XML protocol (used by S3, Route53, CloudFront, etc.)
//! - Request: HTTP method with optional XML body, path parameters
//! - Response: XML

use super::ProtocolHandler;
use crate::aws::client::AwsClients;
use crate::aws::http::xml_to_json;
use crate::resource::path_extractor::{extract_by_path, extract_list};
use crate::resource::protocol::ApiConfig;
use anyhow::Result;
use serde_json::Value;

pub struct RestXmlProtocolHandler;

impl RestXmlProtocolHandler {
    /// Execute the API request (async implementation)
    pub async fn execute_impl(
        &self,
        clients: &AwsClients,
        service: &str,
        config: &ApiConfig,
        params: &Value,
    ) -> Result<String> {
        let method = config.method.as_deref().unwrap_or("GET");
        let path_template = config.path.as_deref().unwrap_or("/");

        // Replace path parameters
        let mut path = path_template.to_string();
        if let Value::Object(map) = params {
            for (key, value) in map {
                if let Some(s) = value.as_str() {
                    path = path.replace(&format!("{{{}}}", key), s);
                }
            }
        }

        // Add query parameters if needed
        let mut query_parts: Vec<String> = Vec::new();

        // Add static params as query params for GET
        if method == "GET" {
            for (key, value) in &config.static_params {
                if let Some(s) = value.as_str() {
                    query_parts.push(format!("{}={}", key, urlencoding::encode(s)));
                }
            }
        }

        // Add pagination token
        if let Some(token) = params.get("_page_token").and_then(|v| v.as_str()) {
            if let Some(pagination) = &config.pagination {
                if let Some(input_token) = &pagination.input_token {
                    query_parts.push(format!("{}={}", input_token, urlencoding::encode(token)));
                }
            }
        }

        if !query_parts.is_empty() {
            if path.contains('?') {
                path = format!("{}&{}", path, query_parts.join("&"));
            } else {
                path = format!("{}?{}", path, query_parts.join("&"));
            }
        }

        // For S3 and other REST-XML services
        clients
            .http
            .rest_xml_request(service, method, &path, None)
            .await
    }
}

impl ProtocolHandler for RestXmlProtocolHandler {
    fn parse_items(
        &self,
        response: &str,
        config: &ApiConfig,
    ) -> Result<(Vec<Value>, Option<String>)> {
        // Convert XML to JSON
        let json = xml_to_json(response)?;

        // Extract items using response_root path
        let items = if let Some(root) = &config.response_root {
            extract_list(&json, root)
        } else {
            vec![]
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
    fn test_parse_s3_list_buckets() {
        // This would be the JSON after xml_to_json conversion
        let json_str = r#"{
            "ListAllMyBucketsResult": {
                "Buckets": {
                    "Bucket": [
                        {"Name": "bucket1", "CreationDate": "2024-01-01"},
                        {"Name": "bucket2", "CreationDate": "2024-01-02"}
                    ]
                }
            }
        }"#;

        let config = ApiConfig {
            response_root: Some("/ListAllMyBucketsResult/Buckets/Bucket".to_string()),
            ..Default::default()
        };

        // Parse as if it were already converted from XML
        let json: Value = serde_json::from_str(json_str).unwrap();
        let items = extract_list(&json, config.response_root.as_ref().unwrap());

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["Name"], "bucket1");
    }

    /// CloudFront ListDistributions carries Aliases and Origins in the summary,
    /// so the list view can show them without an extra GetDistribution call.
    /// Driven by the real registry config so a broken json_path fails here.
    #[test]
    fn test_cloudfront_distribution_aliases_and_origins() {
        use crate::resource::field_mapper::apply_field_mappings;
        use crate::resource::registry::get_resource;

        let xml = r#"<?xml version="1.0"?>
<DistributionList>
  <Items>
    <DistributionSummary>
      <Id>E1MULTI</Id>
      <Status>Deployed</Status>
      <DomainName>d1multi.cloudfront.net</DomainName>
      <Aliases>
        <Quantity>2</Quantity>
        <Items>
          <CNAME>cdn.example.com</CNAME>
          <CNAME>www.example.com</CNAME>
        </Items>
      </Aliases>
      <Origins>
        <Quantity>2</Quantity>
        <Items>
          <Origin>
            <Id>s3-origin</Id>
            <DomainName>assets.s3.eu-west-1.amazonaws.com</DomainName>
          </Origin>
          <Origin>
            <Id>alb-origin</Id>
            <DomainName>alb-123.eu-west-1.elb.amazonaws.com</DomainName>
          </Origin>
        </Items>
      </Origins>
      <Enabled>true</Enabled>
    </DistributionSummary>
    <DistributionSummary>
      <Id>E1SINGLE</Id>
      <Status>InProgress</Status>
      <DomainName>d1single.cloudfront.net</DomainName>
      <Aliases>
        <Quantity>1</Quantity>
        <Items>
          <CNAME>single.example.com</CNAME>
        </Items>
      </Aliases>
      <Origins>
        <Quantity>1</Quantity>
        <Items>
          <Origin>
            <Id>only-origin</Id>
            <DomainName>only.example.com</DomainName>
          </Origin>
        </Items>
      </Origins>
      <Enabled>false</Enabled>
    </DistributionSummary>
    <DistributionSummary>
      <Id>E1NONE</Id>
      <Status>Deployed</Status>
      <DomainName>d1none.cloudfront.net</DomainName>
      <Aliases>
        <Quantity>0</Quantity>
      </Aliases>
      <Origins>
        <Quantity>1</Quantity>
        <Items>
          <Origin>
            <Id>only-origin</Id>
            <DomainName>only.example.com</DomainName>
          </Origin>
        </Items>
      </Origins>
      <Enabled>true</Enabled>
    </DistributionSummary>
  </Items>
</DistributionList>"#;

        let resource = get_resource("cloudfront-distributions").unwrap();
        let config = resource.api_config.as_ref().unwrap();

        let handler = RestXmlProtocolHandler;
        let (items, _) = handler.parse_items(xml, config).unwrap();
        assert_eq!(items.len(), 3);

        let mapped: Vec<Value> = items
            .iter()
            .map(|item| apply_field_mappings(item, &resource.field_mappings))
            .collect();

        // Multiple aliases/origins collapse to a comma-separated string
        assert_eq!(mapped[0]["Id"], "E1MULTI");
        assert_eq!(mapped[0]["Aliases"], "cdn.example.com, www.example.com");
        assert_eq!(
            mapped[0]["Origins"],
            "assets.s3.eu-west-1.amazonaws.com, alb-123.eu-west-1.elb.amazonaws.com"
        );
        assert_eq!(mapped[0]["Enabled"], "Yes");

        // XML-to-JSON gives a bare string when there is only one child
        assert_eq!(mapped[1]["Aliases"], "single.example.com");
        assert_eq!(mapped[1]["Origins"], "only.example.com");
        assert_eq!(mapped[1]["Enabled"], "No");

        // Quantity 0 means no Items element at all
        assert_eq!(mapped[2]["Aliases"], "-");
        assert_eq!(mapped[2]["Origins"], "only.example.com");
    }

    #[test]
    fn test_parse_route53_hosted_zones() {
        let json_str = r#"{
            "ListHostedZonesResponse": {
                "HostedZones": {
                    "HostedZone": [
                        {"Id": "/hostedzone/Z123", "Name": "example.com."},
                        {"Id": "/hostedzone/Z456", "Name": "test.com."}
                    ]
                }
            }
        }"#;

        let config = ApiConfig {
            response_root: Some("/ListHostedZonesResponse/HostedZones/HostedZone".to_string()),
            ..Default::default()
        };

        let json: Value = serde_json::from_str(json_str).unwrap();
        let items = extract_list(&json, config.response_root.as_ref().unwrap());

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["Name"], "example.com.");
    }
}
