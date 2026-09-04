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
    include_str!("../resources/appmesh.json"),
    include_str!("../resources/apigateway.json"),
    include_str!("../resources/apigatewayv2.json"),
    include_str!("../resources/appsync.json"),
    include_str!("../resources/athena.json"),
    include_str!("../resources/autoscaling.json"),
    include_str!("../resources/backup.json"),
    include_str!("../resources/cloudformation.json"),
    include_str!("../resources/cloudfront.json"),
    include_str!("../resources/cloudtrail.json"),
    include_str!("../resources/cloudwatch.json"),
    include_str!("../resources/codebuild.json"),
    include_str!("../resources/config.json"),
    include_str!("../resources/codepipeline.json"),
    include_str!("../resources/cognito.json"),
    include_str!("../resources/common.json"),
    include_str!("../resources/dynamodb.json"),
    include_str!("../resources/datasync.json"),
    include_str!("../resources/docdb.json"),
    include_str!("../resources/ec2.json"),
    include_str!("../resources/ecr.json"),
    include_str!("../resources/ecs.json"),
    include_str!("../resources/ecs-task-definitions.json"),
    include_str!("../resources/efs.json"),
    include_str!("../resources/emr.json"),
    include_str!("../resources/eks.json"),
    include_str!("../resources/elasticache.json"),
    include_str!("../resources/elbv2.json"),
    include_str!("../resources/eventbridge.json"),
    include_str!("../resources/firehose.json"),
    include_str!("../resources/fsx.json"),
    include_str!("../resources/glue.json"),
    include_str!("../resources/guardduty.json"),
    include_str!("../resources/health.json"),
    include_str!("../resources/iam.json"),
    include_str!("../resources/inspector2.json"),
    include_str!("../resources/kinesis.json"),
    include_str!("../resources/kms.json"),
    include_str!("../resources/lambda.json"),
    include_str!("../resources/macie2.json"),
    include_str!("../resources/mq.json"),
    include_str!("../resources/msk.json"),
    include_str!("../resources/neptune.json"),
    include_str!("../resources/rds.json"),
    include_str!("../resources/redshift.json"),
    include_str!("../resources/resource-groups.json"),
    include_str!("../resources/route53.json"),
    include_str!("../resources/route53resolver.json"),
    include_str!("../resources/s3.json"),
    include_str!("../resources/scheduler.json"),
    include_str!("../resources/servicediscovery.json"),
    include_str!("../resources/secretsmanager.json"),
    include_str!("../resources/transfer.json"),
    include_str!("../resources/trustedadvisor.json"),
    include_str!("../resources/vpc-lattice.json"),
    include_str!("../resources/securityhub.json"),
    include_str!("../resources/service-quotas.json"),
    include_str!("../resources/shield.json"),
    include_str!("../resources/sns.json"),
    include_str!("../resources/sso.json"),
    include_str!("../resources/sqs.json"),
    include_str!("../resources/ssm.json"),
    include_str!("../resources/stepfunctions.json"),
    include_str!("../resources/sts.json"),
    include_str!("../resources/vpc.json"),
    include_str!("../resources/vpc-networking.json"),
    include_str!("../resources/waf.json"),
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
    /// A column that can never be hidden. The id column keys describe and
    /// navigation, so hiding it would strand the row with no way to act on
    /// it. Saved preferences cannot remove it, and the picker refuses to
    /// toggle it off.
    #[serde(default)]
    pub locked: bool,
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

    /// If true, the selected row already carries the full resource and describe
    /// renders it directly instead of making another API call. Route53 records
    /// are the canonical case: ListResourceRecordSets returns every record in
    /// full, there is no GetResourceRecordSet to refine it, and a re-fetch scoped
    /// by name/type/setidentifier would be redundant and lossy on pagination.
    #[serde(default)]
    pub describe_from_row: bool,

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
    /// so it must go through the seconds formatter (which parses ISO-8601),
    /// not the raw epoch-millis formatter.
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
            Some("format_epoch_seconds"),
            "EventTime is ISO-8601 in this JSON API, which format_epoch_seconds parses"
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

    /// Shield is classic AWS JSON-RPC: X-Amz-Target = AWSShield_20160616.<Action>,
    /// POST /, served from a regional host. Pin the target prefix (the json handler
    /// builds the header from it) and the NextToken body pagination both lists use.
    #[test]
    fn shield_lists_use_the_aws_shield_json_target() {
        let service = crate::aws::http::get_service("shield")
            .unwrap_or_else(|| panic!("shield service must be registered"));
        assert_eq!(service.target_prefix, Some("AWSShield_20160616"));
        assert_eq!(service.api_version, "2016-06-02");
        assert!(
            !service.is_global,
            "shield answers from shield.<region>.<domain>, not a region-less host"
        );
        for (key, expected_root) in [
            ("shield-protections", "/Protections"),
            ("shield-protection-groups", "/ProtectionGroups"),
        ] {
            let s = get_resource(key).unwrap_or_else(|| panic!("{}", key));
            let api = s
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} has api_config"));
            assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
            assert_eq!(api.response_root.as_deref(), Some(expected_root));
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} paginates in the body"));
            assert_eq!(pag.input_token.as_deref(), Some("NextToken"));
            assert_eq!(pag.output_token.as_deref(), Some("/NextToken"));
            assert_eq!(pag.max_results_param.as_deref(), Some("MaxResults"));
        }
    }

    /// Classic WAF (v1) is a global region-less service -- waf.amazonaws.com,
    /// signed as us-east-1, the same shape as IAM. Every List op shares the same
    /// NextMarker/Limit pagination, distinct from the NextToken of most services.
    #[test]
    fn waf_lists_hit_the_regionless_host_with_next_marker_pagination() {
        let service = crate::aws::http::get_service("waf")
            .unwrap_or_else(|| panic!("waf service must be registered"));
        assert!(
            service.is_global,
            "classic WAF is waf.amazonaws.com; the region must never be in the host"
        );
        assert_eq!(service.target_prefix, Some("AWSWAF_20150824"));
        assert_eq!(service.api_version, "2015-08-24");
        for (key, expected_root) in [
            ("waf-web-acls", "/WebACLs"),
            ("waf-rules", "/Rules"),
            ("waf-ip-sets", "/IPSets"),
        ] {
            let s = get_resource(key).unwrap_or_else(|| panic!("{}", key));
            let api = s
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} has api_config"));
            assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
            assert_eq!(api.response_root.as_deref(), Some(expected_root));
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} paginates"));
            assert_eq!(pag.input_token.as_deref(), Some("NextMarker"));
            assert_eq!(pag.output_token.as_deref(), Some("/NextMarker"));
            assert_eq!(pag.max_results_param.as_deref(), Some("Limit"));
        }
    }

    /// DescribeKeyPairs and DescribePlacementGroups take no NextToken/MaxResults
    /// at all (like DescribeAddresses) -- copying a neighbour's pagination block
    /// makes EC2 answer InvalidParameterCombination and the list renders empty.
    /// Pin that both omit pagination, and that their wire wrappers stay Set-style.
    #[test]
    fn ec2_key_pairs_and_placement_groups_do_not_paginate() {
        for (key, root) in [
            ("ec2-key-pairs", "/DescribeKeyPairsResponse/keySet/item"),
            (
                "ec2-placement-groups",
                "/DescribePlacementGroupsResponse/placementGroupSet/item",
            ),
        ] {
            let s = get_resource(key).unwrap_or_else(|| panic!("{}", key));
            let api = s
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} has api_config"));
            assert!(
                api.pagination.is_none(),
                "{key} must not paginate: DescribeKeyPairs/DescribePlacementGroups reject MaxResults/NextToken"
            );
            assert_eq!(api.response_root.as_deref(), Some(root));
        }
    }

    /// DescribeLaunchTemplates answers with a `<launchTemplates>` wrapper, not the
    /// usual `<xxxSet>` -- and unlike key pairs / placement groups it does
    /// paginate. DescribeHosts uses the standard hostSet wrapper. Pin both
    /// response roots against the wire so the non-Set one cannot be "fixed" back
    /// into a Set by an over-literal AGENTS.md reader.
    #[test]
    fn ec2_launch_templates_and_hosts_paginate_and_keep_their_wire_wrappers() {
        for (key, root, output_token) in [
            (
                "ec2-launch-templates",
                "/DescribeLaunchTemplatesResponse/launchTemplates/item",
                "/DescribeLaunchTemplatesResponse/nextToken",
            ),
            (
                "ec2-hosts",
                "/DescribeHostsResponse/hostSet/item",
                "/DescribeHostsResponse/nextToken",
            ),
        ] {
            let s = get_resource(key).unwrap_or_else(|| panic!("{}", key));
            let api = s
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} has api_config"));
            assert_eq!(api.response_root.as_deref(), Some(root));
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} paginates"));
            assert_eq!(pag.input_token.as_deref(), Some("NextToken"));
            assert_eq!(pag.output_token.as_deref(), Some(output_token));
            assert_eq!(pag.max_results_param.as_deref(), Some("MaxResults"));
        }
    }

    /// The S3 config-document trio (lifecycle, replication, policy) are all
    /// per-bucket children: they carry requires_parent, scope on the bucket row's
    /// Name over the "bucket" param, and hang off s3-buckets' sub_resources.
    #[test]
    fn s3_config_documents_are_bucket_scoped_children() {
        let buckets = get_resource("s3-buckets").expect("s3-buckets");
        for (key, shortcut) in [
            ("s3-lifecycle-rules", "l"),
            ("s3-replication-rules", "r"),
            ("s3-bucket-policy", "p"),
        ] {
            assert!(
                buckets
                    .sub_resources
                    .iter()
                    .any(|sr| sr.resource_key == key),
                "s3-buckets must declare {} as a sub_resource",
                key
            );
            let child = get_resource(key).unwrap_or_else(|| panic!("{}", key));
            assert!(child.requires_parent, "{key} cannot be listed standalone");
            let sub = buckets
                .sub_resources
                .iter()
                .find(|sr| sr.resource_key == key)
                .expect("declared above");
            assert_eq!(sub.shortcut, shortcut);
            assert_eq!(sub.parent_id_field, "Name");
            assert_eq!(sub.filter_param, "bucket");
        }
    }

    /// GetBucketPolicy returns the raw JSON policy document as the body, not an
    /// XML wrapper -- so unlike every other S3 resource it must go through the
    /// rest-json handler (which JSON-parses the body), and each Statement row is
    /// a list item. Lifecycle and replication are ordinary XML lists.
    #[test]
    fn s3_bucket_policy_is_rest_json_while_rules_stay_rest_xml() {
        let policy = get_resource("s3-bucket-policy").expect("s3-bucket-policy");
        let api = policy
            .api_config
            .as_ref()
            .expect("s3-bucket-policy api_config");
        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::RestJson,
            "GetBucketPolicy answers raw JSON, so xml_to_json would corrupt it"
        );
        assert_eq!(api.method.as_deref(), Some("GET"));
        assert_eq!(api.path.as_deref(), Some("/{bucket}?policy"));
        assert_eq!(api.response_root.as_deref(), Some("/Statement"));
        for case in [
            (
                "s3-lifecycle-rules",
                "/LifecycleConfiguration/Rule",
                "/{bucket}?lifecycle",
                crate::resource::protocol::ApiProtocol::RestXml,
            ),
            (
                "s3-replication-rules",
                "/ReplicationConfiguration/Rule",
                "/{bucket}?replication",
                crate::resource::protocol::ApiProtocol::RestXml,
            ),
        ] {
            let s = get_resource(case.0).unwrap_or_else(|| panic!("{}", case.0));
            let api = s
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{} api_config", case.0));
            assert_eq!(api.protocol, case.3, "{} must stay rest-xml", case.0);
            assert_eq!(api.response_root.as_deref(), Some(case.1));
            assert_eq!(api.path.as_deref(), Some(case.2));
        }
    }

    /// Kinesis ListStreams pages with a real NextToken (modern API model) and
    /// returns StreamSummaries. Firehose paginates on the last-returned-name
    /// marker instead, which a path extractor cannot derive, so it sends
    /// Limit=100 without a token loop. Pin both targets and roots.
    #[test]
    fn kinesis_and_firehose_stream_lists_hit_their_json_targets() {
        let k = get_resource("kinesis-streams").expect("kinesis-streams");
        let api = k.api_config.as_ref().expect("kinesis-streams api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.action.as_deref(), Some("ListStreams"));
        assert_eq!(api.response_root.as_deref(), Some("/StreamSummaries"));
        let pag = api.pagination.as_ref().expect("kinesis paginates");
        assert_eq!(pag.input_token.as_deref(), Some("NextToken"));
        assert_eq!(pag.max_results_param.as_deref(), Some("Limit"));
        let service = crate::aws::http::get_service("kinesis")
            .unwrap_or_else(|| panic!("kinesis service must be registered"));
        assert_eq!(service.target_prefix, Some("Kinesis_20131202"));

        let f = get_resource("firehose-delivery-streams").expect("firehose-delivery-streams");
        let api = f
            .api_config
            .as_ref()
            .expect("firehose-delivery-streams api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.response_root.as_deref(), Some("/DeliveryStreamNames"));
        let stream_name = f
            .field_mappings
            .get("DeliveryStreamName")
            .expect("firehose maps the bare stream name");
        assert!(
            stream_name.source.is_empty() || stream_name.source == "/",
            "firehose names are bare strings mapped through the item itself"
        );
        let service = crate::aws::http::get_service("firehose")
            .unwrap_or_else(|| panic!("firehose service must be registered"));
        assert_eq!(service.target_prefix, Some("Firehose_20150804"));
    }

    /// AppSync and Amazon MQ are both rest-json GETs with a JSON response root.
    /// Pin the methods, paths, roots and the token param casing — MQ's wire is
    /// camelCase like AppSync (brokerSummaries/nextToken/maxResults), and a
    /// swapped case silently never pages.
    #[test]
    fn appsync_and_mq_list_api_lists_via_rest_json_get() {
        let a = get_resource("appsync-apis").expect("appsync-apis");
        let api = a.api_config.as_ref().expect("appsync-apis api_config");
        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(api.method.as_deref(), Some("GET"));
        assert_eq!(api.path.as_deref(), Some("/v1/apis"));
        assert_eq!(api.response_root.as_deref(), Some("/graphqlApis"));
        let pag = api.pagination.as_ref().expect("appsync paginates");
        assert_eq!(pag.input_token.as_deref(), Some("nextToken"));
        assert_eq!(pag.max_results_param.as_deref(), Some("maxResults"));
        crate::aws::http::get_service("appsync")
            .unwrap_or_else(|| panic!("appsync service must be registered"));

        let m = get_resource("mq-brokers").expect("mq-brokers");
        let api = m.api_config.as_ref().expect("mq-brokers api_config");
        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(api.method.as_deref(), Some("GET"));
        assert_eq!(api.path.as_deref(), Some("/v1/brokers"));
        assert_eq!(api.response_root.as_deref(), Some("/brokerSummaries"));
        let pag = api.pagination.as_ref().expect("mq paginates");
        assert_eq!(pag.input_token.as_deref(), Some("nextToken"));
        assert_eq!(pag.max_results_param.as_deref(), Some("maxResults"));
        crate::aws::http::get_service("mq")
            .unwrap_or_else(|| panic!("mq service must be registered"));
    }

    /// The MQ family: brokers list under /v1/brokers with the broker-id UUID as
    /// id (DescribeBroker keys on the UUID, not the name), users are a
    /// parent-scoped child whose {broker-id} path placeholder must match the
    /// sub_resource filter_param, and configurations are a standalone sibling.
    /// The broker's describe reuses the same rest-json GET against the row's
    /// id, so pressing d reaches the full broker payload.
    #[test]
    fn mq_brokers_users_and_configurations_line_up_with_the_mq_wire() {
        let brokers = get_resource("mq-brokers").expect("mq-brokers");
        assert_eq!(
            brokers.id_field, "BrokerId",
            "DescribeBroker keys on the UUID"
        );
        assert_eq!(brokers.name_field, "BrokerName");

        let users_sub = brokers
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "mq-users")
            .expect("mq-brokers must declare mq-users");
        assert_eq!(
            users_sub.parent_id_field, "BrokerId",
            "users scope on the broker-id UUID"
        );
        assert_eq!(
            users_sub.filter_param, "broker-id",
            "users filter_param must match the {{broker-id}} path placeholder"
        );

        let users = get_resource("mq-users").expect("mq-users");
        assert!(users.requires_parent, "users need a parent broker");
        assert_eq!(users.id_field, "Username");
        let users_api = users.api_config.as_ref().expect("mq-users api_config");
        assert_eq!(
            users_api.path.as_deref(),
            Some("/v1/brokers/{broker-id}/users")
        );
        assert_eq!(users_api.response_root.as_deref(), Some("/users"));

        let dc = brokers
            .describe_config
            .as_ref()
            .expect("mq-brokers must describe");
        assert_eq!(
            dc.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(dc.method.as_deref(), Some("GET"));
        assert_eq!(
            dc.path.as_deref(),
            Some("/v1/brokers/{resource_id}"),
            "describe must reach the broker by its UUID id"
        );
        assert!(
            dc.response_path.is_none(),
            "broker describe is the bare payload"
        );

        let configs = get_resource("mq-configurations").expect("mq-configurations");
        assert!(!configs.requires_parent, "configurations list standalone");
        let configs_api = configs
            .api_config
            .as_ref()
            .expect("mq-configurations api_config");
        assert_eq!(configs_api.path.as_deref(), Some("/v1/configurations"));
        assert_eq!(
            configs_api.response_root.as_deref(),
            Some("/configurations")
        );
        let pag = configs_api
            .pagination
            .as_ref()
            .expect("mq-configurations paginates");
        assert_eq!(pag.input_token.as_deref(), Some("nextToken"));
        assert_eq!(pag.max_results_param.as_deref(), Some("maxResults"));
        assert!(
            configs.field_mappings.contains_key("Revision"),
            "configurations must map the nested LatestRevision"
        );
        assert_eq!(
            configs.field_mappings.get("Revision").unwrap().source,
            "/latestRevision/revision"
        );
    }

    /// DocumentDB and Neptune are the RDS-family Query protocol and, critically,
    /// share the RDS endpoint and signing identity: both are served from
    /// rds.<region>.amazonaws.com with signing_name "rds", so the service entry
    /// must use endpoint_prefix "rds" and signing_name "rds" even though the
    /// resources are separate. Assert that an (misleading) docdb/neptune pair
    /// is not silently overridden to a docdb.<region> host that does not exist,
    /// and that the four resources agree with RDS's wire roots.
    #[test]
    fn docdb_and_neptune_reuse_the_rds_endpoint_and_wire_shape() {
        let docdb_service =
            crate::aws::http::get_service("docdb").expect("docdb service must be registered");
        assert_eq!(docdb_service.endpoint_prefix, "rds");
        assert_eq!(docdb_service.signing_name, "rds");
        let neptune_service =
            crate::aws::http::get_service("neptune").expect("neptune service must be registered");
        assert_eq!(neptune_service.endpoint_prefix, "rds");
        assert_eq!(neptune_service.signing_name, "rds");

        for (key, action, root) in [
            (
                "docdb-clusters",
                "DescribeDBClusters",
                "/DescribeDBClustersResponse/DescribeDBClustersResult/DBClusters",
            ),
            (
                "docdb-instances",
                "DescribeDBInstances",
                "/DescribeDBInstancesResponse/DescribeDBInstancesResult/DBInstances/DBInstance",
            ),
            (
                "neptune-clusters",
                "DescribeDBClusters",
                "/DescribeDBClustersResponse/DescribeDBClustersResult/DBClusters",
            ),
            (
                "neptune-instances",
                "DescribeDBInstances",
                "/DescribeDBInstancesResponse/DescribeDBInstancesResult/DBInstances/DBInstance",
            ),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs api_config"));
            assert_eq!(
                api.action.as_deref(),
                Some(action),
                "docdb/neptune must use the same action names as RDS"
            );
            assert_eq!(
                api.response_root.as_deref(),
                Some(root),
                "docdb/neptune share RDS's wire roots"
            );
            assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Query);
        }
    }

    /// A DocDB instance's describe uses the same RDS-shaped DescribeDBInstances
    /// call filtered by DBInstanceIdentifier as its list call, so pressing d on
    /// an instance fetches the full instance payload. It renders a formatted
    /// labelled view (Web-console-style configuration/network/backup fields)
    /// rather than a raw JSON dump, so every describe_field source must point
    /// at a field that actually exists in the DescribeDBInstances payload.
    #[test]
    fn docdb_instances_describe_fetches_the_full_instance_by_identifier() {
        let r = get_resource("docdb-instances").expect("docdb-instances");
        let dc = r
            .describe_config
            .as_ref()
            .expect("docdb-instances must have a describe_config");
        assert_eq!(dc.protocol, crate::resource::protocol::ApiProtocol::Query);
        assert_eq!(dc.action.as_deref(), Some("DescribeDBInstances"));
        assert_eq!(dc.id_param.as_deref(), Some("DBInstanceIdentifier"));
        assert_eq!(
            dc.response_path.as_deref(),
            Some("/DescribeDBInstancesResponse/DescribeDBInstancesResult/DBInstances/DBInstance")
        );

        let labels: Vec<&str> = dc
            .describe_fields
            .iter()
            .map(|f| f.label.as_str())
            .collect();
        for expected in [
            "Instance ID",
            "Status",
            "Class",
            "Engine",
            "Engine Version",
            "Endpoint",
            "Port",
            "Region & AZ",
            "VPC",
            "Security Groups",
            "Storage Encrypted",
            "Backup Retention",
            "Maintenance Window",
            "CloudWatch Logs",
        ] {
            assert!(
                labels.contains(&expected),
                "missing describe field {expected}"
            );
        }
        let endpoint = dc
            .describe_fields
            .iter()
            .find(|f| f.source == "/Endpoint/Port")
            .expect("port is nested under Endpoint");
        assert_eq!(endpoint.label, "Port");
    }

    /// The instance detail mirrors the console's tabs, and a section heading is
    /// emitted only when the section *changes*, so a field filed under a section
    /// its neighbours do not share reprints that heading further down the panel.
    /// Pin that every section's fields are contiguous, that the console's tabs
    /// are all covered, and that the live metrics carry the transform that turns
    /// a CloudWatch datapoint array into one reading (without it the panel shows
    /// a raw JSON array).
    #[test]
    fn docdb_instance_detail_groups_the_console_sections_contiguously() {
        let r = get_resource("docdb-instances").expect("docdb-instances");
        let dc = r.describe_config.as_ref().expect("describe_config");

        let mut order: Vec<&str> = Vec::new();
        for field in &dc.describe_fields {
            let section = field
                .section
                .as_deref()
                .unwrap_or_else(|| panic!("describe field {} needs a section", field.label));
            if order.last() != Some(&section) {
                assert!(
                    !order.contains(&section),
                    "section {section} is split apart; its heading would print twice"
                );
                order.push(section);
            }
        }
        for expected in [
            "Summary",
            "Connectivity & security",
            "Security group rules",
            "Configuration",
            "Maintenance & backups",
            "Replication",
            "Tags",
            "Events (7 days)",
        ] {
            assert!(order.contains(&expected), "missing section {expected}");
        }

        for (source, metric) in [
            ("/CpuDatapoints", "CPUUtilization"),
            ("/ConnectionDatapoints", "DatabaseConnections"),
            ("/MemoryDatapoints", "FreeableMemory"),
        ] {
            let field = dc
                .describe_fields
                .iter()
                .find(|f| f.source == source)
                .unwrap_or_else(|| panic!("{metric} needs a describe field"));
            assert_eq!(
                field.transform.as_deref(),
                Some("cloudwatch_latest"),
                "{metric} is a datapoint array, not a scalar"
            );
        }
    }

    /// A DocDB instance's rules and metrics live in other services, so its
    /// enrich calls override service and protocol. Every override must resolve
    /// in the service table (an unknown key only fails at request time, in the
    /// user's face) and carry the params its protocol needs: query/json need an
    /// action, json needs a body, rest needs a path. Also assert the EC2 rule
    /// lookup stays filtered -- unfiltered it would return every rule in the
    /// account.
    #[test]
    fn enrich_calls_declare_a_reachable_service_and_their_protocols_params() {
        use crate::resource::protocol::ApiProtocol;
        let mut checked = 0;
        for key in get_all_resource_keys() {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let Some(dc) = r.describe_config.as_ref() else {
                continue;
            };
            for enrich in &dc.enrich_calls {
                checked += 1;
                let what = format!("{key} enrich {}", enrich.result_field);
                if let Some(service) = enrich.service.as_deref() {
                    assert!(
                        crate::aws::http::get_service(service).is_some(),
                        "{what} targets unregistered service {service}"
                    );
                }
                match enrich.protocol.unwrap_or(dc.protocol) {
                    ApiProtocol::Query => {
                        assert!(enrich.action.is_some(), "{what} needs an action")
                    }
                    ApiProtocol::Json => {
                        assert!(
                            enrich.action.is_some() || enrich.target.is_some(),
                            "{what} needs an action or target"
                        );
                        assert!(
                            enrich.body.is_some() || enrich.body_template.is_some(),
                            "{what} needs a json body or body_template"
                        );
                    }
                    ApiProtocol::RestJson | ApiProtocol::RestXml => {
                        assert!(enrich.path.is_some(), "{what} needs a path")
                    }
                }
                for filter in &enrich.filters {
                    assert!(
                        filter.values_source.starts_with('/'),
                        "{what} filter {} must read the describe response",
                        filter.name
                    );
                }
            }
        }
        assert!(checked > 0, "no enrich calls found -- the sweep is vacuous");

        let dc = get_resource("docdb-instances")
            .expect("docdb-instances")
            .describe_config
            .clone()
            .expect("describe_config");
        let rules = dc
            .enrich_calls
            .iter()
            .find(|e| e.result_field == "SecurityGroupRules")
            .expect("security group rules enrichment");
        assert_eq!(rules.service.as_deref(), Some("ec2"));
        assert_eq!(rules.action.as_deref(), Some("DescribeSecurityGroupRules"));
        assert_eq!(
            rules.filters.first().map(|f| f.name.as_str()),
            Some("group-id"),
            "an unfiltered rule lookup returns the whole account"
        );
    }

    /// A subnet's detail view pulls five things the row does not carry -- its
    /// flow logs, route table, network ACL and both CIDR reservation lists --
    /// each an extra EC2 call. Every one must be scoped to this subnet: the
    /// resource-driven calls filter on `resource-id`, the association ones on
    /// `association.subnet-id`, and the reservations call takes the scalar
    /// SubnetId, because GetSubnetCidrReservations has no filter form for
    /// subnet-id beyond the bare parameter. An unscoped route table lookup
    /// returns every table in the account and presents it as this subnet's.
    #[test]
    fn subnet_detail_pulls_related_resources_scoped_to_the_subnet() {
        let dc = get_resource("subnets")
            .expect("subnets")
            .describe_config
            .as_ref()
            .expect("subnets describe_config");
        assert_eq!(dc.action.as_deref(), Some("DescribeSubnets"));
        assert_eq!(dc.id_param.as_deref(), Some("SubnetId.1"));
        assert_eq!(
            dc.response_path.as_deref(),
            Some("/DescribeSubnetsResponse/subnetSet/item")
        );

        let flow = dc
            .enrich_calls
            .iter()
            .find(|e| e.result_field == "FlowLogs")
            .expect("flow logs enrichment");
        assert_eq!(flow.action.as_deref(), Some("DescribeFlowLogs"));
        assert_eq!(flow.filters[0].name, "resource-id");
        assert_eq!(flow.filters[0].values_source, "/subnetId");

        for (field, filter) in [
            ("RouteTable", "DescribeRouteTables"),
            ("NetworkAcl", "DescribeNetworkAcls"),
        ] {
            let enrich = dc
                .enrich_calls
                .iter()
                .find(|e| e.result_field == field)
                .unwrap_or_else(|| panic!("missing {field} enrichment"));
            assert_eq!(enrich.action.as_deref(), Some(filter), "{field} action");
            assert_eq!(
                enrich.filters[0].name, "association.subnet-id",
                "{field} must filter on the subnet association"
            );
            assert_eq!(enrich.filters[0].values_source, "/subnetId");
        }

        for (field, extract) in [
            (
                "CidrReservationsV4",
                "/GetSubnetCidrReservationsResponse/subnetIpv4CidrReservationSet/item",
            ),
            (
                "CidrReservationsV6",
                "/GetSubnetCidrReservationsResponse/subnetIpv6CidrReservationSet/item",
            ),
        ] {
            let enrich = dc
                .enrich_calls
                .iter()
                .find(|e| e.result_field == field)
                .unwrap_or_else(|| panic!("missing {field} enrichment"));
            assert_eq!(enrich.action.as_deref(), Some("GetSubnetCidrReservations"));
            assert_eq!(enrich.extract_path.as_deref(), Some(extract));
            assert_eq!(
                enrich.params.get("SubnetId").map(String::as_str),
                Some("{resource_id}"),
                "GetSubnetCidrReservations has no filter for subnet-id; it needs the bare param"
            );
        }
        for enrich in &dc.enrich_calls {
            assert!(
                enrich
                    .extract_path
                    .as_deref()
                    .unwrap_or("")
                    .ends_with("/item"),
                "subnets enrich {} must read the item list root under its response wrapper",
                enrich.result_field
            );
        }
    }

    /// VPCs, VPC endpoints and peering connections each got the console-style
    /// detail view. The VPC and endpoint describes enrich their attached
    /// resources, and every such call must be scoped to the row -- leaving the
    /// VPC's internet-gateway lookup unfiltered would list every gateway in the
    /// account as this VPC's own. The peering detail needs no enrichment: its
    /// request/response call already returns the whole requester+accepter VPC
    /// pair inline. Pin the describe entry points so a drift in the query
    /// protocol's indexed id params (VpcId.1, not VpcId) is caught.
    #[test]
    fn vpc_networking_detail_views_are_scoped_and_use_indexed_id_params() {
        let cases: Vec<(&str, &str, &str)> = vec![
            ("vpc", "DescribeVpcs", "/DescribeVpcsResponse/vpcSet/item"),
            (
                "vpc-endpoints",
                "DescribeVpcEndpoints",
                "/DescribeVpcEndpointsResponse/vpcEndpointSet/item",
            ),
            (
                "vpc-peering-connections",
                "DescribeVpcPeeringConnections",
                "/DescribeVpcPeeringConnectionsResponse/vpcPeeringConnectionSet/item",
            ),
        ];
        for (key, action, root) in &cases {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let dc = r
                .describe_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs describe_config"));
            assert_eq!(dc.action.as_deref(), Some(*action), "{key} describe action");
            assert_eq!(
                dc.response_path.as_deref(),
                Some(*root),
                "{key} describe response root"
            );
        }
        for (key, id_param) in [
            ("vpc", "VpcId.1"),
            ("vpc-endpoints", "VpcEndpointId.1"),
            ("vpc-peering-connections", "VpcPeeringConnectionId.1"),
        ] {
            let r = get_resource(key).unwrap();
            let dc = r.describe_config.as_ref().unwrap();
            assert_eq!(
                dc.id_param.as_deref(),
                Some(id_param),
                "{key} describe id param must be the indexed list member, not the bare name"
            );
        }

        let vpc = get_resource("vpc")
            .unwrap()
            .describe_config
            .clone()
            .unwrap();
        let igw = vpc
            .enrich_calls
            .iter()
            .find(|e| e.result_field == "InternetGateways")
            .expect("vpc internet gateway enrichment");
        assert_eq!(igw.filters[0].name, "attachment.vpc-id");
        assert_eq!(igw.filters[0].values_source, "/vpcId");

        let qe = get_resource("vpc-endpoints")
            .unwrap()
            .describe_config
            .clone()
            .unwrap();
        let eni = qe
            .enrich_calls
            .iter()
            .find(|e| e.result_field == "NetworkInterfaces")
            .expect("endpoint network interface enrichment");
        assert_eq!(eni.filters[0].name, "network-interface-id");
        assert_eq!(eni.filters[0].values_source, "/networkInterfaceIdSet/item");
    }

    /// ACM certificates keep their DNS validation records, not on the list row
    /// but only in DescribeCertificate, where they are the interesting part.
    /// The ask is "all the domains, their records": the describe must hold the
    /// DomainValidationOptions as a rendered list whose item template walks into
    /// each ResourceRecord, and it must carry the ISO names the console shows.
    #[test]
    fn acm_certificate_describe_surfaces_each_domain_validation_record() {
        let r = get_resource("acm-certificates")
            .unwrap_or_else(|| panic!("acm-certificates must parse"));
        let dc = r.describe_config.as_ref().expect("needs describe_config");

        assert_eq!(dc.action.as_deref(), Some("DescribeCertificate"));
        assert_eq!(dc.id_param.as_deref(), Some("CertificateArn"));
        assert_eq!(dc.response_path.as_deref(), Some("/Certificate"));

        let validation = dc
            .describe_fields
            .iter()
            .find(|f| f.source == "/DomainValidationOptions")
            .expect("domain validation list field");
        assert!(
            validation.list,
            "DomainValidationOptions must render as a list"
        );
        let tmpl = validation.item_template.as_deref().unwrap();
        for needle in [
            "{ResourceRecord/Name}",
            "{ResourceRecord/Type}",
            "{ResourceRecord/Value}",
        ] {
            assert!(
                tmpl.contains(needle),
                "item template must surface the DNS record {needle}: {tmpl}"
            );
        }

        let names: Vec<&str> = dc
            .describe_fields
            .iter()
            .map(|f| f.source.as_str())
            .collect();
        for want in ["/SubjectAlternativeNames", "/InUseBy", "/KeyUsages"] {
            assert!(
                names.contains(&want),
                "{want} should be in the ACM detail view"
            );
        }

        for ts in ["/CreatedAt", "/IssuedAt", "/NotBefore", "/NotAfter"] {
            let f = dc
                .describe_fields
                .iter()
                .find(|f| f.source == ts)
                .unwrap_or_else(|| panic!("{ts} describe field"));
            assert_eq!(
                f.transform.as_deref(),
                Some("format_epoch_seconds"),
                "{ts} is epoch-seconds on the ACM wire"
            );
        }

        let tags = dc.enrich_calls.iter().find(|e| e.result_field == "Tags");
        let tags = tags.expect("tags enrichment");
        assert_eq!(
            tags.action.as_deref(),
            Some("ListTagsForCertificate"),
            "certificates use the certificate-scoped tag API, not ListTagsForResource"
        );
        assert_eq!(tags.extract_path.as_deref(), Some("/Tags"));
    }

    /// The ACM list replaced its four columns with the full console set, and
    /// its id column is the one thing that can never be turned off: describe
    /// keys off CertificateArn, so hiding it would strand a row with no id.
    /// Pin the column set and that exactly one column is locked to the id.
    #[test]
    fn acm_list_columns_follow_the_console_and_lock_the_id() {
        let r = get_resource("acm-certificates").expect("acm-certificates");
        let headers: Vec<&str> = r.columns.iter().map(|c| c.header.as_str()).collect();
        for want in [
            "CERTIFICATE ID",
            "COMMON NAME",
            "TYPE",
            "STATUS",
            "IN USE",
            "RENEWAL ELIGIBILITY",
            "KEY ALGORITHM",
            "DNS NAMES",
            "KEY USAGE NAME",
            "EXT KEY USAGE NAME",
            "CREATED AT",
            "ISSUED AT",
            "NOT BEFORE",
            "NOT AFTER",
            "REVOKED AT",
            "MANAGED BY",
            "IMPORTED AT",
            "IS EXPORTED",
            "EXPORT OPTION",
            "KEY SOURCE",
        ] {
            assert!(headers.contains(&want), "ACM should show column {want}");
        }

        let locked: Vec<&str> = r
            .columns
            .iter()
            .filter(|c| c.locked)
            .map(|c| c.header.as_str())
            .collect();
        assert_eq!(
            locked,
            vec!["CERTIFICATE ID"],
            "the cert id column is the only non-hideable ACM column"
        );

        let id_col = r.columns.iter().find(|c| c.locked).unwrap();
        assert_eq!(id_col.json_path, "CertificateArn");

        for col in &r.columns {
            let root = col.json_path.split('.').next().unwrap_or("");
            assert!(
                r.field_mappings.contains_key(root),
                "ACM column {} json_path root {} is not in field_mappings",
                col.header,
                root
            );
        }
    }

    /// Secrets Manager list mirrors the console: the full set of columns, but
    /// only a console-style handful visible by default (the rest opt-in via p).
    /// Secret Name is locked because describe and every action key off the ARN
    /// shown in that row, and hiding it would strand a row with no id.
    #[test]
    fn secretsmanager_list_follows_the_console_and_locks_the_name() {
        let r = get_resource("secretsmanager-secrets").expect("secretsmanager-secrets");
        let headers: Vec<&str> = r.columns.iter().map(|c| c.header.as_str()).collect();
        let default_visible: Vec<&str> = r
            .columns
            .iter()
            .filter(|c| c.visible)
            .map(|c| c.header.as_str())
            .collect();
        assert_eq!(
            default_visible,
            vec!["SECRET NAME", "DESCRIPTION", "LAST RETRIEVED", "CREATED ON"],
            "the console shows one leaf (name) plus a handful of columns by default"
        );

        for want in [
            "DELETED ON",
            "LAST UPDATED",
            "LAST ROTATED",
            "NEXT ROTATION",
            "MANAGED BY",
            "TYPE",
        ] {
            assert!(
                headers.contains(&want),
                "Secrets Manager should carry a {want} column (opt-in via p)"
            );
        }

        let locked: Vec<&str> = r
            .columns
            .iter()
            .filter(|c| c.locked)
            .map(|c| c.header.as_str())
            .collect();
        assert_eq!(
            locked,
            vec!["SECRET NAME"],
            "the secret name column is the only non-hideable Secrets Manager column"
        );
        let name_col = r.columns.iter().find(|c| c.locked).unwrap();
        assert_eq!(name_col.json_path, "Name");

        for col in &r.columns {
            let root = col.json_path.split('.').next().unwrap_or("");
            assert!(
                r.field_mappings.contains_key(root),
                "Secrets Manager column {} json_path root {} is not in field_mappings",
                col.header,
                root
            );
        }
    }

    /// Secrets Manager describe is a plain DescribeSecret JSON call (the secret
    /// object is returned at top level, not wrapped), and its date fields are
    /// epoch-seconds floats on the wire -- the FormatDuration-free transform the
    /// Health and ACM resources established.
    #[test]
    fn secretsmanager_describe_renders_console_sections_with_epoch_second_dates() {
        let r = get_resource("secretsmanager-secrets").expect("secretsmanager-secrets");
        let dc = r.describe_config.as_ref().expect("needs describe_config");
        assert_eq!(dc.action.as_deref(), Some("DescribeSecret"));
        assert!(dc.response_path.is_none(), "secret object is at top level");

        let names: Vec<&str> = dc
            .describe_fields
            .iter()
            .map(|f| f.source.as_str())
            .collect();
        for want in ["/Name", "/ARN", "/KmsKeyId", "/PrimaryRegion"] {
            assert!(
                names.contains(&want),
                "{want} should be in the Secrets Manager detail view"
            );
        }

        for ts in ["/CreatedDate", "/LastAccessedDate", "/LastChangedDate"] {
            let f = dc
                .describe_fields
                .iter()
                .find(|f| f.source == ts)
                .unwrap_or_else(|| panic!("{ts} describe field"));
            assert_eq!(
                f.transform.as_deref(),
                Some("format_epoch_seconds"),
                "{ts} is epoch-seconds on the Secrets Manager wire"
            );
        }

        let tags = dc.describe_fields.iter().find(|f| f.source == "/Tags");
        let tags = tags.expect("tags list field");
        assert!(tags.list, "/Tags must render as a list");
        assert_eq!(
            tags.item_template.as_deref(),
            Some("{Key}: {Value}"),
            "tags render as Key: Value"
        );
    }

    /// An enrichment costs an extra API call per describe, so one whose result
    /// no describe field reads is pure latency the user pays for nothing -- and
    /// a typo between the two (Tags vs TagList) shows as a silently blank row,
    /// orbit's worst failure mode. Pin that every result_field is displayed.
    #[test]
    fn every_enrich_result_is_read_by_a_describe_field() {
        for key in get_all_resource_keys() {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let Some(dc) = r.describe_config.as_ref() else {
                continue;
            };
            if dc.describe_fields.is_empty() {
                continue; // raw JSON dump shows every enrichment by definition
            }
            for enrich in &dc.enrich_calls {
                let prefix = format!("/{}", enrich.result_field);
                let shown = dc
                    .describe_fields
                    .iter()
                    .any(|f| f.source == prefix || f.source.starts_with(&format!("{prefix}/")))
                    || dc.overview.as_ref().is_some_and(|o| {
                        o.resources.iter().any(|r| {
                            r.source == prefix || r.source.starts_with(&format!("{prefix}/"))
                        })
                    });
                assert!(
                    shown,
                    "{key} fetches {} but no describe field shows it",
                    enrich.result_field
                );
            }
        }
    }

    /// Glue and EMR are JSON protocol services whose targets and pagination
    /// shapes differ. Glue pages every list on NextToken/MaxResults with rich
    /// summary shapes. EMR pages on a bare Marker token and, unlike most json
    /// APIs, accepts NO MaxResults parameter at all -- sending one is the
    /// DescribeAddresses failure mode (parameter not supported, fails silently
    /// to a single page). Pin the roots and that EMR omits max_results_param.
    #[test]
    fn glue_and_emr_lists_page_with_their_distinct_json_tokens() {
        for (key, action, root, target, pag_in, pag_max) in [
            (
                "glue-jobs",
                "GetJobs",
                "/Jobs",
                "AWSGlue",
                Some("NextToken"),
                Some("MaxResults"),
            ),
            (
                "glue-databases",
                "GetDatabases",
                "/DatabaseList",
                "AWSGlue",
                Some("NextToken"),
                Some("MaxResults"),
            ),
            (
                "glue-crawlers",
                "GetCrawlers",
                "/Crawlers",
                "AWSGlue",
                Some("NextToken"),
                Some("MaxResults"),
            ),
            (
                "glue-triggers",
                "GetTriggers",
                "/Triggers",
                "AWSGlue",
                Some("NextToken"),
                Some("MaxResults"),
            ),
            (
                "emr-clusters",
                "ListClusters",
                "/Clusters",
                "ElasticMapReduce",
                Some("Marker"),
                None,
            ),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs api_config"));
            assert_eq!(api.action.as_deref(), Some(action), "{key} action");
            assert_eq!(api.response_root.as_deref(), Some(root), "{key} root");
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} paginates"));
            assert_eq!(pag.input_token.as_deref(), pag_in, "{key} input token");
            assert_eq!(
                pag.max_results_param.as_deref(),
                pag_max,
                "{key} max-results param"
            );
            let svc_key = if key.starts_with("glue-") {
                "glue"
            } else {
                "emr"
            };
            let svc = crate::aws::http::get_service(svc_key)
                .unwrap_or_else(|| panic!("{svc_key} service must be registered"));
            assert_eq!(svc.target_prefix, Some(target), "{key} target prefix");
        }
    }

    /// DataSync and Transfer are plain JSON protocol lists with their own
    /// X-Amz-Target prefixes and camelCase roots. Transfer's users are scoped
    /// per-server via a ServerId parent filter and require a parent. Pin the
    /// targets and roots so a missing include_str or a typoed root lists empty.
    #[test]
    fn datasync_and_transfer_lists_and_parent_scope() {
        for (key, action, root, target) in [
            ("datasync-tasks", "ListTasks", "/Tasks", "FmrsService"),
            (
                "transfer-servers",
                "ListServers",
                "/Servers",
                "TransferService",
            ),
            ("transfer-users", "ListUsers", "/Users", "TransferService"),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs api_config"));
            assert_eq!(api.action.as_deref(), Some(action), "{key} action");
            assert_eq!(api.response_root.as_deref(), Some(root), "{key} root");
            let svc = crate::aws::http::get_service(if key.starts_with("datasync-") {
                "datasync"
            } else {
                "transfer"
            })
            .unwrap_or_else(|| {
                panic!(
                    "{} service must be registered",
                    if key.starts_with("datasync-") {
                        "datasync"
                    } else {
                        "transfer"
                    }
                )
            });
            assert_eq!(svc.target_prefix, Some(target), "{key} target prefix");
        }

        let users = get_resource("transfer-users").expect("transfer-users");
        assert!(
            users.requires_parent,
            "transfer-users must require a parent server"
        );
        let servers = get_resource("transfer-servers").expect("transfer-servers");
        let drill = servers
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "transfer-users")
            .expect("servers drill into users");
        assert_eq!(drill.filter_param, "ServerId", "users scope by ServerId");
        assert_eq!(
            servers.enter_sub_resource.as_deref(),
            Some("transfer-users"),
            "Enter drills into users"
        );
    }

    /// Mesh/networking lists: App Mesh and VPC Lattice are rest-json GETs with
    /// lowercase camelCase roots and tokens (/meshes, /items); Cloud Map and
    /// Route53 Resolver are JSON protocol with their own X-Amz-Target prefixes
    /// and PascalCase roots. Pin each so a misspelled root or a wrong
    /// protocol silently lists empty.
    #[test]
    fn mesh_cloud_map_lattice_and_resolver_lists_keep_their_wire_shapes() {
        for (key, target, root, pag_in) in [
            ("appmesh-meshes", None, "/meshes", Some("nextToken")),
            (
                "cloudmap-services",
                Some("Route53AutoNaming_v20170314"),
                "/Services",
                Some("NextToken"),
            ),
            (
                "cloudmap-namespaces",
                Some("Route53AutoNaming_v20170314"),
                "/Namespaces",
                Some("NextToken"),
            ),
            ("vpclattice-services", None, "/items", Some("nextToken")),
            (
                "r53resolver-endpoints",
                Some("Route53Resolver"),
                "/ResolverEndpoints",
                Some("NextToken"),
            ),
            (
                "r53resolver-rules",
                Some("Route53Resolver"),
                "/ResolverRules",
                Some("NextToken"),
            ),
            (
                "r53resolver-rule-associations",
                Some("Route53Resolver"),
                "/ResolverRuleAssociations",
                Some("NextToken"),
            ),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs api_config"));
            assert_eq!(api.response_root.as_deref(), Some(root), "{key} root");
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} paginates"));
            assert_eq!(pag.input_token.as_deref(), pag_in, "{key} input token");
            let svc_key = match key {
                "appmesh-meshes" => "appmesh",
                "cloudmap-services" | "cloudmap-namespaces" => "servicediscovery",
                "vpclattice-services" => "vpc-lattice",
                _ => "route53resolver",
            };
            let service = crate::aws::http::get_service(svc_key)
                .unwrap_or_else(|| panic!("{svc_key} service must be registered"));
            assert_eq!(service.target_prefix, target, "{key} target prefix");
            let expected_protocol = if key.starts_with("appmesh-") || key.starts_with("vpclattice-")
            {
                crate::resource::protocol::ApiProtocol::RestJson
            } else {
                crate::resource::protocol::ApiProtocol::Json
            };
            assert_eq!(api.protocol, expected_protocol, "{key} protocol");
        }
    }

    /// Backup and Resource Groups are rest-json; Service Quotas is plain JSON.
    /// The interesting bit is Resource Groups' list-call quirk: ListGroups is
    /// a POST whose MaxResults/NextToken live in the *query string*, which the
    /// rest-json handler cannot express (it pages POSTs in the body), so the
    /// resource deliberately omits pagination rather than send params AWS
    /// would silently ignore -- it falls back to AWS's default one page.
    /// Pin the three roots and the no-pagination choice for resource-groups.
    #[test]
    fn backup_resource_groups_and_quotas_keep_their_endpoint_protocols() {
        for (key, root, protocol, has_paging) in [
            (
                "backup-vaults",
                "/BackupVaultList",
                crate::resource::protocol::ApiProtocol::RestJson,
                true,
            ),
            (
                "backup-plans",
                "/BackupPlansList",
                crate::resource::protocol::ApiProtocol::RestJson,
                true,
            ),
            (
                "resource-groups",
                "/GroupIdentifiers",
                crate::resource::protocol::ApiProtocol::RestJson,
                false,
            ),
            (
                "service-quotas",
                "/Services",
                crate::resource::protocol::ApiProtocol::Json,
                true,
            ),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs api_config"));
            assert_eq!(api.protocol, protocol, "{key} protocol");
            assert_eq!(api.response_root.as_deref(), Some(root), "{key} root");
            assert_eq!(
                api.pagination.is_some(),
                has_paging,
                "{key} pagination state"
            );
        }
        let svc = crate::aws::http::get_service("servicequotas")
            .unwrap_or_else(|| panic!("servicequotas service must be registered"));
        assert_eq!(svc.target_prefix, Some("ServiceQuotasV20190624"));
        crate::aws::http::get_service("backup")
            .unwrap_or_else(|| panic!("backup service must be registered"));
        crate::aws::http::get_service("resource-groups")
            .unwrap_or_else(|| panic!("resource-groups service must be registered"));
    }

    /// Trusted Advisor is a global service (trustedadvisor.amazonaws.com, no
    /// region in the host) served by a rest-json GET; Health is a regional
    /// JSON protocol POST. Pin the TA global flip and both roots so a wrong
    /// regional/global choice doesn't silently resolve to a dead host.
    #[test]
    fn trusted_advisor_is_global_and_health_is_regional() {
        let ta = get_resource("trusted-advisor-checks").expect("trusted-advisor-checks");
        let api = ta
            .api_config
            .as_ref()
            .expect("trusted-advisor-checks api_config");
        assert_eq!(
            api.protocol,
            crate::resource::protocol::ApiProtocol::RestJson
        );
        assert_eq!(api.response_root.as_deref(), Some("/checkSummaries"));
        assert!(ta.is_global, "trusted-advisor-checks must be marked global");
        let tasvc = crate::aws::http::get_service("trustedadvisor")
            .unwrap_or_else(|| panic!("trustedadvisor service must be registered"));
        assert!(tasvc.is_global, "trustedadvisor is a global service");

        let h = get_resource("health-events").expect("health-events");
        let api = h.api_config.as_ref().expect("health-events api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.response_root.as_deref(), Some("/events"));
        let hsvc = crate::aws::http::get_service("health")
            .unwrap_or_else(|| panic!("health service must be registered"));
        assert_eq!(hsvc.target_prefix, Some("AWSHealth_20160804"));
        assert!(
            !hsvc.is_global,
            "health stays regional in the selected region"
        );

        let start_time = h
            .field_mappings
            .get("startTime")
            .expect("health-events startTime mapping");
        assert_eq!(
            start_time.transform.as_deref(),
            Some("format_epoch_seconds"),
            "health events ship startTime as thread epoch seconds on the json wire; \
             formatting it is what makes the START column readable"
        );

        let status_col = h
            .columns
            .iter()
            .find(|c| c.json_path == "statusCode")
            .expect("health-events STATUS column");
        assert_eq!(
            status_col.color_map.as_deref(),
            Some("health_status"),
            "the STATUS column needs its own map because the shared state map does \
             not know open/closed/upcoming"
        );
        assert_eq!(
            get_color_for_value("health_status", "upcoming"),
            Some([255, 128, 0]),
            "upcoming must read orange"
        );
        assert_eq!(
            get_color_for_value("health_status", "open"),
            Some([255, 0, 0]),
            "open must read red"
        );
        assert_eq!(
            get_color_for_value("health_status", "closed"),
            Some([0, 255, 0]),
            "closed must read green"
        );

        // DescribeEventDetails keys off the ARN, not the event type code, and
        // takes the ARNs in a list -- the same list-in-body shape ECS clusters
        // use for DescribeClusters. The id field therefore has to be the ARN
        // or d describes an empty string.
        assert_eq!(h.id_field, "arn", "health-events id field drives describe");
        let dc = h
            .describe_config
            .as_ref()
            .expect("health-events describe_config");
        assert_eq!(dc.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(dc.action.as_deref(), Some("DescribeEventDetails"));
        assert_eq!(
            dc.body_template.as_deref(),
            Some(r#"{"eventArns": ["{resource_id}"]}"#),
            "eventArns is a list even for a single event"
        );
        assert_eq!(dc.response_path.as_deref(), Some("/successfulSet/0"));
        let description = dc
            .describe_fields
            .iter()
            .find(|f| f.label == "Description")
            .expect("describe must show the event description");
        assert_eq!(
            description.source, "/eventDescription/latestDescription",
            "the readable prose lives in latestDescription, not in the summary event"
        );
        assert!(
            dc.describe_fields
                .iter()
                .any(|f| f.source == "/event/startTime"
                    && f.transform.as_deref() == Some("format_epoch_seconds")),
            "describe timestamps need the same epoch formatting as the START column"
        );
    }

    /// Config rules and the two new SSM lists all live behind the JSON protocol
    /// with their own targets. Config's DescribeConfigRules is the trap: it
    /// pages on NextToken only and takes NO MaxResults parameter, so the block
    /// omits max_results_param (the DescribeAddresses/EMR failure mode). SSM
    /// documents and managed instances page normally. Pin roots, targets, and
    /// the Config no-max-choice.
    #[test]
    fn config_rules_and_ssm_lists_keep_their_targets_and_paging() {
        let c = get_resource("config-rules").expect("config-rules");
        let api = c.api_config.as_ref().expect("config-rules api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.response_root.as_deref(), Some("/ConfigRules"));
        let pag = api.pagination.as_ref().expect("config rules paginate");
        assert_eq!(pag.input_token.as_deref(), Some("NextToken"));
        assert!(
            pag.max_results_param.is_none(),
            "DescribeConfigRules takes no MaxResults; a max-results param would fail silently"
        );
        let csvc = crate::aws::http::get_service("config")
            .unwrap_or_else(|| panic!("config service must be registered"));
        assert_eq!(csvc.target_prefix, Some("StarlingDoveService"));

        for (key, root) in [
            ("ssm-documents", "/DocumentIdentifiers"),
            ("ssm-managed-instances", "/InstanceInformationList"),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} needs api_config"));
            assert_eq!(api.response_root.as_deref(), Some(root), "{key} root");
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} paginates"));
            assert_eq!(
                pag.max_results_param.as_deref(),
                Some("MaxResults"),
                "{key}"
            );
        }
        let ssvc = crate::aws::http::get_service("ssm")
            .unwrap_or_else(|| panic!("ssm service must be registered"));
        assert_eq!(ssvc.target_prefix, Some("AmazonSSM"));
    }

    /// IAM Identity Center (sso-admin) is served from the sso.<region> host and
    /// signs as "sso" even though the CLI commands are sso-admin. The service
    /// entry must carry signing_name "sso" and endpoint_prefix "sso" or the
    /// SigV4 credential scope and host both break. Pin that alongside the
    /// SWBExternalService target.
    #[test]
    fn identity_center_lists_instances_via_the_sso_endpoint() {
        let r = get_resource("sso-admin-instances").expect("sso-admin-instances");
        let api = r
            .api_config
            .as_ref()
            .expect("sso-admin-instances api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.action.as_deref(), Some("ListInstances"));
        assert_eq!(api.response_root.as_deref(), Some("/Instances"));
        let svc = crate::aws::http::get_service("sso-admin")
            .unwrap_or_else(|| panic!("sso-admin service must be registered"));
        assert_eq!(svc.signing_name, "sso");
        assert_eq!(svc.endpoint_prefix, "sso");
        assert_eq!(svc.target_prefix, Some("SWBExternalService"));
    }

    /// The role's formatted describe reaches GetRole by RoleName (the row's
    /// id is the RoleName, not RoleId: GetRole rejects the UUID), then enriches
    /// with the attached, inline and tag lists. The policy fields are read-only
    /// for now; the inline policies get their own `i` shortcut list.
    #[test]
    fn iam_role_describe_uses_rolename_and_enriches_policies() {
        let roles = get_resource("iam-roles").expect("iam-roles");
        assert_eq!(roles.id_field, "RoleName", "GetRole keys on RoleName");
        let dc = roles
            .describe_config
            .as_ref()
            .expect("iam-roles must have a formatted describe");
        assert_eq!(dc.action.as_deref(), Some("GetRole"));
        assert_eq!(dc.id_param.as_deref(), Some("RoleName"));

        let enrich: Vec<_> = dc
            .enrich_calls
            .iter()
            .map(|e| (e.result_field.as_str(), e.action.as_deref().unwrap_or("")))
            .collect();
        for (field, action) in [
            ("AttachedPolicies", "ListAttachedRolePolicies"),
            ("InlinePolicies", "ListRolePolicies"),
            ("Tags", "ListRoleTags"),
        ] {
            assert!(
                enrich.contains(&(field, action)),
                "iam-roles must enrich {field} via {action}"
            );
        }

        // The trust policy is URL-encoded on the wire; the describe field must
        // decode it or the panel shows a wall of %7B/%22.
        let trust = dc
            .describe_fields
            .iter()
            .find(|f| f.source == "/AssumeRolePolicyDocument")
            .expect("trust policy field");
        assert_eq!(
            trust.transform.as_deref(),
            Some("url_decode"),
            "AssumeRolePolicyDocument must be url-decoded"
        );

        let inline = get_resource("iam-role-inline-policies").expect("iam-role-inline-policies");
        assert!(inline.requires_parent, "inline policies need a parent role");
        assert_eq!(
            inline.api_config.as_ref().unwrap().response_root.as_deref(),
            Some("/ListRolePoliciesResponse/ListRolePoliciesResult/PolicyNames")
        );
        assert!(roles
            .sub_resources
            .iter()
            .any(|s| s.resource_key == "iam-role-inline-policies"));

        // The policy lists are drillable: Enter on an item fetches its
        // document (managed via ARN, inline via the bare name).
        let attached = dc
            .describe_fields
            .iter()
            .find(|f| f.source == "/AttachedPolicies")
            .expect("attached policies field");
        let attached_drill = attached.drill.as_ref().expect("attached policies drill");
        assert_eq!(
            attached_drill.kind,
            crate::resource::protocol::DrillKind::ManagedPolicyDocument
        );
        assert_eq!(
            attached_drill.item_field, "PolicyArn",
            "managed drill keys on the policy ARN"
        );
        let inline_field = dc
            .describe_fields
            .iter()
            .find(|f| f.source == "/InlinePolicies")
            .expect("inline policies field");
        let inline_drill = inline_field.drill.as_ref().expect("inline policies drill");
        assert_eq!(
            inline_drill.kind,
            crate::resource::protocol::DrillKind::InlinePolicyDocument
        );
        assert!(
            inline_drill.item_field.is_empty(),
            "inline items are bare policy names"
        );
    }

    /// Identity pools and user-pool app clients both list via the JSON protocol
    /// but from two different services: cognito-identity (AWSCognitoIdentityService)
    /// for pools, and the existing cognito-idp entry for app clients. The app
    /// client list is scoped by UserPoolId and can't run standalone, so it must
    /// be a parent-scoped sub-resource of the user pool. Pin targets, the
    /// identity root, and the parent wiring.
    #[test]
    fn cognito_identity_pools_and_app_clients_keep_their_services_and_scope() {
        let ip = get_resource("cognito-identity-pools").expect("cognito-identity-pools");
        let api = ip.api_config.as_ref().expect("identity pools api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Json);
        assert_eq!(api.response_root.as_deref(), Some("/IdentityPools"));
        let isvc = crate::aws::http::get_service("cognito-identity")
            .unwrap_or_else(|| panic!("cognito-identity service must be registered"));
        assert_eq!(isvc.signing_name, "cognito-identity");
        assert_eq!(isvc.target_prefix, Some("AWSCognitoIdentityService"));

        let up = get_resource("cognito-user-pools").expect("cognito-user-pools");
        let sub = up
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "cognito-user-pool-clients")
            .expect("user pools must drill into app clients");
        assert_eq!(sub.shortcut, "c");
        assert_eq!(sub.parent_id_field, "Id");
        assert_eq!(sub.filter_param, "UserPoolId");

        let clients = get_resource("cognito-user-pool-clients").expect("clients");
        assert!(
            clients.requires_parent,
            "app clients need a UserPoolId and can't list standalone"
        );
        let capi = clients.api_config.as_ref().expect("clients api_config");
        assert_eq!(capi.response_root.as_deref(), Some("/UserPoolClients"));
    }

    /// The three extra IAM lists are query-protocol `member` lists off the
    /// global iam endpoint. Only ListInstanceProfiles paginates (Marker) —
    /// ListSAMLProviders and ListOpenIDConnectProviders have no paginator and
    /// reject a Marker block, so they carry none (the DescribeAddresses trap).
    /// Instances profiles also parse their Roles member list down to RoleName,
    /// exercising the array_to_csv path over a nested member.
    #[test]
    fn iam_provider_and_instance_profile_lists_use_global_member_roots() {
        let profiles = get_resource("iam-instance-profiles").expect("iam-instance-profiles");
        assert!(profiles.is_global, "iam is global");
        let pup = profiles.api_config.as_ref().expect("profiles api_config");
        assert_eq!(pup.protocol, crate::resource::protocol::ApiProtocol::Query);
        assert_eq!(
            pup.response_root.as_deref(),
            Some(
                "/ListInstanceProfilesResponse/ListInstanceProfilesResult/InstanceProfiles/member"
            )
        );
        let pag = pup.pagination.as_ref().expect("instance profiles page");
        assert_eq!(pag.input_token.as_deref(), Some("Marker"));
        assert_eq!(
            profiles.field_mappings["Roles"].source,
            "/Roles/member/RoleName"
        );

        for (key, action) in [
            ("iam-saml-providers", "ListSAMLProviders"),
            ("iam-oidc-providers", "ListOpenIDConnectProviders"),
        ] {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            assert!(r.is_global, "{key} is global");
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} config"));
            assert_eq!(
                api.protocol,
                crate::resource::protocol::ApiProtocol::Query,
                "{key}"
            );
            assert!(
                api.pagination.is_none(),
                "{action} has no paginator and must not carry a Marker block"
            );
        }
    }

    /// KMS pages on Marker/NextMarker/Limit, a token trio distinct from the
    /// NextToken services, so each KMS block must use it exactly. Aliases list
    /// standalone; grants and key policies need a KeyId and can't run alone, so
    /// they are parent-scoped sub-resources of the key. Key policies are a bare
    /// string list, so their field reads the item itself (source ""), and
    /// grants flatten the Operations list.
    #[test]
    fn kms_aliases_and_key_children_page_on_marker_and_scope_to_keyid() {
        let aliases = get_resource("kms-aliases").expect("kms-aliases");
        let pag = aliases
            .api_config
            .as_ref()
            .expect("aliases api_config")
            .pagination
            .as_ref()
            .expect("aliases paginate");
        assert_eq!(pag.input_token.as_deref(), Some("Marker"));
        assert_eq!(pag.output_token.as_deref(), Some("/NextMarker"));
        assert_eq!(pag.max_results_param.as_deref(), Some("Limit"));
        assert!(!aliases.requires_parent, "aliases list standalone");

        let keys = get_resource("kms-keys").expect("kms-keys");
        for (key, target) in [
            ("kms-key-grants", "/Grants"),
            ("kms-key-policies", "/PolicyNames"),
        ] {
            let child = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            assert!(child.requires_parent, "{key} needs a KeyId parent");
            let api = child
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} config"));
            assert_eq!(api.response_root.as_deref(), Some(target), "{key} root");
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} pages"));
            assert_eq!(pag.input_token.as_deref(), Some("Marker"), "{key} token");
        }
        let policies = get_resource("kms-key-policies").expect("kms-key-policies");
        assert_eq!(
            policies.field_mappings["PolicyName"].source, "",
            "key policies are a bare string list; the field reads the item"
        );
        let grants = get_resource("kms-key-grants").expect("kms-key-grants");
        assert_eq!(grants.field_mappings["Operations"].source, "/Operations");

        let sub: Vec<_> = keys
            .sub_resources
            .iter()
            .map(|s| s.resource_key.as_str())
            .collect();
        assert!(sub.contains(&"kms-key-grants"), "keys drill grants");
        assert!(sub.contains(&"kms-key-policies"), "keys drill policies");
    }

    /// SNS subscriptions are query protocol with the Subscriptions/member root,
    /// both standalone (ListSubscriptions) and scoped to a topic
    /// (ListSubscriptionsByTopic, parent-scoped by TopicArn). SQS is the
    /// opposite trap: the current botocore model is JSON protocol signed as
    /// AmazonSQS, but the resource was wired as query with an XML root, so it
    /// silently returned no rows. Pin SQS to json + /QueueUrls and json actions.
    #[test]
    fn sns_subscriptions_are_query_and_sqs_is_json_not_query() {
        let subs = get_resource("sns-subscriptions").expect("sns-subscriptions");
        let api = subs.api_config.as_ref().expect("subs api_config");
        assert_eq!(api.protocol, crate::resource::protocol::ApiProtocol::Query);
        assert_eq!(
            api.response_root.as_deref(),
            Some("/ListSubscriptionsResponse/ListSubscriptionsResult/Subscriptions/member")
        );

        let topic_subs = get_resource("sns-topic-subscriptions").expect("sns-topic-subscriptions");
        let tsapi = topic_subs
            .api_config
            .as_ref()
            .expect("topic subs api_config");
        assert_eq!(
            tsapi.response_root.as_deref(),
            Some("/ListSubscriptionsByTopicResponse/ListSubscriptionsByTopicResult/Subscriptions/member")
        );
        assert!(
            topic_subs.requires_parent,
            "topic subscriptions need a TopicArn"
        );
        let topics = get_resource("sns-topics").expect("sns-topics");
        let drill = topics
            .sub_resources
            .iter()
            .find(|s| s.resource_key == "sns-topic-subscriptions")
            .expect("topics must drill subscriptions");
        assert_eq!(drill.parent_id_field, "TopicArn");

        let sqs = get_resource("sqs-queues").expect("sqs-queues");
        let qapi = sqs.api_config.as_ref().expect("sqs api_config");
        assert_eq!(
            qapi.protocol,
            crate::resource::protocol::ApiProtocol::Json,
            "SQS is a JSON API now"
        );
        assert_eq!(qapi.response_root.as_deref(), Some("/QueueUrls"));
        let qsvc = crate::aws::http::get_service("sqs").expect("sqs service");
        assert_eq!(qsvc.protocol, crate::aws::http::Protocol::Json);
        assert_eq!(qsvc.target_prefix, Some("AmazonSQS"));
        for (action_id, expected) in [
            ("purge_queue", "PurgeQueue"),
            ("delete_queue", "DeleteQueue"),
        ] {
            let ac = sqs
                .action_configs
                .get(action_id)
                .unwrap_or_else(|| panic!("{action_id} action must exist"));
            assert_eq!(
                ac.protocol,
                crate::resource::protocol::ApiProtocol::Json,
                "{action_id} must follow SQS to json"
            );
            assert_eq!(ac.action.as_deref(), Some(expected));
        }
    }

    /// The Elasticache/Redshift/RDS parameter groups, reserved nodes, and
    /// events are all query-protocol lists that page on Marker/MaxRecords (not
    /// NextToken) and answer from a `<action>Result/<List>/<member>` root. Pin
    /// every one of the ten with its exact root, plus the shared paging trio.
    #[test]
    fn database_family_parameter_groups_reserved_nodes_and_events_follow_the_query_pattern() {
        let data: &[(&str, &str, &str)] = &[
            ("elasticache-parameter-groups", "DescribeCacheParameterGroups",
             "/DescribeCacheParameterGroupsResponse/DescribeCacheParameterGroupsResult/CacheParameterGroups/CacheParameterGroup"),
            ("elasticache-reserved-nodes", "DescribeReservedCacheNodes",
             "/DescribeReservedCacheNodesResponse/DescribeReservedCacheNodesResult/ReservedCacheNodes/ReservedCacheNode"),
            ("elasticache-events", "DescribeEvents",
             "/DescribeEventsResponse/DescribeEventsResult/Events/Event"),
            ("elasticache-snapshots", "DescribeSnapshots",
             "/DescribeSnapshotsResponse/DescribeSnapshotsResult/Snapshots/Snapshot"),
            ("redshift-parameter-groups", "DescribeClusterParameterGroups",
             "/DescribeClusterParameterGroupsResponse/DescribeClusterParameterGroupsResult/ParameterGroups/ClusterParameterGroup"),
            ("redshift-reserved-nodes", "DescribeReservedNodes",
             "/DescribeReservedNodesResponse/DescribeReservedNodesResult/ReservedNodes/ReservedNode"),
            ("redshift-events", "DescribeEvents",
             "/DescribeEventsResponse/DescribeEventsResult/Events/Event"),
            ("rds-parameter-groups", "DescribeDBParameterGroups",
             "/DescribeDBParameterGroupsResponse/DescribeDBParameterGroupsResult/DBParameterGroups/DBParameterGroup"),
            ("rds-reserved-instances", "DescribeReservedDBInstances",
             "/DescribeReservedDBInstancesResponse/DescribeReservedDBInstancesResult/ReservedDBInstances/ReservedDBInstance"),
            ("rds-events", "DescribeEvents",
             "/DescribeEventsResponse/DescribeEventsResult/Events/Event"),
        ];
        for (key, action, root) in data {
            let r = get_resource(key).unwrap_or_else(|| panic!("{key} must parse"));
            let api = r
                .api_config
                .as_ref()
                .unwrap_or_else(|| panic!("{key} api_config"));
            assert_eq!(
                api.protocol,
                crate::resource::protocol::ApiProtocol::Query,
                "{key} is query protocol"
            );
            assert_eq!(api.action.as_deref(), Some(*action), "{key} action");
            assert_eq!(api.response_root.as_deref(), Some(*root), "{key} root");
            let pag = api
                .pagination
                .as_ref()
                .unwrap_or_else(|| panic!("{key} pages on Marker"));
            assert_eq!(
                pag.input_token.as_deref(),
                Some("Marker"),
                "{key} input token"
            );
            assert_eq!(
                pag.max_results_param.as_deref(),
                Some("MaxRecords"),
                "{key} max"
            );
            let expected_marker = format!("/{action}Response/{action}Result/Marker");
            assert_eq!(
                pag.output_token.as_deref(),
                Some(expected_marker.as_str()),
                "{key} output token"
            );
            for column in &r.columns {
                let map_root = column.json_path.split('.').next().unwrap();
                assert!(
                    r.field_mappings.contains_key(map_root),
                    "{key} column {map_root} must be mapped"
                );
            }
        }
        let snap = get_resource("elasticache-snapshots").expect("elasticache-snapshots");
        let snap_pag = snap
            .api_config
            .as_ref()
            .expect("snapshots pagination")
            .pagination
            .as_ref()
            .expect("snapshots page");
        assert_eq!(
            snap_pag.max_results,
            Some(50),
            "DescribeSnapshots caps MaxRecords at 50; 100 draws InvalidParameterValue"
        );
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

    /// ListResourceRecordSets returns every record in full and Route53 has no
    /// GetResourceRecordSet, so describe must not re-fetch: it reads the row
    /// orbit already has. The row is the *mapped* list item, so the raw record
    /// must be kept alongside (the `Raw` mapping) and describe_fields path into
    /// it; the API never returns RoutingPolicy/Alias, so those are derived by
    /// transform instead.
    #[test]
    fn route53_records_describe_from_the_row_already_fetched() {
        let r = get_resource("route53-records").expect("route53-records");
        assert!(
            r.describe_from_row,
            "records describe from the fetched row, not a re-fetch"
        );
        let raw_mapping = r
            .field_mappings
            .get("Raw")
            .expect("records must keep the raw record via the Raw mapping");
        assert_eq!(
            raw_mapping.source, "/",
            "Raw mapping must copy the whole raw record"
        );
        let dc = r.describe_config.as_ref().expect("describe_config");
        assert!(
            dc.path.is_none() && dc.action.is_none(),
            "a from-row describe declares no API call"
        );
        for (label, source) in [
            ("Name", "/Raw/Name"),
            ("Type", "/Raw/Type"),
            ("TTL", "/Raw/TTL"),
            ("Set ID", "/Raw/SetIdentifier"),
            ("Weight", "/Raw/Weight"),
            ("Failover", "/Raw/Failover"),
            ("Alias DNS", "/Raw/AliasTarget/DNSName"),
            ("Alias Zone ID", "/Raw/AliasTarget/HostedZoneId"),
            ("Country", "/Raw/GeoLocation/CountryCode"),
            ("CIDR Collection", "/Raw/CidrRoutingConfig/CollectionId"),
        ] {
            assert!(
                dc.describe_fields
                    .iter()
                    .any(|f| f.label == label && f.source == source),
                "records detail must surface {label} from {source}"
            );
        }
        for (label, transform) in [
            ("Alias", "route53_is_alias"),
            ("Routing Policy", "route53_routing_policy"),
        ] {
            let field = dc
                .describe_fields
                .iter()
                .find(|f| f.label == label)
                .unwrap_or_else(|| panic!("{label} needs a describe field"));
            assert_eq!(
                field.transform.as_deref(),
                Some(transform),
                "{label} is derived by transform, not on the wire"
            );
        }
        let values = dc
            .describe_fields
            .iter()
            .find(|f| f.source == "/Raw/ResourceRecords/ResourceRecord")
            .unwrap_or_else(|| panic!("records detail must list the record values"));
        assert!(values.list, "values is a multi-value field");
        assert_eq!(
            values.item_template.as_deref(),
            Some("{Value}"),
            "one templated line per record value"
        );
    }

    /// DescribeInstances already returns the full instance in the list, so the
    /// instance detail panel renders from the fetched row exactly like route53
    /// records — no second API call. Every describe field and overview path must
    /// extract against the real wire shape, whose element names are
    /// lowerCamelCase and do NOT match the PascalCase the AWS CLI prints.
    #[test]
    fn ec2_instance_detail_paths_extract_from_the_real_describeinstances_wire_shape() {
        use serde_json::json;

        let r = get_resource("ec2-instances").expect("ec2-instances");
        assert!(
            r.describe_from_row,
            "instances describe from the fetched row, not a re-fetch"
        );
        let raw_mapping = r
            .field_mappings
            .get("Raw")
            .expect("instances must keep the raw instance via the Raw mapping");
        assert_eq!(
            raw_mapping.source, "/",
            "Raw mapping must copy the whole raw instance"
        );
        let dc = r.describe_config.as_ref().expect("describe_config");
        assert!(
            dc.path.is_none() && dc.action.is_none(),
            "a from-row describe declares no API call"
        );

        // A slice of a real DescribeInstances response (eu-west-1c, captured 2026),
        // covering every element the detail panel reads.
        let raw = json!({
            "instanceId": "i-0123456789abcdef0",
            "instanceType": "m5.large",
            "instanceState": { "code": 16, "name": "running" },
            "architecture": "x86_64",
            "platformDetails": "Linux/UNIX",
            "virtualizationType": "hvm",
            "hypervisor": "xen",
            "launchTime": "2022-08-15T11:11:35.000Z",
            "imageId": "ami-0123456789abcdef0",
            "placement": {
                "availabilityZone": "eu-west-1c",
                "availabilityZoneId": "euw1-az2",
                "groupName": "",
                "tenancy": "default"
            },
            "monitoring": { "state": "enabled" },
            "dnsName": "ec2-52-210-150-98.eu-west-1.compute.amazonaws.com",
            "privateDnsName": "ip-172-31-27-199.eu-west-1.compute.internal",
            "ipAddress": "52.210.150.98",
            "privateIpAddress": "172.31.27.199",
            "ipv6Addresses": ["2001:db8::1"],
            "vpcId": "vpc-0123456789abcdef0",
            "subnetId": "subnet-0123456789abcdef0",
            "groupSet": {
                "item": [
                    { "groupId": "sg-1327d375", "groupName": "test-remove" },
                    { "groupId": "sg-fe6da399", "groupName": "DMC-CM-SSH" }
                ]
            },
            "rootDeviceName": "/dev/sda1",
            "rootDeviceType": "ebs",
            "ebsOptimized": true,
            "blockDeviceMapping": {
                "item": [{ "deviceName": "/dev/sda1", "ebs": { "volumeId": "vol-999f9a1e", "status": "attached" } }]
            },
            "keyName": "whg-deploy",
            "iamInstanceProfile": { "arn": "arn:aws:iam::259740665173:instance-profile/whg" },
            "hibernationOptions": { "configured": true },
            "metadataOptions": {
                "httpTokens": "required",
                "httpEndpoint": "enabled",
                "httpPutResponseHopLimit": 1,
                "state": "applied"
            },
            "enaSupport": true,
            "usageOperation": "RunInstances",
            "outpostArn": "arn:aws:outposts:eu-west-1:259740665173:outpost/op-01234567",
            "instanceLifecycle": "spot",
            "spotInstanceRequestId": "sir-0123456789abcdef0",
            "stateReason": {
                "code": "Client.UserInitiatedShutdown",
                "message": "Client.UserInitiatedShutdown: User initiated shutdown"
            },
            "tagSet": {
                "item": [
                    { "key": "Name", "value": "Reporting" },
                    { "key": "Team", "value": "DB" }
                ]
            }
        });
        let row = json!({
            "InstanceId": "i-0123456789abcdef0",
            "State": "running",
            "InstanceType": "m5.large",
            "PrivateIpAddress": "172.31.27.199",
            "PublicIpAddress": "52.210.150.98",
            "Tags": { "Name": "Reporting", "Team": "DB" },
            "Raw": raw
        });

        for field in &dc.describe_fields {
            let v = crate::resource::path_extractor::extract_by_path(&row, &field.source);
            assert!(
                !v.is_null(),
                "ec2-instances describe field {} source {} misses the DescribeInstances wire shape",
                field.label,
                field.source
            );
        }

        let overview = dc
            .overview
            .as_ref()
            .expect("instances get an overview banner");
        assert_eq!(overview.title_source, "/Tags/Name");
        for (label, source) in [
            ("State", "/State"),
            ("Type", "/InstanceType"),
            ("Private IP", "/PrivateIpAddress"),
            ("Public IP", "/PublicIpAddress"),
        ] {
            assert!(
                overview
                    .chips
                    .iter()
                    .any(|c| c.label == label && c.source == source),
                "overview chip {label} must read {source}"
            );
            let v = crate::resource::path_extractor::extract_by_path(&row, source);
            assert!(
                !v.is_null(),
                "overview chip {label} source {source} misses a mapped field"
            );
        }
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
    fn test_secretsmanager_value_is_not_revealable_from_the_list_row() {
        let resource = get_resource("secretsmanager-secrets").unwrap();
        assert!(
            !resource.actions.is_empty(),
            "Secrets Manager should keep its mutating actions"
        );

        // The secret value must only be shown once the user has entered the
        // secret (the detail graph hands off to the s reveal in the describe
        // view). A get_secret_value action here would dump it straight from
        // the list row without entering, which the reveal gate forbids.
        let view_action = resource
            .actions
            .iter()
            .find(|a| a.sdk_method == "get_secret_value");
        assert!(
            view_action.is_none(),
            "Secrets Manager must not expose get_secret_value as a list-row action"
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

    #[test]
    fn lambda_functions_has_expected_columns() {
        let resource = get_resource("lambda-functions").unwrap();
        let headers: Vec<&str> = resource.columns.iter().map(|c| c.header.as_str()).collect();

        for expected in &[
            "FUNCTION NAME",
            "RUNTIME",
            "MEMORY",
            "TIMEOUT",
            "STATE",
            "MODIFIED",
        ] {
            assert!(
                headers.contains(expected),
                "lambda-functions missing column {}",
                expected
            );
        }

        let hidden: Vec<&str> = resource
            .columns
            .iter()
            .filter(|c| !c.visible)
            .map(|c| c.header.as_str())
            .collect();
        for expected in &[
            "DESCRIPTION",
            "PACKAGE TYPE",
            "ARCHITECTURE",
            "CODE SIZE",
            "HANDLER",
            "ROLE",
            "VERSION",
        ] {
            assert!(
                hidden.contains(expected),
                "lambda-functions hidden column {} should have visible: false",
                expected
            );
        }
    }

    #[test]
    fn lambda_functions_describe_has_formatted_fields() {
        let resource = get_resource("lambda-functions").unwrap();
        let dc = resource
            .describe_config
            .as_ref()
            .expect("lambda-functions needs describe_config");
        assert_eq!(
            dc.response_path.as_deref(),
            Some("/Configuration"),
            "describe should narrow to /Configuration"
        );
        assert!(
            !dc.describe_fields.is_empty(),
            "lambda-functions needs describe_fields"
        );

        let labels: Vec<&str> = dc
            .describe_fields
            .iter()
            .map(|f| f.label.as_str())
            .collect();
        assert!(labels.contains(&"Function Name"), "missing Function Name");
        assert!(labels.contains(&"ARN"), "missing ARN");
        assert!(labels.contains(&"State"), "missing State");
        assert!(labels.contains(&"Runtime"), "missing Runtime");
        assert!(labels.contains(&"Memory"), "missing Memory");
        assert!(labels.contains(&"Timeout"), "missing Timeout");
    }

    #[test]
    fn lambda_functions_describe_has_overview_banner_and_triggers_diagram() {
        let resource = get_resource("lambda-functions").unwrap();
        let dc = resource
            .describe_config
            .as_ref()
            .expect("lambda-functions needs describe_config");

        let overview = dc
            .overview
            .as_ref()
            .expect("lambda-functions needs an overview banner");
        assert_eq!(
            overview.title_source.as_str(),
            "/FunctionName",
            "overview title should be the function name"
        );
        assert!(
            !overview.chips.is_empty(),
            "lambda overview should carry identity chips"
        );

        let resource_groups: Vec<&str> = overview
            .resources
            .iter()
            .map(|r| r.label.as_str())
            .collect();
        assert!(
            resource_groups.contains(&"EVENT SOURCE MAPPINGS"),
            "lambda overview should draw an EVENT SOURCE MAPPINGS group"
        );

        assert!(
            dc.enrich_calls.iter().any(|e| e
                .path
                .as_deref()
                .is_some_and(|p| p.contains("event-source-mappings"))),
            "lambda describe must enrich with ListEventSourceMappings for the mapping diagram"
        );
    }

    #[test]
    fn lambda_overview_trigger_source_maps_to_enrich_result_field() {
        let resource = get_resource("lambda-functions").unwrap();
        let dc = resource.describe_config.as_ref().unwrap();
        let overview = dc.overview.as_ref().unwrap();

        let triggers = overview
            .resources
            .iter()
            .find(|r| r.label == "EVENT SOURCE MAPPINGS")
            .expect("EVENT SOURCE MAPPINGS group exists");
        assert_eq!(
            triggers.source.as_str(),
            "/EventSourceMappings",
            "EVENT SOURCE MAPPINGS must read the enrich result field"
        );
        assert!(
            dc.enrich_calls
                .iter()
                .any(|e| e.result_field == "EventSourceMappings"),
            "enrich result_field must be EventSourceMappings"
        );
    }

    #[test]
    fn lambda_overview_lists_eventbridge_rules_that_target_the_function() {
        let resource = get_resource("lambda-functions").unwrap();
        let dc = resource.describe_config.as_ref().unwrap();

        // The console shows EventBridge rules (CloudWatch Events targets) as a
        // second trigger type distinct from event-source mappings. That lives
        // in the `events` service and needs the function ARN, so the enrich
        // must be a cross-service Json call, not another Lambda call.
        let rules = dc
            .enrich_calls
            .iter()
            .find(|e| e.result_field == "EventBridgeRules")
            .expect("lambda must enrich with EventBridgeRules");
        assert_eq!(
            rules.service.as_deref(),
            Some("events"),
            "EventBridge rules come from the events service"
        );
        assert_eq!(
            rules.protocol.as_ref(),
            Some(&crate::resource::protocol::ApiProtocol::Json),
            "events speaks the Json (X-Amz-Target) protocol"
        );
        assert_eq!(
            rules.target.as_deref(),
            Some("ListRuleNamesByTarget"),
            "rules enrich targets ListRuleNamesByTarget"
        );
        assert!(
            rules
                .body_template
                .as_deref()
                .is_some_and(|b| b.contains("{FunctionArn}")),
            "rules enrich must pass the function ARN, not the name"
        );

        let overview = dc.overview.as_ref().unwrap();
        assert!(
            overview
                .resources
                .iter()
                .any(|r| r.label == "EVENTBRIDGE RULES"),
            "overview must draw an EVENTBRIDGE RULES group"
        );
    }

    #[test]
    fn lambda_functions_id_field_is_mapped() {
        let resource = get_resource("lambda-functions").unwrap();
        assert!(
            resource.field_mappings.contains_key(&resource.id_field),
            "lambda-functions id_field {} must be in field_mappings",
            resource.id_field
        );
    }

    #[test]
    fn lambda_functions_new_columns_have_field_mappings() {
        let resource = get_resource("lambda-functions").unwrap();
        for col in &resource.columns {
            let root = col.json_path.split('.').next().unwrap_or("");
            assert!(
                resource.field_mappings.contains_key(&col.json_path)
                    || resource.field_mappings.contains_key(root),
                "lambda-functions column {} json_path {} has no field_mapping",
                col.header,
                col.json_path
            );
        }
    }
}
