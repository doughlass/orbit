//! Resource Registry - Load resource definitions from JSON
//!
//! This module loads all AWS resource definitions from embedded JSON files
//! and provides lookup functions for the rest of the application.

use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::sync::OnceLock;

use super::protocol::{ActionConfig, ApiConfig, DescribeConfig, FieldMapping};

/// Embedded resource JSON files (compiled into the binary)
const RESOURCE_FILES: &[&str] = &[
    include_str!("../resources/acm.json"),
    include_str!("../resources/apigateway.json"),
    include_str!("../resources/apigatewayv2.json"),
    include_str!("../resources/athena.json"),
    include_str!("../resources/autoscaling.json"),
    include_str!("../resources/cloudformation.json"),
    include_str!("../resources/cloudfront.json"),
    include_str!("../resources/cloudtrail.json"),
    include_str!("../resources/cloudwatch.json"),
    include_str!("../resources/codebuild.json"),
    include_str!("../resources/codepipeline.json"),
    include_str!("../resources/cognito.json"),
    include_str!("../resources/common.json"),
    include_str!("../resources/dynamodb.json"),
    include_str!("../resources/ec2.json"),
    include_str!("../resources/ecr.json"),
    include_str!("../resources/ecs.json"),
    include_str!("../resources/ecs-task-definitions.json"),
    include_str!("../resources/efs.json"),
    include_str!("../resources/eks.json"),
    include_str!("../resources/elasticache.json"),
    include_str!("../resources/elbv2.json"),
    include_str!("../resources/eventbridge.json"),
    include_str!("../resources/fsx.json"),
    include_str!("../resources/guardduty.json"),
    include_str!("../resources/iam.json"),
    include_str!("../resources/inspector2.json"),
    include_str!("../resources/kms.json"),
    include_str!("../resources/lambda.json"),
    include_str!("../resources/macie2.json"),
    include_str!("../resources/msk.json"),
    include_str!("../resources/rds.json"),
    include_str!("../resources/redshift.json"),
    include_str!("../resources/route53.json"),
    include_str!("../resources/s3.json"),
    include_str!("../resources/scheduler.json"),
    include_str!("../resources/secretsmanager.json"),
    include_str!("../resources/securityhub.json"),
    include_str!("../resources/sns.json"),
    include_str!("../resources/sqs.json"),
    include_str!("../resources/ssm.json"),
    include_str!("../resources/stepfunctions.json"),
    include_str!("../resources/sts.json"),
    include_str!("../resources/vpc.json"),
    include_str!("../resources/vpc-networking.json"),
    include_str!("../resources/wafv2.json"),
];

/// Color definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ColorDef {
    pub value: String,
    pub color: [u8; 3],
}

/// Column definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ColumnDef {
    pub header: String,
    pub json_path: String,
    pub width: u16,
    #[serde(default)]
    pub color_map: Option<String>,
    /// Default visibility before the user saves column preferences. Extended
    /// attribute columns are defined in JSON but start hidden; the picker
    /// (p key) toggles them, and saved preferences override this entirely.
    #[serde(default = "default_true")]
    pub visible: bool,
}

fn default_true() -> bool {
    true
}

/// Sub-resource definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct SubResourceDef {
    pub resource_key: String,
    pub display_name: String,
    pub shortcut: String,
    pub parent_id_field: String,
    pub filter_param: String,
    /// Filter type: "scalar" (default) for single-value params (IAM, ELBv2, RDS),
    /// "ec2_filter" for EC2-style Filter.N.Name/Value params (VPC subnets, security groups)
    #[serde(default = "default_filter_type")]
    pub filter_type: String,
}

fn default_filter_type() -> String {
    "scalar".to_string()
}

/// Confirmation config for actions
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConfirmConfig {
    /// Message to show in confirmation dialog
    #[serde(default)]
    pub message: Option<String>,
    /// If true, default selection is Yes; if false, default is No
    #[serde(default)]
    pub default_yes: bool,
    /// If true, action is destructive (shown in red)
    #[serde(default)]
    pub destructive: bool,
}

/// Filters configuration for resources that support AWS API filtering
#[derive(Debug, Clone, Deserialize, Default)]
pub struct FiltersConfig {
    /// Whether this resource supports filtering via AWS API
    #[serde(default)]
    pub enabled: bool,
    /// Hint text showing available filter keys (e.g., "owner, architecture, state")
    #[serde(default)]
    pub hint: Option<String>,
}

/// Action definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ActionDef {
    /// Key identifier for the action (kept for JSON compatibility)
    #[allow(dead_code)]
    pub key: String,
    pub display_name: String,
    #[serde(default)]
    pub shortcut: Option<String>,
    pub sdk_method: String,
    /// Parameter name for the resource ID (kept for potential future use)
    #[serde(default)]
    #[allow(dead_code)]
    pub id_param: Option<String>,
    /// Legacy field - use `confirm` instead
    #[serde(default)]
    pub needs_confirm: bool,
    /// Confirmation configuration
    #[serde(default)]
    pub confirm: Option<ConfirmConfig>,
    /// If true, display the action result in the JSON viewer instead of just executing
    #[serde(default)]
    pub show_result: bool,
}

impl ActionDef {
    /// Check if this action requires confirmation
    pub fn requires_confirm(&self) -> bool {
        self.confirm.is_some() || self.needs_confirm
    }

    /// Get the confirmation config (with defaults)
    pub fn get_confirm_config(&self) -> Option<ConfirmConfig> {
        if let Some(ref config) = self.confirm {
            Some(config.clone())
        } else if self.needs_confirm {
            Some(ConfirmConfig {
                message: Some(self.display_name.clone()),
                default_yes: false,
                destructive: false,
            })
        } else {
            None
        }
    }
}

/// Resource definition from JSON
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceDef {
    pub display_name: String,
    pub service: String,
    /// Legacy: SDK method name (used by old sdk_dispatch.rs)
    /// New resources should use api_config instead
    pub sdk_method: String,
    #[serde(default)]
    pub sdk_method_params: Value,
    pub response_path: String,
    pub id_field: String,
    pub name_field: String,
    #[serde(default)]
    pub is_global: bool,
    pub columns: Vec<ColumnDef>,
    #[serde(default)]
    pub sub_resources: Vec<SubResourceDef>,
    #[serde(default)]
    pub actions: Vec<ActionDef>,
    /// SDK method to call when fetching details for a single resource
    #[serde(default)]
    pub detail_sdk_method: Option<String>,
    /// Parameters for detail_sdk_method (maps param name -> field from resource)
    #[serde(default)]
    pub detail_sdk_method_params: Value,

    // === NEW DATA-DRIVEN FIELDS ===
    /// API configuration for data-driven dispatch (list operations)
    /// If present, this takes precedence over sdk_method for fetching
    #[serde(default)]
    pub api_config: Option<ApiConfig>,

    /// Field mappings from raw API response to normalized output
    /// If present, these are used to transform API responses
    #[serde(default)]
    pub field_mappings: HashMap<String, FieldMapping>,

    /// Data-driven action configurations
    /// Maps action_id (e.g., "start_instance") to its API config
    #[serde(default)]
    pub action_configs: HashMap<String, ActionConfig>,

    /// Data-driven describe configuration
    /// For fetching single resource details
    #[serde(default)]
    pub describe_config: Option<DescribeConfig>,

    /// Filters configuration
    /// If present and enabled, the resource supports AWS API filtering (Filters: key=value)
    #[serde(default)]
    pub filters_config: Option<FiltersConfig>,

    /// If true, this resource requires a parent context and cannot be accessed directly
    /// Used for sub-resources like Log Streams that need a Log Group
    #[serde(default)]
    pub requires_parent: bool,

    /// If true, preserve the order returned by the API instead of sorting alphabetically
    #[serde(default)]
    pub preserve_order: bool,

    /// Sub-resource that Enter drills into, instead of opening the details panel.
    /// Must also appear in `sub_resources`. Rows with no children (S3 files) still
    /// fall back to details, so Enter is never a dead key.
    #[serde(default)]
    pub enter_sub_resource: Option<String>,
}

impl ResourceDef {
    /// Check if this resource has API config for list operations
    pub fn has_api_config(&self) -> bool {
        self.api_config.is_some() && !self.field_mappings.is_empty()
    }

    /// Check if this resource supports filtering via AWS API
    pub fn supports_filters(&self) -> bool {
        self.filters_config
            .as_ref()
            .map(|fc| fc.enabled)
            .unwrap_or(false)
    }

    /// Get the filter hint for this resource
    pub fn filters_hint(&self) -> Option<&str> {
        self.filters_config
            .as_ref()
            .and_then(|fc| fc.hint.as_deref())
    }
}

/// Root structure of resources/*.json
#[derive(Debug, Clone, Deserialize)]
pub struct ResourceConfig {
    #[serde(default)]
    pub color_maps: HashMap<String, Vec<ColorDef>>,
    #[serde(default)]
    pub resources: HashMap<String, ResourceDef>,
}

/// Global registry loaded from JSON
static REGISTRY: OnceLock<ResourceConfig> = OnceLock::new();

/// Get the resource registry (loads from embedded JSON on first access)
pub fn get_registry() -> &'static ResourceConfig {
    REGISTRY.get_or_init(|| {
        let mut final_config = ResourceConfig {
            color_maps: HashMap::new(),
            resources: HashMap::new(),
        };

        for content in RESOURCE_FILES {
            let partial: ResourceConfig = serde_json::from_str(content)
                .unwrap_or_else(|e| panic!("Failed to parse embedded resource JSON: {}", e));
            final_config.color_maps.extend(partial.color_maps);
            final_config.resources.extend(partial.resources);
        }

        final_config
    })
}

/// Get a resource definition by key
pub fn get_resource(key: &str) -> Option<&'static ResourceDef> {
    get_registry().resources.get(key)
}

/// Get all resource keys (for autocomplete)
/// Excludes resources that require a parent context (like log-streams, ecs-tasks, etc.)
pub fn get_all_resource_keys() -> Vec<&'static str> {
    get_registry()
        .resources
        .iter()
        .filter(|(_, def)| !def.requires_parent)
        .map(|(key, _)| key.as_str())
        .collect()
}

/// Get a color map by name
pub fn get_color_map(name: &str) -> Option<&'static Vec<ColorDef>> {
    get_registry().color_maps.get(name)
}

/// Get color for a value based on color map name
pub fn get_color_for_value(color_map_name: &str, value: &str) -> Option<[u8; 3]> {
    get_color_map(color_map_name)?
        .iter()
        .find(|c| c.value == value)
        .map(|c| c.color)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_registry_loads_successfully() {
        let registry = get_registry();
        assert!(
            !registry.resources.is_empty(),
            "Registry should have resources"
        );
    }

    #[test]
    fn test_ec2_instances_resource_exists() {
        let resource = get_resource("ec2-instances");
        assert!(resource.is_some(), "EC2 instances resource should exist");

        let resource = resource.unwrap();
        assert_eq!(resource.display_name, "EC2 Instances");
        assert_eq!(resource.service, "ec2");
        assert_eq!(resource.sdk_method, "describe_instances");
        assert!(
            !resource.columns.is_empty(),
            "EC2 instances should have columns"
        );
    }

    /// MSK answers in lowerCamelCase on the wire even though the AWS CLI prints
    /// PascalCase. Mapping the CLI's names silently yields an empty cluster list.
    #[test]
    fn test_msk_clusters_map_wire_casing() {
        let resource = get_resource("msk-clusters").expect("MSK clusters resource should exist");
        assert_eq!(resource.service, "kafka");

        let api = resource
            .api_config
            .as_ref()
            .expect("MSK needs an api_config");
        assert_eq!(api.response_root.as_deref(), Some("/clusterInfoList"));

        let sources: Vec<&str> = [
            "ClusterName",
            "ClusterArn",
            "State",
            "KafkaVersion",
            "Brokers",
        ]
        .iter()
        .map(|field| {
            resource
                .field_mappings
                .get(*field)
                .unwrap_or_else(|| panic!("MSK should map {}", field))
                .source
                .as_str()
        })
        .collect();
        assert_eq!(
            sources,
            vec![
                "/clusterName",
                "/clusterArn",
                "/state",
                "/provisioned/currentBrokerSoftwareInfo/kafkaVersion",
                "/provisioned/numberOfBrokerNodes",
            ]
        );
    }

    /// The STATE column's colour only shows on unselected rows, so an account with
    /// a single cluster can never reveal a mis-named map or an unmapped state.
    #[test]
    fn test_msk_cluster_states_resolve_to_colours() {
        let resource = get_resource("msk-clusters").unwrap();
        let state_column = resource
            .columns
            .iter()
            .find(|c| c.json_path == "State")
            .expect("MSK should have a STATE column");
        let map_name = state_column
            .color_map
            .as_deref()
            .expect("STATE column should be colour-mapped");

        const GREEN: [u8; 3] = [0, 255, 0];
        const YELLOW: [u8; 3] = [255, 255, 0];
        const RED: [u8; 3] = [255, 0, 0];
        for (state, expected) in [
            ("ACTIVE", GREEN),
            ("CREATING", YELLOW),
            ("UPDATING", YELLOW),
            ("HEALING", YELLOW),
            ("MAINTENANCE", YELLOW),
            ("REBOOTING_BROKER", YELLOW),
            ("DELETING", RED),
            ("FAILED", RED),
        ] {
            assert_eq!(
                get_color_for_value(map_name, state),
                Some(expected),
                "MSK state {} should be colour-mapped",
                state
            );
        }
    }

    #[test]
    fn test_iam_users_resource_exists() {
        let resource = get_resource("iam-users");
        assert!(resource.is_some(), "IAM users resource should exist");

        let resource = resource.unwrap();
        assert_eq!(resource.service, "iam");
        assert!(resource.is_global, "IAM should be a global service");
    }

    #[test]
    fn test_iam_users_has_sub_resources() {
        let resource = get_resource("iam-users").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "IAM users should have sub-resources"
        );

        let policy_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "iam-user-policies");
        assert!(
            policy_sub.is_some(),
            "IAM users should have policies sub-resource"
        );
    }

    #[test]
    fn test_ec2_instances_has_actions() {
        let resource = get_resource("ec2-instances").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "EC2 instances should have actions"
        );

        let start_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "start_instance");
        assert!(start_action.is_some(), "EC2 should have start action");

        let reboot_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "reboot_instance");
        assert!(reboot_action.is_some(), "EC2 should have reboot action");

        let terminate_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "terminate_instance");
        assert!(
            terminate_action.is_some(),
            "EC2 should have terminate action"
        );
        assert!(
            terminate_action.unwrap().requires_confirm(),
            "Terminate should require confirmation"
        );
    }

    #[test]
    fn test_get_all_resource_keys() {
        let keys = get_all_resource_keys();
        assert!(keys.len() >= 30, "Should have at least 30 resource types");
        assert!(
            keys.contains(&"ec2-instances"),
            "Should contain ec2-instances"
        );
        assert!(
            keys.contains(&"lambda-functions"),
            "Should contain lambda-functions"
        );
        assert!(keys.contains(&"s3-buckets"), "Should contain s3-buckets");
    }

    #[test]
    fn test_common_color_maps_exist() {
        let state_map = get_color_map("state");
        assert!(state_map.is_some(), "State color map should exist");

        let bool_map = get_color_map("bool");
        assert!(bool_map.is_some(), "Bool color map should exist");
    }

    #[test]
    fn test_get_color_for_running_state() {
        let color = get_color_for_value("state", "running");
        assert!(color.is_some(), "Should have color for 'running' state");
        // Green color
        assert_eq!(color.unwrap(), [0, 255, 0]);
    }

    #[test]
    fn test_rds_has_sub_resources() {
        let resource = get_resource("rds-instances").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "RDS should have sub-resources"
        );

        let snapshot_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "rds-snapshots");
        assert!(
            snapshot_sub.is_some(),
            "RDS should have snapshots sub-resource"
        );
    }

    #[test]
    fn test_ecs_has_sub_resources() {
        let resource = get_resource("ecs-clusters").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "ECS clusters should have sub-resources"
        );

        let services_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "ecs-services");
        assert!(
            services_sub.is_some(),
            "ECS should have services sub-resource"
        );

        let tasks_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "ecs-tasks");
        assert!(tasks_sub.is_some(), "ECS should have tasks sub-resource");
    }

    #[test]
    fn test_lambda_has_actions() {
        let resource = get_resource("lambda-functions").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "Lambda functions should have actions"
        );

        let invoke_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "invoke_function");
        assert!(invoke_action.is_some(), "Lambda should have invoke action");
    }

    /// CloudWatch migrated to the `GraniteServiceVersion20100801` JSON target;
    /// the resource must resolve a service entry that produces that exact
    /// header, otherwise every request 404s. The wire was verified live.
    #[test]
    fn cloudwatch_alarms_resolves_the_monitoring_json_target() {
        let resource = get_resource("cloudwatch-alarms").expect("cloudwatch-alarms");
        let api = resource.api_config.as_ref().expect("api_config");

        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::Json,
            "cloudwatch-alarms must speak the JSON target protocol"
        );
        let service_name = api.service_name.as_deref().unwrap_or(&resource.service);
        let service = crate::aws::http::get_service(service_name)
            .unwrap_or_else(|| panic!("uses unknown service {service_name}"));
        assert_eq!(
            service.target_prefix,
            Some("GraniteServiceVersion20100801"),
            "the live cloudwatch API only answers the GraniteServiceVersion target"
        );
        assert_eq!(api.action.as_deref(), Some("DescribeAlarms"));
        assert_eq!(
            api.response_root.as_deref(),
            Some("/MetricAlarms"),
            "MetricAlarms is the list element in the DescribeAlarms response"
        );
    }

    /// The id field has to be mapped, or describe silently sends an empty
    /// string (AGENTS.md invariant).
    #[test]
    fn cloudwatch_alarms_id_is_mapped() {
        let resource = get_resource("cloudwatch-alarms").expect("cloudwatch-alarms");
        assert_eq!(resource.id_field, "AlarmName");
        assert!(
            resource.field_mappings.contains_key("AlarmName"),
            "id field AlarmName must be mapped"
        );
        // Column json_paths must have roots in field_mappings.
        for col in &resource.columns {
            let root = col.json_path.split('.').next().unwrap_or("");
            assert!(
                resource.field_mappings.contains_key(&col.json_path)
                    || resource.field_mappings.contains_key(root),
                "column {} json_path {} root {} not in field_mappings",
                col.header,
                col.json_path,
                root
            );
        }
    }

    /// Dashboards share the monitoring (query-mode) JSON target and must resolve
    /// the same service entry as alarms, and their list element is
    /// `DashboardEntries`.
    #[test]
    fn cloudwatch_dashboards_use_the_monitoring_query_mode_target() {
        let resource = get_resource("cloudwatch-dashboards").expect("cloudwatch-dashboards");
        let api = resource.api_config.as_ref().expect("api_config");
        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::Json,
            "cloudwatch-dashboards must speak the JSON target protocol"
        );
        let service_name = api.service_name.as_deref().unwrap_or(&resource.service);
        let service = crate::aws::http::get_service(service_name)
            .unwrap_or_else(|| panic!("uses unknown service {service_name}"));
        assert_eq!(
            service.target_prefix,
            Some("GraniteServiceVersion20100801"),
            "dashboards must hit the same Granite JSON endpoint as alarms"
        );
        assert_eq!(api.action.as_deref(), Some("ListDashboards"));
        assert_eq!(api.response_root.as_deref(), Some("/DashboardEntries"));
        assert_eq!(resource.id_field, "DashboardName");
        assert!(
            resource.field_mappings.contains_key("DashboardName"),
            "id field DashboardName must be mapped"
        );
    }

    /// Aurora clusters use the DescribeDBClustersResult wrapper that RDS wraps
    /// list elements in (unlike a bare single-level Set), and tag on TagList.
    #[test]
    fn aurora_clusters_hit_the_dbclusters_wrapper_and_paginate_on_marker() {
        let resource = get_resource("rds-aurora-clusters").expect("rds-aurora-clusters");
        let api = resource.api_config.as_ref().expect("api_config");
        assert_eq!(api.action.as_deref(), Some("DescribeDBClusters"));
        assert_eq!(
            api.response_root.as_deref(),
            Some("/DescribeDBClustersResponse/DescribeDBClustersResult/DBClusters"),
            "Aurora clusters must read from the DescribeDBClustersResult wrapper"
        );
        assert_eq!(
            resource.id_field, "DBClusterIdentifier",
            "Aurora id field must be DBClusterIdentifier"
        );
        let pag = api.pagination.as_ref().expect("pagination");
        assert_eq!(pag.input_token.as_deref(), Some("Marker"));
        assert_eq!(
            pag.output_token.as_deref(),
            Some("/DescribeDBClustersResponse/DescribeDBClustersResult/Marker")
        );
        assert_eq!(pag.max_results_param.as_deref(), Some("MaxRecords"));
        let tags = resource
            .field_mappings
            .get("Tags")
            .expect("Tags mapping on Aurora clusters");
        assert_eq!(
            tags.transform.as_deref(),
            Some("tags_to_map"),
            "Aurora clusters tag on TagList, must be mapped to a map"
        );
    }

    /// LookupEvents returns a flat `Events` array (no result wrapper) and pages
    /// on NextToken. EventTime arrives as an ISO-8601 string in the JSON API,
    /// so it must NOT be passed through the epoch-millis formatter.
    #[test]
    fn cloudtrail_events_read_from_flat_events_array_and_keep_iso_time() {
        let resource = get_resource("cloudtrail-events").expect("cloudtrail-events");
        let api = resource.api_config.as_ref().expect("api_config");
        assert_eq!(api.action.as_deref(), Some("LookupEvents"));
        assert_eq!(
            api.response_root.as_deref(),
            Some("/Events"),
            "LookupEvents returns a bare Events array, not a wrapper"
        );
        let pag = api.pagination.as_ref().expect("pagination");
        assert_eq!(pag.input_token.as_deref(), Some("NextToken"));
        assert_eq!(pag.output_token.as_deref(), Some("/NextToken"));
        let time = resource.field_mappings.get("EventTime").expect("EventTime");
        assert_eq!(
            time.transform.as_deref(),
            None,
            "EventTime is ISO-8601 in this JSON API, not epoch millis"
        );
        assert!(resource.field_mappings.contains_key("EventId"));
    }

    /// Lambda aliases/versions are reached from a function row; their REST path
    /// placeholder must line up with the filter_param used to scope the child.
    /// Layers is standalone and nests its latest version under
    /// `LatestMatchingVersion`.
    #[test]
    fn lambda_aliases_and_versions_are_parent_scoped_children_with_matching_path() {
        let functions = get_resource("lambda-functions").expect("lambda-functions");
        for key in ["lambda-aliases", "lambda-versions"] {
            let sub = functions
                .sub_resources
                .iter()
                .find(|s| s.resource_key == key)
                .unwrap_or_else(|| panic!("lambda-functions must declare {key} as a sub-resource"));
            assert_eq!(
                sub.filter_param, "functionName",
                "{key} filter_param must match the {{functionName}} path placeholder"
            );
            assert_eq!(
                sub.parent_id_field, "FunctionName",
                "{key} parent_id_field must read FunctionName from the parent row"
            );
            let child = get_resource(key).expect(key);
            assert!(
                child.requires_parent,
                "{key} must require a parent function"
            );
            let api = child.api_config.as_ref().expect(key);
            assert!(
                api.path.as_deref().expect(key).contains("{functionName}"),
                "{key} path must carry the {{functionName}} placeholder"
            );
            assert_eq!(
                api.response_root.as_deref().unwrap(),
                format!(
                    "/{}",
                    if key == "lambda-aliases" {
                        "Aliases"
                    } else {
                        "Versions"
                    }
                )
            );
        }

        let layers = get_resource("lambda-layers").expect("lambda-layers");
        assert!(
            !layers.requires_parent,
            "lambda-layers must list standalone"
        );
        let layers_api = layers.api_config.as_ref().expect("layers api_config");
        assert_eq!(layers_api.response_root.as_deref(), Some("/Layers"));
        assert!(
            layers_api.path.as_deref().unwrap().contains("/2018-10-31/"),
            "ListLayers lives under the 2018-10-31 API, not 2015-03-31; a wrong version \
             makes Lambda answer AccessDenied: unable to determine operation"
        );
        assert!(
            layers.field_mappings.contains_key("Version"),
            "layers must map the nested LatestMatchingVersion"
        );
        assert_eq!(
            layers.field_mappings.get("Version").unwrap().source,
            "/LatestMatchingVersion/Version",
            "layers Version comes from the nested latest-matching-version object"
        );
    }

    /// Step Functions is a clean JSON-RPC service (X-Amz-Target AWSStepFunctions,
    /// unlike CloudWatch's query-mode quirk), so it needs no special header.
    /// Executions are a parent-scoped JSON child whose parent id flows into the
    /// body as stateMachineArn, same mechanism ECR images use.
    #[test]
    fn stepfunctions_uses_the_json_target_and_executions_are_parent_scoped() {
        let sm =
            get_resource("stepfunctions-state-machines").expect("stepfunctions-state-machines");
        let sm_api = sm.api_config.as_ref().expect("api_config");
        assert_eq!(
            sm_api.protocol,
            crate::resource::protocol::ApiProtocol::Json
        );
        assert_eq!(sm_api.action.as_deref(), Some("ListStateMachines"));
        assert_eq!(sm_api.response_root.as_deref(), Some("/stateMachines"));
        let service = crate::aws::http::get_service("states")
            .unwrap_or_else(|| panic!("states service must be registered"));
        assert_eq!(
            service.target_prefix,
            Some("AWSStepFunctions"),
            "states must target AWSStepFunctions"
        );
        let ex = get_resource("stepfunctions-executions").expect("stepfunctions-executions");
        assert!(ex.requires_parent, "executions need a parent state machine");
        let sub = sm
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "stepfunctions-executions")
            .expect("state machines declare executions");
        assert_eq!(sub.filter_param, "stateMachineArn");
        assert_eq!(sub.parent_id_field, "stateMachineArn");
        assert!(ex.field_mappings.contains_key("stateMachineArn"));
        assert_eq!(
            ex.api_config.as_ref().unwrap().response_root.as_deref(),
            Some("/executions")
        );
    }

    /// EFS is a REST-JSON GET service whose children (access points, mount
    /// targets) are scoped by a URI query parameter, not a path segment the way
    /// EKS/Lambda children are. That only works if the resource declares the
    /// param in `query_params`; otherwise the parent FileSystemId would be
    /// silently dropped and DescribeMountTargets (which requires a filter)
    /// would fail its own request. Pin the declaration here.
    #[test]
    fn efs_children_scope_file_system_by_uri_query_param() {
        let fs = get_resource("efs-file-systems").expect("efs-file-systems");
        let fs_api = fs.api_config.as_ref().expect("api_config");
        assert_eq!(
            fs_api.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(fs_api.path.as_deref(), Some("/2015-02-01/file-systems"));
        assert_eq!(fs_api.response_root.as_deref(), Some("/FileSystems"));
        let service = crate::aws::http::get_service("efs")
            .unwrap_or_else(|| panic!("efs service must be registered"));
        assert_eq!(
            service.signing_name, "elasticfilesystem",
            "efs signs with elasticfilesystem, not efs"
        );

        for (key, sub, root, expected_token_pair) in [
            (
                "efs-access-points",
                "efs-access-points",
                "/AccessPoints",
                ("NextToken", "/NextToken"),
            ),
            (
                "efs-mount-targets",
                "efs-mount-targets",
                "/MountTargets",
                ("Marker", "/NextMarker"),
            ),
        ] {
            let child = get_resource(key).expect(key);
            assert!(child.requires_parent, "{key} needs a parent file system");
            let child_api = child.api_config.as_ref().expect("api_config");
            assert_eq!(
                child_api.query_params.as_slice(),
                &["FileSystemId".to_string()],
                "{key} must declare FileSystemId as a URI query param"
            );
            assert_eq!(child_api.response_root.as_deref(), Some(root));
            assert_eq!(
                child_api.pagination.as_ref().map(|p| (
                    p.input_token.as_deref().unwrap_or("").to_string(),
                    p.output_token.as_deref().unwrap_or("").to_string()
                )),
                Some((
                    expected_token_pair.0.to_string(),
                    expected_token_pair.1.to_string()
                )),
                "{key} must page on the EFS token pair"
            );
            let declared = fs
                .sub_resources
                .iter()
                .find(|s| s.resource_key == sub)
                .expect("file systems declare the child");
            assert_eq!(declared.filter_param, "FileSystemId");
            assert_eq!(declared.parent_id_field, "FileSystemId");
        }
    }

    /// FSx is a plain JSON-RPC service under the AWSSimbaAPIService target.
    /// DescribeFileSystems caps MaxResults at 50, so a neighbour's 100 would be
    /// out of bounds; pin the cap plus the NextToken token pair here.
    #[test]
    fn fsx_file_systems_use_the_simba_target_and_cap_at_50() {
        let fsx = get_resource("fsx-file-systems").expect("fsx-file-systems");
        let api = fsx.api_config.as_ref().expect("api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.action.as_deref(), Some("DescribeFileSystems"));
        assert_eq!(api.response_root.as_deref(), Some("/FileSystems"));
        let pagination = api.pagination.as_ref().expect("pagination");
        assert_eq!(pagination.input_token.as_deref(), Some("NextToken"));
        assert_eq!(pagination.output_token.as_deref(), Some("/NextToken"));
        assert_eq!(
            pagination.max_results,
            Some(50),
            "DescribeFileSystems rejects MaxResults above 50"
        );
        let service = crate::aws::http::get_service("fsx")
            .unwrap_or_else(|| panic!("fsx service must be registered"));
        assert_eq!(
            service.target_prefix,
            Some("AWSSimbaAPIService_v20180301"),
            "fsx must target AWSSimbaAPIService_v20180301"
        );
        assert!(
            fsx.field_mappings.contains_key("Tags"),
            "FSx has no native Name field; the name must come from the tags map"
        );
        assert_eq!(
            fsx.field_mappings.get("Tags").unwrap().transform.as_deref(),
            Some("tags_to_map")
        );
    }

    /// EventBridge Scheduler is a REST-JSON GET service (GET /schedules, no
    /// X-Amz-Target), not the JSON-RPC style of the old "events" service. Pin
    /// that so nobody mistakes it for a sibling that needs a target prefix.
    #[test]
    fn scheduler_schedules_are_a_rest_json_get() {
        let sched = get_resource("scheduler-schedules").expect("scheduler-schedules");
        let api = sched.api_config.as_ref().expect("api_config");
        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(api.method.as_deref(), Some("GET"));
        assert_eq!(api.path.as_deref(), Some("/schedules"));
        assert_eq!(api.response_root.as_deref(), Some("/Schedules"));
        let service = crate::aws::http::get_service("scheduler")
            .unwrap_or_else(|| panic!("scheduler service must be registered"));
        assert!(
            service.target_prefix.is_none(),
            "scheduler sends no X-Amz-Target; it is REST-JSON GET"
        );
        let pagination = api.pagination.as_ref().expect("pagination");
        assert_eq!(pagination.input_token.as_deref(), Some("NextToken"));
        assert_eq!(pagination.output_token.as_deref(), Some("/NextToken"));
        assert_eq!(pagination.max_results_param.as_deref(), Some("MaxResults"));
        assert!(
            !sched.requires_parent,
            "schedules list standalone account-wide"
        );
    }

    /// API Gateway v2 (HTTP/WebSocket APIs) reuses the existing "apigateway"
    /// service entry: the v2 CLI is also REST-JSON GET with the same host and
    /// signing name, and the Version param only matters for query-protocol
    /// services. All that differs is the request path "/v2/apis", which lives
    /// in the resource JSON, not the service table.
    #[test]
    fn apigatewayv2_apis_share_the_apigateway_service_entry() {
        let api = get_resource("apigatewayv2-apis").expect("apigatewayv2-apis");
        let cfg = api.api_config.as_ref().expect("api_config");
        assert_eq!(
            cfg.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(cfg.method.as_deref(), Some("GET"));
        assert_eq!(cfg.path.as_deref(), Some("/v2/apis"));
        assert_eq!(cfg.response_root.as_deref(), Some("/Items"));
        let pagination = cfg.pagination.as_ref().expect("pagination");
        assert_eq!(pagination.output_token.as_deref(), Some("/NextToken"));
        let service = crate::aws::http::get_service("apigateway")
            .unwrap_or_else(|| panic!("apigateway service must be registered"));
        assert!(
            service.target_prefix.is_none(),
            "apigatewayv2 sends no X-Amz-Target; it is REST-JSON GET"
        );
    }

    /// ECS task definitions are the classic scalar-string list (the API lists
    /// ARNs only, not full objects). The empty source maps the bare ARN string,
    /// and the three taskdef_arn_* transforms split family:revision out of it.
    /// Pin that the id field (full ARN, needed for a describe) is mapped too.
    #[test]
    fn ecs_task_definitions_map_bare_arn_strings_into_name_family_revision() {
        let td = get_resource("ecs-task-definitions").expect("ecs-task-definitions");
        let cfg = td.api_config.as_ref().expect("api_config");
        assert_eq!(cfg.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(cfg.action.as_deref(), Some("ListTaskDefinitions"));
        assert_eq!(cfg.response_root.as_deref(), Some("/taskDefinitionArns"));
        let arn = td
            .field_mappings
            .get("Arn")
            .expect("ecs-task-definitions must map Arn");
        assert!(
            arn.source.is_empty() || arn.source == "/",
            "bare-string items map through an empty (or /) source, got {:?}",
            arn.source
        );
        let name = td
            .field_mappings
            .get("Name")
            .expect("ecs-task-definitions must map Name");
        assert_eq!(name.transform.as_deref(), Some("taskdef_arn_name"));
        let family = td
            .field_mappings
            .get("Family")
            .expect("ecs-task-definitions must map Family");
        assert_eq!(family.transform.as_deref(), Some("taskdef_arn_family"));
        let revision = td
            .field_mappings
            .get("Revision")
            .expect("ecs-task-definitions must map Revision");
        assert_eq!(revision.transform.as_deref(), Some("taskdef_arn_revision"));
        assert_eq!(td.id_field, "Arn");
        let service = crate::aws::http::get_service("ecs")
            .unwrap_or_else(|| panic!("ecs service must be registered"));
        assert_eq!(
            service.target_prefix,
            Some("AmazonEC2ContainerServiceV20141113")
        );
    }

    /// GuardDuty detectors is another bare-ID list (the API returns DetectorIds
    /// only), so it lists via GET /detector but has no describe -- a bare-string
    /// id cannot be extracted for the {resource_id} path, the same reason the
    /// ECS task-definition resource lists IDs with no describe. It is real
    /// rest-json (no X-Amz-Target). Pin the scalar-string mapping and service.
    #[test]
    fn guardduty_detectors_list_bare_ids_without_describe() {
        let det = get_resource("guardduty-detectors").expect("guardduty-detectors");
        assert!(
            det.describe_config.is_none(),
            "bare-string ids cannot describe; no describe_config expected"
        );
        let cfg = det.api_config.as_ref().expect("api_config");
        assert_eq!(
            cfg.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(cfg.method.as_deref(), Some("GET"));
        assert_eq!(cfg.path.as_deref(), Some("/detector"));
        assert_eq!(cfg.response_root.as_deref(), Some("/DetectorIds"));
        let mapped = det
            .field_mappings
            .get("DetectorId")
            .expect("guardduty-detectors must map DetectorId");
        assert!(
            mapped.source.is_empty() || mapped.source == "/",
            "bare-id items map through an empty (or /) source, got {:?}",
            mapped.source
        );
        let service = crate::aws::http::get_service("guardduty")
            .unwrap_or_else(|| panic!("guardduty service must be registered"));
        assert!(
            service.target_prefix.is_none(),
            "guardduty sends no X-Amz-Target; it is REST-JSON"
        );
        assert_eq!(service.api_version, "2017-11-28");
    }

    /// Security Hub standards are a straightforward REST-JSON GET (no
    /// X-Amz-Target), unlike findings which are a POST needing body pagination.
    /// Pin the GET /standards list, the EnabledByDefault boolean transform, and
    /// the securityhub service entry.
    #[test]
    fn securityhub_standards_are_a_rest_json_get() {
        let s = get_resource("securityhub-standards").expect("securityhub-standards");
        let cfg = s.api_config.as_ref().expect("api_config");
        assert_eq!(
            cfg.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(cfg.method.as_deref(), Some("GET"));
        assert_eq!(cfg.path.as_deref(), Some("/standards"));
        assert_eq!(cfg.response_root.as_deref(), Some("/Standards"));
        let enabled = s
            .field_mappings
            .get("EnabledByDefault")
            .expect("securityhub-standards must map EnabledByDefault");
        assert_eq!(enabled.transform.as_deref(), Some("bool_to_yes_no"));
        let service = crate::aws::http::get_service("securityhub")
            .unwrap_or_else(|| panic!("securityhub service must be registered"));
        assert!(
            service.target_prefix.is_none(),
            "securityhub sends no X-Amz-Target; it is REST-JSON GET"
        );
        assert_eq!(service.api_version, "2018-10-26");
    }

    /// Macie2 has no GET list; ListClassificationJobs is a POST whose pagination
    /// (nextToken/maxResults) lives in the JSON body. The rest-json handler only
    /// paginates GET query strings, so this resource needs the boolean that lets
    /// POST requests put pagination in the body.
    #[test]
    fn macie2_classification_jobs_post_with_body_pagination() {
        let s = get_resource("macie2-classification-jobs").expect("macie2-classification-jobs");
        let cfg = s.api_config.as_ref().expect("api_config");
        assert_eq!(
            cfg.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(cfg.method.as_deref(), Some("POST"));
        assert_eq!(cfg.path.as_deref(), Some("/jobs/list"));
        assert_eq!(cfg.response_root.as_deref(), Some("/items"));
        let pag = cfg
            .pagination
            .as_ref()
            .expect("macie2 paginates in the body");
        assert_eq!(pag.max_results_param.as_deref(), Some("maxResults"));
        assert_eq!(pag.input_token.as_deref(), Some("nextToken"));
        let service = crate::aws::http::get_service("macie2")
            .unwrap_or_else(|| panic!("macie2 service must be registered"));
        assert!(
            service.target_prefix.is_none(),
            "macie2 sends no X-Amz-Target; it is REST-JSON"
        );
        assert_eq!(service.api_version, "2020-01-01");
    }

    /// Inspector2 findings list is a POST with body pagination, like Macie2.
    /// Pin the path / root and the severity colour map referenced from the UI.
    #[test]
    fn inspector2_findings_post_with_body_pagination() {
        let s = get_resource("inspector2-findings").expect("inspector2-findings");
        let cfg = s.api_config.as_ref().expect("api_config");
        assert_eq!(
            cfg.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(cfg.method.as_deref(), Some("POST"));
        assert_eq!(cfg.path.as_deref(), Some("/findings/list"));
        assert_eq!(cfg.response_root.as_deref(), Some("/findings"));
        let pag = cfg
            .pagination
            .as_ref()
            .expect("inspector2 paginates in the body");
        assert_eq!(pag.max_results_param.as_deref(), Some("maxResults"));
        assert_eq!(pag.input_token.as_deref(), Some("nextToken"));
        let service = crate::aws::http::get_service("inspector2")
            .unwrap_or_else(|| panic!("inspector2 service must be registered"));
        assert!(
            service.target_prefix.is_none(),
            "inspector2 sends no X-Amz-Target; it is REST-JSON"
        );
        assert_eq!(service.api_version, "2020-06-08");
        let severity = s
            .columns
            .iter()
            .find(|c| c.json_path == "severity")
            .expect("inspector2-findings must show a severity column");
        assert_eq!(severity.color_map.as_deref(), Some("severity"));
    }

    /// Every WAFv2 resource, keyed as it appears in the registry.
    fn wafv2_resources() -> Vec<(&'static String, &'static ResourceDef)> {
        let found: Vec<_> = get_registry()
            .resources
            .iter()
            .filter(|(key, _)| key.starts_with("wafv2-"))
            .collect();
        assert!(!found.is_empty(), "no wafv2 resources in the registry");
        found
    }

    /// Scope as the list call sends it.
    fn list_scope(resource: &ResourceDef) -> &str {
        resource
            .api_config
            .as_ref()
            .expect("wafv2 resources are data-driven")
            .static_params
            .get("Scope")
            .and_then(|scope| scope.as_str())
            .expect("wafv2 list calls must pin a Scope")
    }

    #[test]
    fn wafv2_web_acls_exist_for_both_scopes() {
        let regional = get_resource("wafv2-web-acls").expect("regional web ACLs");
        assert_eq!(list_scope(regional), "REGIONAL");
        assert!(!regional.is_global);

        let cloudfront = get_resource("wafv2-web-acls-cloudfront").expect("CloudFront web ACLs");
        assert_eq!(list_scope(cloudfront), "CLOUDFRONT");
        assert!(
            cloudfront.is_global,
            "CloudFront-scope ACLs are one global set, so the title should not claim a region"
        );
    }

    /// Scope is not optional on any WAFv2 call, and it must be the same one on the
    /// list and the describe. Listing CLOUDFRONT then describing REGIONAL returns a
    /// bare "not found" that looks like the resource vanished.
    #[test]
    fn every_wafv2_resource_uses_one_scope_throughout() {
        for (key, resource) in wafv2_resources() {
            let template = resource
                .describe_config
                .as_ref()
                .and_then(|d| d.body_template.as_deref())
                .unwrap_or_else(|| panic!("{} needs a describe body template", key));

            let compact: String = template.chars().filter(|c| !c.is_whitespace()).collect();
            let expected = format!("\"Scope\":\"{}\"", list_scope(resource));
            assert!(
                compact.contains(&expected),
                "{} lists {} but describes with {}",
                key,
                expected,
                template
            );
        }
    }

    /// CLOUDFRONT-scope objects are only served from us-east-1, and the endpoint comes
    /// from the service entry, not from the resource's own is_global flag.
    #[test]
    fn every_wafv2_resource_calls_the_endpoint_for_its_scope() {
        for (key, resource) in wafv2_resources() {
            let cloudfront = list_scope(resource) == "CLOUDFRONT";

            for service_name in [
                resource
                    .api_config
                    .as_ref()
                    .and_then(|c| c.service_name.as_deref())
                    .unwrap_or(&resource.service),
                resource
                    .describe_config
                    .as_ref()
                    .and_then(|d| d.service_name.as_deref())
                    .unwrap_or(&resource.service),
            ] {
                let service = crate::aws::http::get_service(service_name)
                    .unwrap_or_else(|| panic!("{} uses unknown service {}", key, service_name));
                assert_eq!(
                    service.is_global,
                    cloudfront,
                    "{} is scoped {} but {} points at the {} endpoint",
                    key,
                    list_scope(resource),
                    service_name,
                    if service.is_global {
                        "us-east-1"
                    } else {
                        "selected region"
                    }
                );
            }
        }
    }

    /// Describe needs the name and the id, and only the ARN carries both, so the ARN
    /// has to be the id field and has to be mapped. Miss either and `d` describes an
    /// empty string.
    #[test]
    fn every_wafv2_resource_describes_from_its_arn() {
        for (key, resource) in wafv2_resources() {
            assert_eq!(resource.id_field, "ARN", "{} id field", key);
            assert!(
                resource.field_mappings.contains_key("ARN"),
                "{} does not map ARN, so its id would be blank",
                key
            );
        }
    }

    /// `navigate_to_sub_resource` refuses a key that the current resource does not
    /// declare in `sub_resources`, so an `enter_sub_resource` missing from that list
    /// turns Enter into an error message rather than a drill-in.
    #[test]
    fn enter_sub_resource_is_always_a_declared_sub_resource() {
        let registry = get_registry();
        for (key, resource) in &registry.resources {
            let Some(target) = resource.enter_sub_resource.as_deref() else {
                continue;
            };

            assert!(
                registry.resources.contains_key(target),
                "{} enters {}, which is not a resource",
                key,
                target
            );
            assert!(
                resource
                    .sub_resources
                    .iter()
                    .any(|s| s.resource_key == target),
                "{} enters {} but does not list it in sub_resources, so Enter would fail",
                key,
                target
            );
        }
    }

    /// The records are the reason to open a zone, so Enter drills into them. The zone's
    /// own detail stays reachable on `d`.
    #[test]
    fn hosted_zones_enter_their_records() {
        let zones = get_resource("route53-hosted-zones").expect("route53-hosted-zones");
        assert_eq!(
            zones.enter_sub_resource.as_deref(),
            Some("route53-records"),
            "Enter on a hosted zone should list its records"
        );
    }

    /// ListResourceRecordSets pages on two coordinated tokens (NextRecordName +
    /// NextRecordType). A single-token PaginationConfig cannot represent this, so
    /// the resource must use the multi_token scheme instead.
    #[test]
    fn route53_records_use_multi_token_pagination() {
        let records = get_resource("route53-records").expect("route53-records");
        let config = records
            .api_config
            .as_ref()
            .expect("route53-records needs api_config");
        let pagination = config
            .pagination
            .as_ref()
            .expect("route53-records needs pagination");

        assert!(
            pagination.multi_token.is_some(),
            "route53-records must use multi_token, not single input_token/output_token"
        );
        assert!(
            pagination.input_token.is_none() && pagination.output_token.is_none(),
            "route53-records must not set single-token fields alongside multi_token"
        );

        let multi = pagination.multi_token.as_ref().unwrap();
        assert_eq!(
            multi.len(),
            2,
            "route53-records needs two tokens: name + type"
        );

        let names: Vec<&str> = multi.iter().map(|f| f.query_param.as_str()).collect();
        assert!(
            names.contains(&"name"),
            "route53-records needs a 'name' query param"
        );
        assert!(
            names.contains(&"type"),
            "route53-records needs a 'type' query param"
        );

        assert_eq!(
            pagination.max_results_param.as_deref(),
            Some("maxitems"),
            "route53-records max_results_param"
        );
    }

    /// The EC2 networking family, listed explicitly so that dropping one from the JSON
    /// is a test failure rather than a silently smaller loop.
    const VPC_NETWORKING_KEYS: &[&str] = &[
        "route-tables",
        "internet-gateways",
        "nat-gateways",
        "vpc-endpoints",
        "network-acls",
        "vpc-peering-connections",
        "network-interfaces",
        "elastic-ips",
        "transit-gateways",
    ];

    fn vpc_networking_resources() -> Vec<(&'static str, &'static ResourceDef)> {
        let registry = get_registry();
        VPC_NETWORKING_KEYS
            .iter()
            .map(|key| {
                let resource = registry
                    .resources
                    .get(*key)
                    .unwrap_or_else(|| panic!("{} is not in the registry", key));
                (*key, resource)
            })
            .collect()
    }

    /// EC2's Query protocol wraps every reply in `<Describe...Response>` and every item
    /// in a `<xxxSet><item>`. Get the root wrong and the list comes back empty with no
    /// error at all, which is the single easiest mistake to make in these files.
    #[test]
    fn every_vpc_networking_resource_reads_from_the_ec2_response_envelope() {
        for (key, resource) in vpc_networking_resources() {
            assert_eq!(resource.service, "ec2", "{} service", key);

            let config = resource
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{} needs an api_config", key));
            let action = config
                .action
                .as_deref()
                .unwrap_or_else(|| panic!("{} needs an action", key));
            let root = config
                .response_root
                .as_deref()
                .unwrap_or_else(|| panic!("{} needs a response_root", key));

            let prefix = format!("/{}Response/", action);
            assert!(
                root.starts_with(&prefix),
                "{} calls {} but reads from {}",
                key,
                action,
                root
            );
            assert!(
                root.ends_with("Set/item"),
                "{} reads from {}, which is not an EC2 item set",
                key,
                root
            );
        }
    }

    /// A column whose json_path has no field mapping renders as an empty cell forever.
    /// `Tags.Name` and friends index into a mapped map, so only the first segment has
    /// to exist.
    #[test]
    fn every_vpc_networking_column_has_a_field_mapping() {
        for (key, resource) in vpc_networking_resources() {
            for column in &resource.columns {
                let root_field = column.json_path.split('.').next().unwrap();
                assert!(
                    resource.field_mappings.contains_key(root_field),
                    "{} column {:?} reads {} but nothing maps {}",
                    key,
                    column.header,
                    column.json_path,
                    root_field
                );
            }
        }
    }

    /// DescribeAddresses is the one operation here with no paginator: sending MaxResults
    /// or NextToken earns an InvalidParameterCombination and no rows. Everything else
    /// must page, or long-lived accounts silently show only the first screenful.
    #[test]
    fn vpc_networking_pagination_follows_the_ec2_api() {
        for (key, resource) in vpc_networking_resources() {
            let config = resource.api_config.as_ref().unwrap();
            let action = config.action.as_deref().unwrap();
            let pagination = config.pagination.as_ref();

            if action == "DescribeAddresses" {
                assert!(
                    pagination.is_none(),
                    "{} paginates, but DescribeAddresses rejects MaxResults and NextToken",
                    key
                );
                continue;
            }

            let pagination =
                pagination.unwrap_or_else(|| panic!("{} needs pagination for {}", key, action));
            assert_eq!(
                pagination.input_token.as_deref(),
                Some("NextToken"),
                "{} input token",
                key
            );
            assert_eq!(
                pagination.output_token.as_deref(),
                Some(format!("/{}Response/nextToken", action).as_str()),
                "{} output token",
                key
            );
        }
    }

    #[test]
    fn test_all_resources_have_required_fields() {
        let registry = get_registry();
        for (key, resource) in &registry.resources {
            assert!(
                !resource.display_name.is_empty(),
                "Resource {} should have display_name",
                key
            );
            assert!(
                !resource.service.is_empty(),
                "Resource {} should have service",
                key
            );
            assert!(
                !resource.sdk_method.is_empty(),
                "Resource {} should have sdk_method",
                key
            );
            assert!(
                !resource.id_field.is_empty(),
                "Resource {} should have id_field",
                key
            );
            assert!(
                !resource.name_field.is_empty(),
                "Resource {} should have name_field",
                key
            );
        }
    }

    /// Every describe_field must have a non-empty label and source. A blank label
    /// renders an empty row with no purpose, and a blank source silently returns
    /// null for every resource, showing "-" in every cell.
    #[test]
    fn describe_fields_have_labels_and_sources() {
        let registry = get_registry();
        for (key, resource) in &registry.resources {
            if let Some(ref dc) = resource.describe_config {
                for (i, field) in dc.describe_fields.iter().enumerate() {
                    assert!(
                        !field.label.is_empty(),
                        "{} describe_fields[{}] label is empty",
                        key,
                        i
                    );
                    assert!(
                        !field.source.is_empty(),
                        "{} describe_fields[{}] source is empty",
                        key,
                        i
                    );
                }
            }
        }
    }

    #[test]
    fn eks_clusters_have_nodegroups_as_enter_sub_resource() {
        let clusters = get_resource("eks-clusters").expect("eks-clusters");
        assert_eq!(
            clusters.enter_sub_resource.as_deref(),
            Some("eks-nodegroups"),
            "Enter on an EKS cluster should list node groups"
        );
        assert!(clusters
            .sub_resources
            .iter()
            .any(|s| s.resource_key == "eks-nodegroups"));
        assert!(clusters
            .sub_resources
            .iter()
            .any(|s| s.resource_key == "eks-fargate-profiles"));
        assert!(clusters
            .sub_resources
            .iter()
            .any(|s| s.resource_key == "eks-addons"));
        assert!(clusters
            .sub_resources
            .iter()
            .any(|s| s.resource_key == "eks-updates"));

        // All sub-resources must use 'name' as filter_param
        for sub in &clusters.sub_resources {
            assert_eq!(
                sub.filter_param, "name",
                "{} sub_resource filter_param",
                sub.resource_key
            );
            assert_eq!(
                sub.parent_id_field, "name",
                "{} sub_resource parent_id_field",
                sub.resource_key
            );
        }
    }

    #[test]
    /// EC2's extended attribute columns exist for the picker but start hidden,
    /// so the default table stays the compact 7-column view. A typo in a new
    /// column's visible flag shows up here as a wrong default view.
    fn ec2_instances_extended_columns_start_hidden() {
        let resource = get_resource("ec2-instances").expect("ec2-instances");
        assert!(
            resource.columns.len() > 40,
            "ec2-instances should carry the extended attribute set, got {}",
            resource.columns.len()
        );

        let visible: Vec<&str> = resource
            .columns
            .iter()
            .filter(|c| c.visible)
            .map(|c| c.header.as_str())
            .collect();
        assert_eq!(
            visible,
            vec![
                "NAME",
                "INSTANCE ID",
                "STATE",
                "TYPE",
                "AZ",
                "PUBLIC IP",
                "PRIVATE IP"
            ],
            "ec2-instances default visible columns"
        );

        // Every column's json_path must exist in field_mappings or the cell
        // renders blank with no error.
        for col in &resource.columns {
            let root = col.json_path.split('.').next().unwrap_or("");
            assert!(
                resource.field_mappings.contains_key(root),
                "ec2-instances column {} json_path root {} is not in field_mappings",
                col.header,
                root
            );
        }
    }

    /// A column whose json_path root is missing from field_mappings renders
    /// blank with no error at all — the exact failure mode the AGENTS doc
    /// calls the easiest mistake in these files. Pin it across every resource
    /// so new columns cannot ship silently broken. Resources without
    /// field_mappings (s3-objects, sts-caller-identity) are exempt: their
    /// items are built directly by special-case handlers in dispatch.rs.
    #[test]
    fn every_column_json_path_root_exists_in_field_mappings() {
        let registry = get_registry();
        for (key, resource) in &registry.resources {
            if resource.field_mappings.is_empty() {
                continue;
            }
            for col in &resource.columns {
                let root = col.json_path.split('.').next().unwrap_or("");
                let mapped = resource.field_mappings.contains_key(&col.json_path)
                    || resource.field_mappings.contains_key(root);
                assert!(
                    mapped,
                    "{} column {} reads json_path {} whose root {} is not in field_mappings",
                    key, col.header, col.json_path, root
                );
            }
        }
    }

    #[test]
    fn eks_sub_resources_require_parent() {
        for key in &[
            "eks-nodegroups",
            "eks-fargate-profiles",
            "eks-addons",
            "eks-updates",
        ] {
            let resource = get_resource(key).unwrap_or_else(|| panic!("{}", key));
            assert!(
                resource.requires_parent,
                "{} must require a parent cluster",
                key
            );
            assert!(
                resource.describe_config.is_some(),
                "{} must have a describe_config",
                key
            );
            let dc = resource.describe_config.as_ref().unwrap();
            assert!(
                !dc.describe_fields.is_empty(),
                "{} must have describe_fields for formatted overview",
                key
            );
        }
    }

    #[test]
    fn test_elbv2_load_balancers_resource_exists() {
        let resource = get_resource("elbv2-load-balancers");
        assert!(
            resource.is_some(),
            "ELBv2 load balancers resource should exist"
        );

        let resource = resource.unwrap();
        assert_eq!(resource.display_name, "Load Balancers");
        assert_eq!(resource.service, "elbv2");
        assert_eq!(resource.sdk_method, "describe_load_balancers");
    }

    #[test]
    fn test_elbv2_has_sub_resources() {
        let resource = get_resource("elbv2-load-balancers").unwrap();
        assert!(
            !resource.sub_resources.is_empty(),
            "ELBv2 load balancers should have sub-resources"
        );

        let listeners_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-listeners");
        assert!(
            listeners_sub.is_some(),
            "ELBv2 should have listeners sub-resource"
        );

        let target_groups_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-target-groups");
        assert!(
            target_groups_sub.is_some(),
            "ELBv2 should have target groups sub-resource"
        );
    }

    #[test]
    fn test_elbv2_listeners_has_rules_sub_resource() {
        let resource = get_resource("elbv2-listeners").unwrap();

        let rules_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-rules");
        assert!(
            rules_sub.is_some(),
            "ELBv2 listeners should have rules sub-resource"
        );
    }

    #[test]
    fn test_elbv2_target_groups_has_targets_sub_resource() {
        let resource = get_resource("elbv2-target-groups").unwrap();

        let targets_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "elbv2-targets");
        assert!(
            targets_sub.is_some(),
            "ELBv2 target groups should have targets sub-resource"
        );
    }

    #[test]
    fn test_elbv2_health_color_map_exists() {
        let health_map = get_color_map("health");
        assert!(health_map.is_some(), "Health color map should exist");

        let color = get_color_for_value("health", "healthy");
        assert!(color.is_some(), "Should have color for 'healthy' state");
        assert_eq!(color.unwrap(), [0, 255, 0]); // Green color
    }

    #[test]
    fn test_secretsmanager_has_view_value_action() {
        let resource = get_resource("secretsmanager-secrets").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "Secrets Manager should have actions"
        );

        let view_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "get_secret_value");
        assert!(
            view_action.is_some(),
            "Secrets Manager should have get_secret_value action"
        );

        let view_action = view_action.unwrap();
        assert!(
            view_action.show_result,
            "get_secret_value action should have show_result=true"
        );
        assert_eq!(
            view_action.shortcut.as_deref(),
            Some("x"),
            "get_secret_value should use 'x' shortcut"
        );
    }

    #[test]
    fn test_ssm_parameters_has_view_value_action() {
        let resource = get_resource("ssm-parameters").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "SSM Parameters should have actions"
        );

        let view_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "get_parameter");
        assert!(
            view_action.is_some(),
            "SSM Parameters should have get_parameter action"
        );

        let view_action = view_action.unwrap();
        assert!(
            view_action.show_result,
            "get_parameter action should have show_result=true"
        );
        assert_eq!(
            view_action.shortcut.as_deref(),
            Some("x"),
            "get_parameter should use 'x' shortcut"
        );
    }

    #[test]
    fn test_cloudformation_stacks_has_sub_resources() {
        let resource = get_resource("cloudformation-stacks").unwrap();
        assert_eq!(resource.display_name, "CloudFormation Stacks");

        let events_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "cloudformation-events");
        assert!(
            events_sub.is_some(),
            "Stacks should have events sub-resource"
        );
        assert_eq!(events_sub.unwrap().shortcut, "e");

        let outputs_sub = resource
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "cloudformation-outputs");
        assert!(
            outputs_sub.is_some(),
            "Stacks should have outputs sub-resource"
        );
        assert_eq!(outputs_sub.unwrap().shortcut, "o");
    }

    #[test]
    fn test_cloudformation_events_resource() {
        let resource = get_resource("cloudformation-events").unwrap();
        assert_eq!(resource.display_name, "Stack Events");
        assert!(
            resource.requires_parent,
            "Events should require a parent stack"
        );
        assert!(
            resource.preserve_order,
            "Events should preserve API order (chronological)"
        );

        let col_headers: Vec<&str> = resource.columns.iter().map(|c| c.header.as_str()).collect();
        assert!(col_headers.contains(&"TIMESTAMP"));
        assert!(col_headers.contains(&"STATUS"));
        assert!(col_headers.contains(&"LOGICAL ID"));
    }

    #[test]
    fn test_cloudformation_outputs_resource() {
        let resource = get_resource("cloudformation-outputs").unwrap();
        assert_eq!(resource.display_name, "Stack Outputs");
        assert!(
            resource.requires_parent,
            "Outputs should require a parent stack"
        );

        let col_headers: Vec<&str> = resource.columns.iter().map(|c| c.header.as_str()).collect();
        assert!(col_headers.contains(&"KEY"));
        assert!(col_headers.contains(&"VALUE"));
    }

    #[test]
    fn test_cfn_state_colors_exist() {
        let create_complete = get_color_for_value("state", "CREATE_COMPLETE");
        assert_eq!(create_complete, Some([0, 255, 0]));

        let create_failed = get_color_for_value("state", "CREATE_FAILED");
        assert_eq!(create_failed, Some([255, 0, 0]));

        let create_in_progress = get_color_for_value("state", "CREATE_IN_PROGRESS");
        assert_eq!(create_in_progress, Some([255, 255, 0]));

        let delete_complete = get_color_for_value("state", "DELETE_COMPLETE");
        assert_eq!(delete_complete, Some([128, 128, 128]));
    }
}
