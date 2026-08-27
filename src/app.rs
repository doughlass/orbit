use crate::aws;
use crate::aws::client::AwsClients;
use crate::config::Config;
use crate::resource::{
    extract_json_value, fetch_resources_paginated, get_all_resource_keys, get_resource,
    ResourceDef, ResourceFilter,
};
use anyhow::Result;
use crossterm::event::KeyCode;
use fuzzy_matcher::{skim::SkimMatcherV2, FuzzyMatcher};
use serde_json::Value;
use std::cmp::Ordering;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, PartialEq)]
pub enum Mode {
    Normal,       // Viewing list
    Command,      // : command input
    Help,         // ? help popup
    Confirm,      // Confirmation dialog
    Warning,      // Warning/info dialog (OK only)
    Profiles,     // Profile selection
    Regions,      // Region selection
    Describe,     // Viewing JSON details of selected item
    SsoLogin,     // SSO login dialog (IAM Identity Center)
    ConsoleLogin, // Console login dialog (aws login)
    LogTail,      // Tailing CloudWatch logs
    ColumnPicker, // Column visibility picker (p)
}

/// Pending action that requires confirmation
#[derive(Debug, Clone)]
pub struct PendingAction {
    /// Service name (e.g., "ec2")
    pub service: String,
    /// SDK method to call (e.g., "terminate_instance")  
    pub sdk_method: String,
    /// Resource ID to act on
    pub resource_id: String,
    /// Display message for confirmation dialog
    pub message: String,
    /// If true, default selection is No (kept for potential future use)
    #[allow(dead_code)]
    pub default_no: bool,
    /// If true, show as destructive (red)
    pub destructive: bool,
    /// Currently selected option (true = Yes, false = No)
    pub selected_yes: bool,
}

/// Parent context for hierarchical navigation
#[derive(Debug, Clone)]
pub struct ParentContext {
    /// Parent resource key (e.g., "vpc")
    pub resource_key: String,
    /// Parent item (the selected VPC, etc.)
    pub item: Value,
    /// Display name for breadcrumb
    pub display_name: String,
    /// Saved selection index to restore when navigating back
    pub saved_selected: usize,
}

/// AWS API Filters for server-side filtering
/// Supports key=value pairs like: architecture=arm64, owner=amazon, tag:Environment=prod
#[derive(Debug, Clone, Default)]
pub struct AwsFilters {
    /// List of filter key-value pairs
    pub filters: Vec<(String, String)>,
}

impl AwsFilters {
    /// Parse filters from text (format: "Filters: key=value, key2=value2")
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if !text.to_lowercase().starts_with("filters:") {
            return None;
        }

        let filters_part = text[8..].trim(); // Skip "Filters:"
        if filters_part.is_empty() {
            return None;
        }

        let mut filters = Vec::new();
        for part in filters_part.split(',') {
            let part = part.trim();
            if let Some(eq_pos) = part.find('=') {
                let key = part[..eq_pos].trim().to_string();
                let value = part[eq_pos + 1..].trim().to_string();
                if !key.is_empty() && !value.is_empty() {
                    filters.push((key, value));
                }
            }
        }

        if filters.is_empty() {
            None
        } else {
            Some(AwsFilters { filters })
        }
    }

    /// Display string for the filters
    pub fn display(&self) -> String {
        let pairs: Vec<String> = self
            .filters
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect();
        format!("Filters: {}", pairs.join(", "))
    }
}

/// Client-side column sort for the current table view.
///
/// The cursor and the sorted column are separate: arrows move the cursor freely
/// and only Tab commits it to a sort, so browsing columns never reshuffles rows.
///
/// Only sorts the rows already fetched, so on a paginated resource this orders
/// the loaded pages, not the full result set.
#[derive(Debug, Clone, Default)]
pub struct SortState {
    /// Column the arrows are pointing at (index into the resource's `columns`)
    pub cursor: usize,
    /// Column actually sorted, or None for API order
    pub column: Option<usize>,
    pub descending: bool,
}

impl SortState {
    /// Move the cursor one column right, wrapping at the end
    pub fn cursor_right(&mut self, column_count: usize) {
        if column_count == 0 {
            return;
        }
        self.cursor = (self.cursor + 1) % column_count;
    }

    /// Move the cursor one column left, wrapping at the start
    pub fn cursor_left(&mut self, column_count: usize) {
        if column_count == 0 {
            return;
        }
        self.cursor = (self.cursor + column_count - 1) % column_count;
    }

    /// Sort by the cursor's column: ascending on a newly picked column, otherwise
    /// flip the direction of the column already sorted.
    pub fn sort_by_cursor(&mut self) {
        if self.column == Some(self.cursor) {
            self.descending = !self.descending;
        } else {
            self.column = Some(self.cursor);
            self.descending = false;
        }
    }

    /// Drop back to the order the API returned, leaving the cursor where it is
    pub fn clear(&mut self) {
        self.column = None;
        self.descending = false;
    }

    /// Full reset for a change of resource, where column indices no longer apply
    pub fn reset(&mut self) {
        self.clear();
        self.cursor = 0;
    }

    pub fn indicator(&self) -> &'static str {
        if self.descending {
            "↓"
        } else {
            "↑"
        }
    }
}

/// A cell with no data. `extract_json_value` renders missing values as "-".
fn is_blank_cell(value: &str) -> bool {
    let trimmed = value.trim();
    trimmed.is_empty() || trimmed == "-"
}

/// Order two cell values, guessing the type from the content.
///
/// Numeric columns compare numerically so "9" sorts before "10". ISO-8601
/// timestamps need no special handling because they already sort lexically.
/// Blanks sink to the bottom in both directions so they never crowd the top.
fn compare_cells(a: &str, b: &str, descending: bool) -> Ordering {
    match (is_blank_cell(a), is_blank_cell(b)) {
        (true, true) => return Ordering::Equal,
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        (false, false) => {}
    }

    let ordering = match (a.trim().parse::<f64>(), b.trim().parse::<f64>()) {
        (Ok(x), Ok(y)) => x.total_cmp(&y),
        _ => a.to_lowercase().cmp(&b.to_lowercase()),
    };

    if descending {
        ordering.reverse()
    } else {
        ordering
    }
}

/// Sort `items` in place by the value at `sort_path`, returning where the row at
/// `selected` ended up so the cursor stays on the same resource.
///
/// Sorts a permutation of indices rather than the items themselves: keys are
/// extracted once each (`extract_json_value` clones the item, so calling it per
/// comparison would be costly), and ties keep their original API order.
fn sort_items(items: &mut [Value], sort_path: &str, descending: bool, selected: usize) -> usize {
    if items.is_empty() {
        return 0;
    }

    let keys: Vec<String> = items
        .iter()
        .map(|item| extract_json_value(item, sort_path))
        .collect();

    let mut order: Vec<usize> = (0..items.len()).collect();
    order.sort_by(|&a, &b| compare_cells(&keys[a], &keys[b], descending));

    let sorted: Vec<Value> = order.iter().map(|&i| items[i].clone()).collect();
    items.clone_from_slice(&sorted);

    order.iter().position(|&i| i == selected).unwrap_or(0)
}

pub struct App {
    // AWS Clients
    pub clients: AwsClients,

    // Demo mode — no AWS calls, uses pre-baked data
    pub demo: bool,

    // Current resource being viewed
    pub current_resource_key: String,

    // Dynamic data storage (JSON)
    pub items: Vec<Value>,
    pub filtered_items: Vec<Value>,

    // Navigation state
    pub selected: usize,
    pub mode: Mode,
    pub filter_text: String,
    pub filter_active: bool,

    // AWS API filters state (unified filter system)
    pub aws_filters: Option<AwsFilters>,
    pub filters_autocomplete_shown: bool,

    // Hierarchical navigation
    pub parent_context: Option<ParentContext>,
    pub navigation_stack: Vec<ParentContext>,

    // Command input
    pub command_text: String,
    pub command_suggestions: Vec<String>,
    pub command_suggestion_selected: usize,
    pub command_preview: Option<String>, // Ghost text for hovered suggestion

    // Profile/Region
    pub profile: String,
    pub region: String,
    pub available_profiles: Vec<String>,
    pub available_regions: Vec<String>,
    pub profiles_selected: usize,
    pub regions_selected: usize,

    // Confirmation
    pub pending_action: Option<PendingAction>,

    // UI state
    pub loading: bool,
    pub error_message: Option<String>,
    pub status_message: Option<String>, // Transient success/info message (not an error)
    pub describe_scroll: usize,
    pub describe_data: Option<Value>, // Full resource details from describe API
    pub last_action_display_name: Option<String>,

    // Describe search state
    pub describe_search_text: String,
    pub describe_search_active: bool,
    pub describe_match_lines: Vec<usize>, // Line numbers containing matches
    pub describe_current_match: usize,    // Index into match_lines

    // Auto-refresh
    pub last_refresh: std::time::Instant,

    // Persistent configuration
    pub config: Config,

    // Key press tracking for sequences (e.g., 'gg')
    pub last_key_press: Option<(KeyCode, std::time::Instant)>,

    // Read-only mode (blocks all write operations)
    pub readonly: bool,

    // Warning message for modal dialog
    pub warning_message: Option<String>,

    // Custom endpoint URL (for LocalStack, etc.)
    pub endpoint_url: Option<String>,

    // SSO login state (IAM Identity Center)
    pub sso_state: Option<SsoLoginState>,

    // Console login state (aws login)
    pub console_login_state: Option<ConsoleLoginState>,

    // Console login child process (not in ConsoleLoginState because Child is not Clone)
    pub console_login_child: Option<std::process::Child>,

    // Console login URL receiver (for capturing URL from subprocess stderr)
    pub console_login_rx: Option<std::sync::mpsc::Receiver<crate::aws::console_login::LoginInfo>>,

    // Pagination state
    pub pagination: PaginationState,

    // Log tail state
    pub log_tail_state: Option<LogTailState>,

    // SSM connect request (instance_id, region, profile)
    pub ssm_connect_request: Option<SsmConnectRequest>,

    // Fuzzy matcher for filtering (reused to avoid repeated allocations)
    pub fuzzy_matcher: SkimMatcherV2,

    // Client-side column sort for the current table
    pub sort: SortState,

    // Column picker state (p key). Toggle state is indexed against the
    // resource's full column list; the picker renders and edits this vec.
    pub column_picker_toggles: Vec<bool>,
    pub column_picker_selected: usize,

    // Horizontal table scroll: index of the first visible column when the
    // table overflows the terminal width. Zero in fit mode.
    pub h_scroll: usize,
}

/// SSM Connect request data
#[derive(Debug, Clone)]
pub struct SsmConnectRequest {
    pub instance_id: String,
    pub region: String,
    pub profile: String,
}

/// Pagination state for resource listings
#[derive(Debug, Clone)]
pub struct PaginationState {
    /// Token for fetching next page (None if no more pages)
    pub next_token: Option<String>,
    /// Stack of previous page tokens for going back
    pub token_stack: Vec<Option<String>>,
    /// Current page number (1-indexed for display)
    pub current_page: usize,
    /// Whether there are more pages available
    pub has_more: bool,
}

impl Default for PaginationState {
    fn default() -> Self {
        Self {
            next_token: None,
            token_stack: Vec::new(),
            current_page: 1,
            has_more: false,
        }
    }
}

/// SSO Login dialog state
#[derive(Debug, Clone)]
pub enum SsoLoginState {
    /// Prompt to start login
    Prompt {
        profile: String,
        sso_session: String,
    },
    /// Waiting for browser auth
    WaitingForAuth {
        profile: String,
        user_code: String,
        verification_uri: String,
        #[allow(dead_code)]
        device_code: String,
        #[allow(dead_code)]
        interval: u64,
        #[allow(dead_code)]
        sso_region: String,
    },
    /// Login succeeded - contains profile to switch to
    Success { profile: String },
    /// Login failed
    Failed { error: String },
}

/// State for console login (aws login) dialog
#[derive(Debug, Clone)]
pub enum ConsoleLoginState {
    /// Prompt to run aws login
    Prompt {
        profile: String,
        login_session: String,
    },
    /// Waiting for browser auth (subprocess running)
    WaitingForAuth {
        profile: String,
        login_session: String,
        /// URL captured from subprocess output (displayed in dialog)
        url: Option<String>,
    },
    /// Login succeeded - contains profile to switch to
    Success { profile: String },
    /// Login failed
    Failed { profile: String, error: String },
}

/// Result of profile switch attempt
#[derive(Debug, Clone)]
pub enum ProfileSwitchResult {
    /// Profile switched successfully
    Success,
    /// SSO login required for this profile (IAM Identity Center)
    SsoRequired {
        profile: String,
        sso_session: String,
    },
    /// Console login required for this profile (aws login)
    ConsoleLoginRequired {
        profile: String,
        login_session: String,
    },
}

/// A single log event from CloudWatch
#[derive(Debug, Clone)]
pub struct LogEvent {
    pub timestamp: i64,
    pub message: String,
}

/// State for log tailing mode
#[derive(Debug, Clone)]
pub struct LogTailState {
    /// Log group name
    pub log_group: String,
    /// Log stream name
    pub log_stream: String,
    /// Collected log events (max 1000)
    pub events: Vec<LogEvent>,
    /// Scroll position in the log view
    pub scroll: usize,
    /// Token for fetching next batch of events
    pub next_forward_token: Option<String>,
    /// Whether to auto-scroll to bottom on new events
    pub auto_scroll: bool,
    /// Whether polling is paused
    pub paused: bool,
    /// Last time we polled for new events
    pub last_poll: std::time::Instant,
    /// Error message if polling failed
    pub error: Option<String>,
}

impl App {
    /// Create App from pre-initialized components (used with splash screen)
    #[allow(clippy::too_many_arguments)]
    pub fn from_initialized(
        clients: AwsClients,
        profile: String,
        region: String,
        available_profiles: Vec<String>,
        available_regions: Vec<String>,
        initial_items: Vec<Value>,
        config: Config,
        readonly: bool,
        endpoint_url: Option<String>,
        demo: bool,
        initial_resource_key: &str,
    ) -> Self {
        let filtered_items = initial_items.clone();

        Self {
            clients,
            demo,
            current_resource_key: initial_resource_key.to_string(),
            items: initial_items,
            filtered_items,
            selected: 0,
            mode: Mode::Normal,
            filter_text: String::new(),
            filter_active: false,
            aws_filters: None,
            filters_autocomplete_shown: false,
            parent_context: None,
            navigation_stack: Vec::new(),
            command_text: String::new(),
            command_suggestions: Vec::new(),
            command_suggestion_selected: 0,
            command_preview: None,
            profile,
            region,
            available_profiles,
            available_regions,
            profiles_selected: 0,
            regions_selected: 0,
            pending_action: None,
            loading: false,
            error_message: None,
            status_message: None,
            describe_scroll: 0,
            describe_data: None,
            last_action_display_name: None,
            describe_search_text: String::new(),
            describe_search_active: false,
            describe_match_lines: Vec::new(),
            describe_current_match: 0,
            last_refresh: std::time::Instant::now(),
            config,
            last_key_press: None,
            readonly,
            warning_message: None,
            endpoint_url,
            sso_state: None,
            console_login_state: None,
            console_login_child: None,
            console_login_rx: None,
            pagination: PaginationState::default(),
            log_tail_state: None,
            ssm_connect_request: None,
            fuzzy_matcher: SkimMatcherV2::default().ignore_case(),
            sort: SortState::default(),
            column_picker_toggles: Vec::new(),
            column_picker_selected: 0,
            h_scroll: 0,
        }
    }

    /// Check if auto-refresh is needed
    /// Auto-refresh is disabled - use 'R' to manually refresh
    pub fn needs_refresh(&self) -> bool {
        false
    }

    /// Reset refresh timer
    pub fn mark_refreshed(&mut self) {
        self.last_refresh = std::time::Instant::now();
    }

    // =========================================================================
    // Resource Definition Access
    // =========================================================================

    /// Get current resource definition
    pub fn current_resource(&self) -> Option<&'static ResourceDef> {
        get_resource(&self.current_resource_key)
    }

    /// Get available commands for autocomplete
    pub fn get_available_commands(&self) -> Vec<String> {
        let mut commands: Vec<String> = get_all_resource_keys()
            .iter()
            .map(|s| s.to_string())
            .collect();

        // Add profiles and regions commands
        commands.push("profiles".to_string());
        commands.push("regions".to_string());

        commands.sort();
        commands
    }

    // =========================================================================
    // Data Fetching
    // =========================================================================

    /// Fetch data for current resource (first page or current page based on pagination state)
    pub async fn refresh_current(&mut self) -> Result<()> {
        // Fetch the current page (uses pagination.next_token if set by next_page/prev_page)
        self.fetch_page(self.pagination.next_token.clone()).await
    }

    /// Fetch a specific page of resources
    async fn fetch_page(&mut self, page_token: Option<String>) -> Result<()> {
        if self.current_resource().is_none() {
            self.error_message = Some(format!("Unknown resource: {}", self.current_resource_key));
            return Ok(());
        }

        if self.demo {
            return Ok(());
        }

        self.loading = true;
        self.error_message = None;

        // Build filters from parent context
        let filters = self.build_filters_from_context();

        // Use paginated fetch - returns only one page of results
        match fetch_resources_paginated(
            &self.current_resource_key,
            &self.clients,
            &filters,
            page_token.as_deref(),
        )
        .await
        {
            Ok(result) => {
                // Preserve selection if possible
                let prev_selected = self.selected;
                self.items = result.items;
                self.apply_filter();

                // Update pagination state
                self.pagination.has_more = result.next_token.is_some();
                self.pagination.next_token = result.next_token;

                // Try to keep the same selection index
                if prev_selected < self.filtered_items.len() {
                    self.selected = prev_selected;
                } else {
                    self.selected = 0;
                }
            }
            Err(e) => {
                self.error_message = Some(aws::client::format_aws_error(&e));
                // Clear items to prevent mismatch between current_resource_key and stale items
                self.items.clear();
                self.filtered_items.clear();
                self.selected = 0;
                self.pagination = PaginationState::default();
            }
        }

        self.loading = false;
        self.mark_refreshed();
        Ok(())
    }

    /// Fetch next page of resources
    pub async fn next_page(&mut self) -> Result<()> {
        if !self.pagination.has_more {
            return Ok(());
        }

        // Save current token to stack for going back
        let current_token = self.pagination.next_token.clone();
        self.pagination.token_stack.push(current_token.clone());
        self.pagination.current_page += 1;

        // Fetch next page
        self.fetch_page(current_token).await
    }

    /// Fetch previous page of resources
    pub async fn prev_page(&mut self) -> Result<()> {
        if self.pagination.current_page <= 1 {
            return Ok(());
        }

        // Pop the previous token from stack
        self.pagination.token_stack.pop(); // Remove current page's token
        let prev_token = self.pagination.token_stack.pop().flatten(); // Get previous page's token
        self.pagination.current_page -= 1;

        // Fetch previous page
        self.fetch_page(prev_token).await
    }

    /// Reset pagination state (call when navigating to new resource)
    pub fn reset_pagination(&mut self) {
        self.pagination = PaginationState::default();
        self.h_scroll = 0;
    }

    /// Build AWS filters from parent context and AWS API filters
    /// For S3, this collects both bucket_names and prefix from navigation stack
    fn build_filters_from_context(&self) -> Vec<ResourceFilter> {
        let mut filters = Vec::new();

        // Add AWS API filters if present (unified filter system)
        if let Some(ref aws_filters) = self.aws_filters {
            for (key, value) in &aws_filters.filters {
                // Special handling for "owner" - uses Owner.N param, not Filter
                if key.to_lowercase() == "owner" {
                    filters.push(ResourceFilter::new(
                        &format!("owner:{}", value),
                        vec![value.clone()],
                    ));
                } else if key.starts_with("tag:") {
                    // Tag filters: tag:Key=Value -> Filter.N.Name=tag:Key, Filter.N.Value.1=Value
                    filters.push(ResourceFilter::new(key, vec![value.clone()]));
                } else {
                    // Regular filters: key=value -> Filter.N.Name=key, Filter.N.Value.1=value
                    filters.push(ResourceFilter::new(
                        &format!("filter:{}", key),
                        vec![value.clone()],
                    ));
                }
            }
        }

        let Some(parent) = &self.parent_context else {
            return filters;
        };

        let Some(_resource) = self.current_resource() else {
            return filters;
        };

        // For S3 objects, we need to collect filters from entire navigation stack
        // to preserve bucket_names while adding prefix
        if self.current_resource_key == "s3-objects" {
            // First, check navigation stack for bucket_names (from s3-buckets -> s3-objects)
            for ctx in &self.navigation_stack {
                if ctx.resource_key == "s3-buckets" {
                    if let Some(parent_resource) = get_resource(&ctx.resource_key) {
                        for sub in &parent_resource.sub_resources {
                            if sub.resource_key == "s3-objects" {
                                let bucket_name =
                                    extract_json_value(&ctx.item, &sub.parent_id_field);
                                if bucket_name != "-" {
                                    filters.push(ResourceFilter::new(
                                        &sub.filter_param,
                                        vec![bucket_name],
                                    ));
                                }
                            }
                        }
                    }
                }
            }

            // If parent is s3-buckets, get bucket_names from it
            if parent.resource_key == "s3-buckets" {
                if let Some(parent_resource) = get_resource(&parent.resource_key) {
                    for sub in &parent_resource.sub_resources {
                        if sub.resource_key == "s3-objects" {
                            let bucket_name =
                                extract_json_value(&parent.item, &sub.parent_id_field);
                            if bucket_name != "-" {
                                filters.push(ResourceFilter::new(
                                    &sub.filter_param,
                                    vec![bucket_name],
                                ));
                            }
                        }
                    }
                }
            }

            // If parent is s3-objects (folder navigation), get prefix from it
            if parent.resource_key == "s3-objects" {
                // Check if selected item is a folder
                let is_folder = parent
                    .item
                    .get("IsFolder")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);

                if is_folder {
                    let prefix = extract_json_value(&parent.item, "Key");
                    if prefix != "-" {
                        filters.push(ResourceFilter::new("prefix", vec![prefix]));
                    }
                }
            }

            return filters;
        }

        // Default behavior for other resources
        if let Some(parent_resource) = get_resource(&parent.resource_key) {
            for sub in &parent_resource.sub_resources {
                if sub.resource_key == self.current_resource_key {
                    // Extract parent ID value
                    let parent_id = extract_json_value(&parent.item, &sub.parent_id_field);
                    if parent_id != "-" {
                        return vec![ResourceFilter::with_type(
                            &sub.filter_param,
                            vec![parent_id],
                            &sub.filter_type,
                        )];
                    }
                }
            }
        }

        Vec::new()
    }

    // =========================================================================
    // Filtering
    // =========================================================================

    /// Apply text filter to items
    /// Searches across all visible column values (name, id, and all other attributes)
    pub fn apply_filter(&mut self) {
        let query = self.filter_text.trim();

        if query.is_empty() {
            self.filtered_items = self.items.clone();
        } else {
            let resource = self.current_resource();

            // Collect items with their match scores
            let mut scored_items: Vec<(i64, Value)> = self
                .items
                .iter()
                .filter_map(|item| {
                    if let Some(res) = resource {
                        // Search across all column values (visible attributes)
                        let mut best_score: Option<i64> = None;

                        for col in &res.columns {
                            let value = extract_json_value(item, &col.json_path);
                            if let Some(score) = self.fuzzy_matcher.fuzzy_match(&value, query) {
                                best_score = Some(best_score.map_or(score, |s| s.max(score)));
                            }
                        }

                        // Also search name_field and id_field if not already in columns
                        let name = extract_json_value(item, &res.name_field);
                        if let Some(score) = self.fuzzy_matcher.fuzzy_match(&name, query) {
                            best_score = Some(best_score.map_or(score, |s| s.max(score)));
                        }

                        let id = extract_json_value(item, &res.id_field);
                        if let Some(score) = self.fuzzy_matcher.fuzzy_match(&id, query) {
                            best_score = Some(best_score.map_or(score, |s| s.max(score)));
                        }

                        best_score.map(|score| (score, item.clone()))
                    } else {
                        // Fallback: search in JSON string
                        self.fuzzy_matcher
                            .fuzzy_match(&item.to_string(), query)
                            .map(|score| (score, item.clone()))
                    }
                })
                .collect();

            // Sort by score descending (higher score = better match)
            scored_items.sort_by_key(|b| std::cmp::Reverse(b.0));

            // Extract just the items
            self.filtered_items = scored_items.into_iter().map(|(_, item)| item).collect();
        }

        // An explicit column sort overrides the fuzzy relevance ranking above
        self.apply_sort();

        // Adjust selection
        if self.selected >= self.filtered_items.len() && !self.filtered_items.is_empty() {
            self.selected = self.filtered_items.len() - 1;
        }
    }

    // =========================================================================
    // Sorting
    // =========================================================================

    /// Number of sortable columns in the current view
    fn sortable_column_count(&self) -> usize {
        self.current_resource()
            .map(|r| r.columns.len())
            .unwrap_or(0)
    }

    /// Reorder `filtered_items` by the active sort column, keeping the cursor on
    /// the same item. No-op when no sort is active.
    pub fn apply_sort(&mut self) {
        let sort_path = match (self.sort.column, self.current_resource()) {
            (Some(index), Some(resource)) => {
                resource.columns.get(index).map(|col| col.json_path.clone())
            }
            _ => None,
        };
        let Some(sort_path) = sort_path else {
            return;
        };

        self.selected = sort_items(
            &mut self.filtered_items,
            &sort_path,
            self.sort.descending,
            self.selected,
        );
    }

    /// Move the column cursor right. Does not re-sort: Tab commits the choice.
    pub fn sort_cursor_right(&mut self) {
        self.sort.cursor_right(self.sortable_column_count());
    }

    /// Move the column cursor left. Does not re-sort: Tab commits the choice.
    pub fn sort_cursor_left(&mut self) {
        self.sort.cursor_left(self.sortable_column_count());
    }

    /// Sort by the cursor's column, or flip direction if it is already sorted
    pub fn sort_by_cursor(&mut self) {
        if self.sortable_column_count() == 0 {
            return;
        }
        self.sort.sort_by_cursor();
        self.apply_sort();
    }

    /// Tab through sort columns. Each column sorts ascending on first pick
    /// and flips to descending on the next press, then Tab moves on: with
    /// three columns the cycle is A0, D0, A1, D1, A2, D2, A0... The header
    /// arrow always shows what is sorted, so no preview cursor is needed —
    /// the arrow keys belong to horizontal scrolling now.
    pub fn sort_next_column(&mut self) {
        let count = self.sortable_column_count();
        if count == 0 {
            return;
        }
        match self.sort.column {
            // Ascending on the cursor's column: the next press flips it
            Some(sorted) if sorted == self.sort.cursor && !self.sort.descending => {}
            // Descending here, or a different column sorted: move along
            Some(_) => {
                self.sort.cursor = (self.sort.cursor + 1) % count;
            }
            None => {}
        }
        self.sort.sort_by_cursor();
        self.apply_sort();
    }

    /// Scroll the table one column left. No-op at the start.
    pub fn h_scroll_left(&mut self) {
        self.h_scroll = self.h_scroll.saturating_sub(1);
    }

    /// Scroll the table one column right. The UI clamps against the number of
    /// columns that actually overflow; here we only bound by the column count.
    pub fn h_scroll_right(&mut self) {
        let max = self
            .current_resource()
            .map(|r| r.columns.len())
            .unwrap_or(0);
        if self.h_scroll + 1 < max {
            self.h_scroll += 1;
        }
    }

    /// Drop the sort and restore the order the API returned
    pub fn clear_sort(&mut self) {
        if self.sort.column.is_none() {
            return;
        }
        self.sort.clear();
        // Rebuild from `items` — sorting is destructive, so API order can only be recovered by re-filtering
        self.apply_filter();
    }

    /// Header of the column the cursor sits on, i.e. what Tab would sort
    pub fn sort_cursor_header(&self) -> Option<&'static str> {
        self.current_resource()?
            .columns
            .get(self.sort.cursor)
            .map(|col| col.header.as_str())
    }

    /// Header label for the active sort, e.g. "CREATED ↓"
    pub fn sort_display(&self) -> Option<String> {
        let index = self.sort.column?;
        let column = self.current_resource()?.columns.get(index)?;
        Some(format!("{} {}", column.header, self.sort.indicator()))
    }

    /// Start a new filter, clearing any existing AWS filters
    /// Returns true if a refresh is needed (filters were cleared)
    pub fn start_new_filter(&mut self) -> bool {
        let needs_refresh = self.aws_filters.is_some();
        self.filter_text.clear();
        self.aws_filters = None;
        self.filters_autocomplete_shown = false;
        self.filter_active = true;
        if needs_refresh {
            self.reset_pagination();
        }
        needs_refresh
    }

    pub fn clear_filter(&mut self) {
        self.filter_text.clear();
        self.filter_active = false;
        self.aws_filters = None;
        self.filters_autocomplete_shown = false;
        self.apply_filter();
    }

    /// Check if the current resource supports filtering via AWS API
    pub fn current_resource_supports_filters(&self) -> bool {
        self.current_resource()
            .map(|r| r.supports_filters())
            .unwrap_or(false)
    }

    /// Get filter hint for current resource
    pub fn current_resource_filters_hint(&self) -> Option<String> {
        self.current_resource()
            .and_then(|r| r.filters_hint().map(|s| s.to_string()))
    }

    /// Check if filter text should trigger filters autocomplete (just "F" or "Fi" or "Filters")
    pub fn should_show_filters_autocomplete(&self) -> bool {
        if !self.current_resource_supports_filters() {
            return false;
        }
        let text = self.filter_text.trim().to_lowercase();
        !text.is_empty() && "filters:".starts_with(&text) && !text.contains(':')
    }

    /// Clear AWS filters and refresh
    pub async fn clear_aws_filters(&mut self) -> anyhow::Result<()> {
        if self.aws_filters.is_some() {
            self.aws_filters = None;
            self.reset_pagination();
            self.refresh_current().await?;
        }
        Ok(())
    }

    /// Get a display string for the current AWS filters
    pub fn aws_filters_display(&self) -> Option<String> {
        self.aws_filters.as_ref().map(|f| f.display())
    }

    // =========================================================================
    // Navigation
    // =========================================================================

    #[allow(dead_code)]
    pub fn current_list_len(&self) -> usize {
        self.filtered_items.len()
    }

    pub fn selected_item(&self) -> Option<&Value> {
        self.filtered_items.get(self.selected)
    }

    pub fn selected_item_json(&self) -> Option<String> {
        // Use describe_data if available (full details), otherwise fall back to list data
        if let Some(ref data) = self.describe_data {
            return Some(serde_json::to_string_pretty(data).unwrap_or_default());
        }
        self.selected_item()
            .map(|item| serde_json::to_string_pretty(item).unwrap_or_default())
    }

    /// Get the number of lines in the describe content
    pub fn describe_line_count(&self) -> usize {
        self.selected_item_json()
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }

    /// Get the maximum scroll position for describe view
    /// Uses an estimate of visible lines since we don't have access to terminal size here
    fn describe_max_scroll(&self) -> usize {
        let total = self.describe_line_count();
        // Estimate ~40 visible lines (typical terminal height minus headers/footers)
        let visible_estimate = 40;
        total.saturating_sub(visible_estimate)
    }

    /// Scroll describe view down by amount, clamped to max
    pub fn describe_scroll_down(&mut self, amount: usize) {
        let max_scroll = self.describe_max_scroll();
        self.describe_scroll = self.describe_scroll.saturating_add(amount).min(max_scroll);
    }

    /// Scroll describe view up by amount
    pub fn describe_scroll_up(&mut self, amount: usize) {
        self.describe_scroll = self.describe_scroll.saturating_sub(amount);
    }

    /// Scroll describe view to bottom
    pub fn describe_scroll_to_bottom(&mut self, visible_lines: usize) {
        let total = self.describe_line_count();
        self.describe_scroll = total.saturating_sub(visible_lines);
    }

    /// Clear describe search
    pub fn clear_describe_search(&mut self) {
        self.describe_search_text.clear();
        self.describe_search_active = false;
        self.describe_match_lines.clear();
        self.describe_current_match = 0;
    }

    /// Update describe search matches
    pub fn update_describe_search(&mut self) {
        self.describe_match_lines.clear();
        self.describe_current_match = 0;

        if self.describe_search_text.is_empty() {
            return;
        }

        let search_lower = self.describe_search_text.to_lowercase();

        if let Some(json) = self.selected_item_json() {
            for (line_num, line) in json.lines().enumerate() {
                if line.to_lowercase().contains(&search_lower) {
                    self.describe_match_lines.push(line_num);
                }
            }
        }

        // Jump to first match if found
        if !self.describe_match_lines.is_empty() {
            self.describe_scroll = self.describe_match_lines[0];
        }
    }

    /// Jump to next search match
    pub fn describe_next_match(&mut self) {
        if self.describe_match_lines.is_empty() {
            return;
        }
        self.describe_current_match =
            (self.describe_current_match + 1) % self.describe_match_lines.len();
        self.describe_scroll = self.describe_match_lines[self.describe_current_match];
    }

    /// Jump to previous search match
    pub fn describe_prev_match(&mut self) {
        if self.describe_match_lines.is_empty() {
            return;
        }
        if self.describe_current_match == 0 {
            self.describe_current_match = self.describe_match_lines.len() - 1;
        } else {
            self.describe_current_match -= 1;
        }
        self.describe_scroll = self.describe_match_lines[self.describe_current_match];
    }

    pub fn next(&mut self) {
        match self.mode {
            Mode::Profiles => {
                if !self.available_profiles.is_empty() {
                    self.profiles_selected =
                        (self.profiles_selected + 1).min(self.available_profiles.len() - 1);
                }
            }
            Mode::Regions => {
                if !self.available_regions.is_empty() {
                    self.regions_selected =
                        (self.regions_selected + 1).min(self.available_regions.len() - 1);
                }
            }
            _ => {
                if !self.filtered_items.is_empty() {
                    self.selected = (self.selected + 1).min(self.filtered_items.len() - 1);
                }
            }
        }
    }

    pub fn previous(&mut self) {
        match self.mode {
            Mode::Profiles => {
                self.profiles_selected = self.profiles_selected.saturating_sub(1);
            }
            Mode::Regions => {
                self.regions_selected = self.regions_selected.saturating_sub(1);
            }
            _ => {
                self.selected = self.selected.saturating_sub(1);
            }
        }
    }

    pub fn go_to_top(&mut self) {
        match self.mode {
            Mode::Profiles => self.profiles_selected = 0,
            Mode::Regions => self.regions_selected = 0,
            _ => self.selected = 0,
        }
    }

    pub fn go_to_bottom(&mut self) {
        match self.mode {
            Mode::Profiles => {
                if !self.available_profiles.is_empty() {
                    self.profiles_selected = self.available_profiles.len() - 1;
                }
            }
            Mode::Regions => {
                if !self.available_regions.is_empty() {
                    self.regions_selected = self.available_regions.len() - 1;
                }
            }
            _ => {
                if !self.filtered_items.is_empty() {
                    self.selected = self.filtered_items.len() - 1;
                }
            }
        }
    }

    pub fn page_down(&mut self, page_size: usize) {
        match self.mode {
            Mode::Profiles => {
                if !self.available_profiles.is_empty() {
                    self.profiles_selected =
                        (self.profiles_selected + page_size).min(self.available_profiles.len() - 1);
                }
            }
            Mode::Regions => {
                if !self.available_regions.is_empty() {
                    self.regions_selected =
                        (self.regions_selected + page_size).min(self.available_regions.len() - 1);
                }
            }
            _ => {
                if !self.filtered_items.is_empty() {
                    self.selected = (self.selected + page_size).min(self.filtered_items.len() - 1);
                }
            }
        }
    }

    pub fn page_up(&mut self, page_size: usize) {
        match self.mode {
            Mode::Profiles => {
                self.profiles_selected = self.profiles_selected.saturating_sub(page_size);
            }
            Mode::Regions => {
                self.regions_selected = self.regions_selected.saturating_sub(page_size);
            }
            _ => {
                self.selected = self.selected.saturating_sub(page_size);
            }
        }
    }

    // =========================================================================
    // Mode Transitions
    // =========================================================================

    pub fn enter_command_mode(&mut self) {
        self.mode = Mode::Command;
        self.command_text.clear();
        self.command_suggestions = self.get_available_commands();
        self.command_suggestion_selected = 0;
        self.command_preview = None;
    }

    pub fn update_command_suggestions(&mut self) {
        let input = self.command_text.to_lowercase();
        let all_commands = self.get_available_commands();

        if input.is_empty() {
            self.command_suggestions = all_commands;
        } else {
            self.command_suggestions = all_commands
                .into_iter()
                .filter(|cmd| cmd.contains(&input))
                .collect();
        }

        if self.command_suggestion_selected >= self.command_suggestions.len() {
            self.command_suggestion_selected = 0;
        }

        // Update preview to show current selection
        self.update_preview();
    }

    fn update_preview(&mut self) {
        if self.command_suggestions.is_empty() {
            self.command_preview = None;
        } else {
            self.command_preview = self
                .command_suggestions
                .get(self.command_suggestion_selected)
                .cloned();
        }
    }

    pub fn next_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            self.command_suggestion_selected =
                (self.command_suggestion_selected + 1) % self.command_suggestions.len();
            // Update preview (ghost text) without changing command_text
            self.update_preview();
        }
    }

    pub fn prev_suggestion(&mut self) {
        if !self.command_suggestions.is_empty() {
            if self.command_suggestion_selected == 0 {
                self.command_suggestion_selected = self.command_suggestions.len() - 1;
            } else {
                self.command_suggestion_selected -= 1;
            }
            // Update preview (ghost text) without changing command_text
            self.update_preview();
        }
    }

    pub fn apply_suggestion(&mut self) {
        // Apply the preview to command_text (on Tab/Right)
        if let Some(preview) = &self.command_preview {
            self.command_text = preview.clone();
            self.update_command_suggestions();
        }
    }

    pub fn enter_help_mode(&mut self) {
        self.mode = Mode::Help;
    }

    /// Open the column picker for the current resource. Toggle state starts
    /// from saved preferences if any, otherwise from the JSON `visible` flags.
    pub fn enter_column_picker(&mut self) {
        let Some(resource) = self.current_resource() else {
            return;
        };
        if resource.columns.is_empty() {
            return;
        }

        let saved = self.config.column_preferences(&self.current_resource_key);
        self.column_picker_toggles = resource
            .columns
            .iter()
            .map(|col| match saved {
                Some(visible) => visible.contains(&col.header),
                None => col.visible,
            })
            .collect();
        self.column_picker_selected = 0;
        self.mode = Mode::ColumnPicker;
    }

    /// Toggle the column under the picker cursor. Refuses to hide the last
    /// visible column — an empty table is never a useful outcome.
    pub fn column_picker_toggle(&mut self) {
        let idx = self.column_picker_selected;
        if idx >= self.column_picker_toggles.len() {
            return;
        }
        let visible_count = self.column_picker_toggles.iter().filter(|&&v| v).count();
        if self.column_picker_toggles[idx] && visible_count <= 1 {
            self.show_warning("At least one column must stay visible");
            return;
        }
        self.column_picker_toggles[idx] = !self.column_picker_toggles[idx];
    }

    /// Save toggled columns to config (persisted to disk) and close.
    pub fn save_column_picker(&mut self) {
        let Some(resource) = self.current_resource() else {
            self.mode = Mode::Normal;
            return;
        };

        let visible: Vec<String> = resource
            .columns
            .iter()
            .zip(&self.column_picker_toggles)
            .filter(|(_, &on)| on)
            .map(|(col, _)| col.header.clone())
            .collect();

        if visible.is_empty() {
            self.mode = Mode::Normal;
            return;
        }

        self.config
            .column_preferences
            .insert(self.current_resource_key.clone(), visible);
        if let Err(e) = self.config.save() {
            self.error_message = Some(format!("Failed to save config: {}", e));
        }
        self.mode = Mode::Normal;
    }

    /// Resolve the columns to render for the current resource, paired with each
    /// column's original index so sort state stays pinned to full-list
    /// positions even when columns are hidden. Saved picker preferences win
    /// over the JSON `visible` flags.
    pub fn effective_columns(&self) -> Vec<(usize, &crate::resource::ColumnDef)> {
        let Some(resource) = self.current_resource() else {
            return Vec::new();
        };
        match self.config.column_preferences(&self.current_resource_key) {
            Some(visible) => resource
                .columns
                .iter()
                .enumerate()
                .filter(|(_, col)| visible.contains(&col.header))
                .collect(),
            None => resource
                .columns
                .iter()
                .enumerate()
                .filter(|(_, col)| col.visible)
                .collect(),
        }
    }

    pub async fn enter_describe_mode(&mut self) {
        if self.filtered_items.is_empty() {
            return;
        }

        self.mode = Mode::Describe;
        self.describe_scroll = 0;
        self.describe_data = None;

        // Get the selected item's ID
        if let Some(item) = self.selected_item().cloned() {
            if let Some(resource_def) = self.current_resource() {
                // Check if this resource has a detail_sdk_method defined
                if let Some(ref detail_method) = resource_def.detail_sdk_method {
                    // Build params from item data based on detail_sdk_method_params
                    let mut params = serde_json::Map::new();
                    if let Some(param_map) = resource_def.detail_sdk_method_params.as_object() {
                        for (param_name, field_name) in param_map {
                            if let Some(field) = field_name.as_str() {
                                let value = crate::resource::extract_json_value(&item, field);
                                params.insert(param_name.clone(), serde_json::Value::String(value));
                            }
                        }
                    }

                    // Call the detail SDK method
                    match crate::resource::invoke_sdk(
                        &resource_def.service,
                        detail_method,
                        &self.clients,
                        &serde_json::Value::Object(params),
                    )
                    .await
                    {
                        Ok(data) => {
                            self.describe_data = Some(data);
                        }
                        Err(e) => {
                            tracing::warn!(
                                "Failed to fetch detail data via {}: {}",
                                detail_method,
                                e
                            );
                            self.describe_data = Some(item);
                        }
                    }
                } else {
                    // Fall back to existing describe_resource logic
                    let id = crate::resource::extract_json_value(&item, &resource_def.id_field);
                    if id != "-" && !id.is_empty() {
                        // Collect parent params for REST path placeholders
                        let parent_params = self
                            .parent_context
                            .as_ref()
                            .map(|ctx| {
                                let mut params = std::collections::HashMap::new();
                                let sub = ctx.resource_key.clone();
                                if let Some(parent_resource) = crate::resource::get_resource(&sub) {
                                    let parent_id = crate::resource::extract_json_value(
                                        &ctx.item,
                                        &parent_resource.id_field,
                                    );
                                    if parent_id != "-" && !parent_id.is_empty() {
                                        params.insert(parent_resource.id_field.clone(), parent_id);
                                    }
                                }
                                params
                            })
                            .unwrap_or_default();

                        match crate::resource::describe_resource(
                            &self.current_resource_key,
                            &self.clients,
                            &id,
                            &parent_params,
                        )
                        .await
                        {
                            Ok(data) => {
                                self.describe_data = Some(data);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to fetch describe data: {}", e);
                                self.describe_data = Some(item);
                            }
                        }
                    }
                }
            }
        }
    }

    /// Enter confirmation mode for an action
    pub fn enter_confirm_mode(&mut self, pending: PendingAction) {
        self.pending_action = Some(pending);
        self.mode = Mode::Confirm;
    }

    /// Show a warning modal with OK button
    pub fn show_warning(&mut self, message: &str) {
        self.warning_message = Some(message.to_string());
        self.mode = Mode::Warning;
    }

    /// Enter SSO login mode to prompt for browser authentication
    pub fn enter_sso_login_mode(&mut self, profile: &str, sso_session: &str) {
        self.sso_state = Some(SsoLoginState::Prompt {
            profile: profile.to_string(),
            sso_session: sso_session.to_string(),
        });
        self.mode = Mode::SsoLogin;
    }

    pub fn enter_console_login_mode(&mut self, profile: &str, login_session: &str) {
        self.console_login_state = Some(ConsoleLoginState::Prompt {
            profile: profile.to_string(),
            login_session: login_session.to_string(),
        });
        self.mode = Mode::ConsoleLogin;
    }

    /// Create a pending action from an ActionDef
    pub fn create_pending_action(
        &self,
        action: &crate::resource::ActionDef,
        resource_id: &str,
    ) -> Option<PendingAction> {
        let config = action.get_confirm_config()?;
        let resource_name = self
            .selected_item()
            .and_then(|item| {
                if let Some(resource_def) = self.current_resource() {
                    let name = crate::resource::extract_json_value(item, &resource_def.name_field);
                    if name != "-" && !name.is_empty() {
                        return Some(name);
                    }
                }
                None
            })
            .unwrap_or_else(|| resource_id.to_string());

        let message = config
            .message
            .unwrap_or_else(|| action.display_name.clone());
        let default_no = !config.default_yes;

        Some(PendingAction {
            service: self.current_resource()?.service.clone(),
            sdk_method: action.sdk_method.clone(),
            resource_id: resource_id.to_string(),
            message: format!("{} '{}'?", message, resource_name),
            default_no,
            destructive: config.destructive,
            selected_yes: config.default_yes, // Start with default selection
        })
    }

    pub fn enter_profiles_mode(&mut self) {
        self.profiles_selected = self
            .available_profiles
            .iter()
            .position(|p| p == &self.profile)
            .unwrap_or(0);
        self.mode = Mode::Profiles;
    }

    pub fn enter_regions_mode(&mut self) {
        self.regions_selected = self
            .available_regions
            .iter()
            .position(|r| r == &self.region)
            .unwrap_or(0);
        self.mode = Mode::Regions;
    }

    pub fn exit_mode(&mut self) {
        self.mode = Mode::Normal;
        self.pending_action = None;
        self.describe_data = None; // Clear describe data when exiting
        self.last_action_display_name = None;
    }

    /// Copy the primary value from the current action result view to the clipboard.
    /// Only works when viewing an action result (e.g., after pressing 'x' to view a value).
    /// For SSM parameters, copies the Parameter Value.
    /// For Secrets Manager secrets, copies the SecretString.
    pub fn copy_describe_value_to_clipboard(&mut self) {
        // Only allow copy in action result views (not regular describe)
        if self.last_action_display_name.is_none() {
            return;
        }

        let Some(ref data) = self.describe_data else {
            self.error_message = Some("No data to copy".to_string());
            return;
        };

        let value_to_copy = extract_copyable_value(&self.current_resource_key, data);

        match value_to_copy {
            Some(text) if !text.is_empty() => {
                match arboard::Clipboard::new().and_then(|mut cb| cb.set_text(&text)) {
                    Ok(_) => {
                        let label = if self.current_resource_key == "ssm-parameters" {
                            "Parameter value"
                        } else {
                            "Secret value"
                        };
                        self.status_message = Some(format!("{} copied to clipboard", label));
                    }
                    Err(e) => {
                        self.error_message = Some(format!("Failed to copy to clipboard: {}", e));
                    }
                }
            }
            _ => {
                self.error_message = Some("No value found to copy".to_string());
            }
        }
    }

    // =========================================================================
    // Resource Navigation
    // =========================================================================

    /// Navigate to a resource (top-level)
    pub async fn navigate_to_resource(&mut self, resource_key: &str) -> Result<()> {
        if get_resource(resource_key).is_none() {
            self.error_message = Some(format!("Unknown resource: {}", resource_key));
            return Ok(());
        }

        // Clear parent context when navigating to top-level resource
        self.parent_context = None;
        self.navigation_stack.clear();
        self.current_resource_key = resource_key.to_string();
        self.selected = 0;
        self.filter_text.clear();
        self.filter_active = false;
        self.mode = Mode::Normal;
        // Column indices are per-resource, so a sort can't carry over.
        // Deliberately not in reset_pagination(), which 'R' refresh also calls.
        self.sort.reset();

        // Reset pagination for new resource
        self.reset_pagination();

        self.refresh_current().await?;
        Ok(())
    }

    /// Navigate to sub-resource with parent context
    pub async fn navigate_to_sub_resource(&mut self, sub_resource_key: &str) -> Result<()> {
        let Some(selected_item) = self.selected_item().cloned() else {
            return Ok(());
        };

        let Some(current_resource) = self.current_resource() else {
            return Ok(());
        };

        // Verify this is a valid sub-resource
        let is_valid = current_resource
            .sub_resources
            .iter()
            .any(|s| s.resource_key == sub_resource_key);

        if !is_valid {
            self.error_message = Some(format!(
                "{} is not a sub-resource of {}",
                sub_resource_key, self.current_resource_key
            ));
            return Ok(());
        }

        // Rows with no children (S3 files) have nothing to drill into
        if !is_navigable_row(&self.current_resource_key, sub_resource_key, &selected_item) {
            return Ok(());
        }

        // Get display name for parent
        let display_name = extract_json_value(&selected_item, &current_resource.name_field);
        let id = extract_json_value(&selected_item, &current_resource.id_field);
        let display = if display_name != "-" {
            display_name
        } else {
            id
        };

        // Push current context to stack
        if let Some(ctx) = self.parent_context.take() {
            self.navigation_stack.push(ctx);
        }

        // Set new parent context
        self.parent_context = Some(ParentContext {
            resource_key: self.current_resource_key.clone(),
            item: selected_item,
            display_name: display,
            saved_selected: self.selected,
        });

        // Navigate
        self.current_resource_key = sub_resource_key.to_string();
        self.selected = 0;
        self.filter_text.clear();
        self.filter_active = false;
        self.sort.reset();

        // Reset pagination for new resource
        self.reset_pagination();

        self.refresh_current().await?;
        Ok(())
    }

    /// Navigate back to parent resource
    pub async fn navigate_back(&mut self) -> Result<()> {
        if let Some(parent) = self.parent_context.take() {
            // Pop from navigation stack if available
            self.parent_context = self.navigation_stack.pop();

            // Navigate to parent resource
            self.current_resource_key = parent.resource_key;
            self.selected = parent.saved_selected;
            self.filter_text.clear();
            self.filter_active = false;
            self.sort.reset();

            // Reset pagination for parent resource
            self.reset_pagination();

            self.refresh_current().await?;
        }
        Ok(())
    }

    /// Get breadcrumb path
    pub fn get_breadcrumb(&self) -> Vec<String> {
        let mut path = Vec::new();

        for ctx in &self.navigation_stack {
            path.push(format!("{}:{}", ctx.resource_key, ctx.display_name));
        }

        if let Some(ctx) = &self.parent_context {
            path.push(format!("{}:{}", ctx.resource_key, ctx.display_name));
        }

        path.push(self.current_resource_key.clone());
        path
    }

    // =========================================================================
    // EC2 Actions (using SDK dispatcher)
    // =========================================================================
    // Profile/Region Switching
    // =========================================================================

    pub async fn switch_region(&mut self, region: &str) -> Result<()> {
        let actual_region = self.clients.switch_region(&self.profile, region).await?;
        self.region = actual_region.clone();

        // Save to config (log errors but don't fail region switch)
        if let Err(e) = self.config.set_region(&actual_region) {
            tracing::warn!("Failed to save region to config: {}", e);
        }

        Ok(())
    }

    pub async fn switch_profile(&mut self, profile: &str) -> Result<()> {
        let (new_clients, actual_region) =
            AwsClients::new(profile, &self.region, self.endpoint_url.clone()).await?;
        self.clients = new_clients;
        self.profile = profile.to_string();
        self.region = actual_region.clone();

        // Save to config (log errors but don't fail profile switch)
        if let Err(e) = self.config.set_profile(profile) {
            tracing::warn!("Failed to save profile to config: {}", e);
        }
        if let Err(e) = self.config.set_region(&actual_region) {
            tracing::warn!("Failed to save region to config: {}", e);
        }

        Ok(())
    }

    /// Switch profile with SSO/Console login check - returns login required if needed
    pub async fn switch_profile_with_sso_check(
        &mut self,
        profile: &str,
    ) -> Result<ProfileSwitchResult> {
        use crate::aws::client::ClientResult;

        match AwsClients::new_with_sso_check(profile, &self.region, self.endpoint_url.clone())
            .await?
        {
            ClientResult::Ok(new_clients, actual_region) => {
                self.clients = new_clients;
                self.profile = profile.to_string();
                self.region = actual_region.clone();

                // Save to config (log errors but don't fail profile switch)
                if let Err(e) = self.config.set_profile(profile) {
                    tracing::warn!("Failed to save profile to config: {}", e);
                }
                if let Err(e) = self.config.set_region(&actual_region) {
                    tracing::warn!("Failed to save region to config: {}", e);
                }

                Ok(ProfileSwitchResult::Success)
            }
            ClientResult::SsoLoginRequired {
                profile,
                sso_session,
                ..
            } => Ok(ProfileSwitchResult::SsoRequired {
                profile,
                sso_session,
            }),
            ClientResult::ConsoleLoginRequired {
                profile,
                login_session,
                ..
            } => Ok(ProfileSwitchResult::ConsoleLoginRequired {
                profile,
                login_session,
            }),
        }
    }

    /// Select profile - returns true if login (SSO or Console) is required
    pub async fn select_profile(&mut self) -> Result<bool> {
        if let Some(profile) = self.available_profiles.get(self.profiles_selected) {
            let profile = profile.clone();
            match self.switch_profile_with_sso_check(&profile).await? {
                ProfileSwitchResult::Success => {
                    self.refresh_current().await?;
                    self.exit_mode();
                    Ok(false)
                }
                ProfileSwitchResult::SsoRequired {
                    profile,
                    sso_session,
                } => {
                    // Enter SSO login mode (IAM Identity Center)
                    self.enter_sso_login_mode(&profile, &sso_session);
                    Ok(true)
                }
                ProfileSwitchResult::ConsoleLoginRequired {
                    profile,
                    login_session,
                } => {
                    // Enter console login mode (aws login)
                    self.enter_console_login_mode(&profile, &login_session);
                    Ok(true)
                }
            }
        } else {
            self.exit_mode();
            Ok(false)
        }
    }

    pub async fn select_region(&mut self) -> Result<()> {
        if let Some(region) = self.available_regions.get(self.regions_selected) {
            let region = region.clone();
            self.switch_region(&region).await?;
            self.refresh_current().await?;
        }
        self.exit_mode();
        Ok(())
    }

    // =========================================================================
    // Command Execution
    // =========================================================================

    pub async fn execute_command(&mut self) -> Result<bool> {
        // Use preview if user navigated to a suggestion, otherwise use typed text
        let command_text = if self.command_text.is_empty() {
            self.command_preview.clone().unwrap_or_default()
        } else if let Some(preview) = &self.command_preview {
            // If preview matches what would be completed, use preview
            if preview.contains(&self.command_text) {
                preview.clone()
            } else {
                self.command_text.clone()
            }
        } else {
            self.command_text.clone()
        };

        let parts: Vec<&str> = command_text.split_whitespace().collect();

        if parts.is_empty() {
            return Ok(false);
        }

        let cmd = parts[0];

        match cmd {
            "q" | "quit" => return Ok(true),
            "back" => {
                self.navigate_back().await?;
            }
            "profiles" => {
                self.enter_profiles_mode();
            }
            "regions" => {
                self.enter_regions_mode();
            }
            "region" if parts.len() > 1 => {
                self.switch_region(parts[1]).await?;
                self.refresh_current().await?;
            }
            "profile" if parts.len() > 1 => {
                self.switch_profile(parts[1]).await?;
                self.refresh_current().await?;
            }
            _ => {
                // Check if it's a known resource
                if let Some(target_resource) = get_resource(cmd) {
                    // Check if the target resource requires a parent
                    if target_resource.requires_parent {
                        // Check if it's a sub-resource of current and we have a selected item
                        if let Some(resource) = self.current_resource() {
                            let is_sub =
                                resource.sub_resources.iter().any(|s| s.resource_key == cmd);
                            if is_sub && self.selected_item().is_some() {
                                self.navigate_to_sub_resource(cmd).await?;
                            } else {
                                self.error_message = Some(format!(
                                    "'{}' requires a parent resource. Navigate to the parent first and select an item.",
                                    target_resource.display_name
                                ));
                            }
                        } else {
                            self.error_message = Some(format!(
                                "'{}' requires a parent resource. Navigate to the parent first and select an item.",
                                target_resource.display_name
                            ));
                        }
                    } else {
                        // Normal resource - check if it's a sub-resource of current
                        if let Some(resource) = self.current_resource() {
                            let is_sub =
                                resource.sub_resources.iter().any(|s| s.resource_key == cmd);
                            if is_sub && self.selected_item().is_some() {
                                self.navigate_to_sub_resource(cmd).await?;
                            } else {
                                self.navigate_to_resource(cmd).await?;
                            }
                        } else {
                            self.navigate_to_resource(cmd).await?;
                        }
                    }
                } else {
                    self.error_message = Some(format!("Unknown command: {}", cmd));
                }
            }
        }

        Ok(false)
    }

    // =========================================================================
    // Log Tail Mode
    // =========================================================================

    /// Enter log tail mode for the selected log stream
    pub async fn enter_log_tail_mode(&mut self) -> Result<()> {
        // Get the selected log stream item
        let Some(item) = self.selected_item().cloned() else {
            return Ok(());
        };

        // Extract log stream name from selected item
        let log_stream = extract_json_value(&item, "logStreamName");

        // Extract log group name from parent context (log group)
        let log_group = self
            .parent_context
            .as_ref()
            .map(|ctx| extract_json_value(&ctx.item, "logGroupName"))
            .unwrap_or_else(|| "-".to_string());

        if log_group == "-" || log_stream == "-" {
            self.error_message = Some("Could not get log group/stream name".to_string());
            return Ok(());
        }

        // Initialize log tail state
        self.log_tail_state = Some(LogTailState {
            log_group: log_group.clone(),
            log_stream: log_stream.clone(),
            events: Vec::new(),
            scroll: 0,
            next_forward_token: None,
            auto_scroll: true,
            paused: false,
            last_poll: std::time::Instant::now(),
            error: None,
        });

        self.mode = Mode::LogTail;

        // Fetch initial log events
        self.poll_log_events().await?;

        Ok(())
    }

    /// Poll for new log events
    pub async fn poll_log_events(&mut self) -> Result<()> {
        let Some(ref mut state) = self.log_tail_state else {
            return Ok(());
        };

        if state.paused {
            return Ok(());
        }

        // Build params for get_log_events
        let mut params = serde_json::json!({
            "log_group_name": [state.log_group.clone()],
            "log_stream_name": [state.log_stream.clone()],
        });

        if let Some(ref token) = state.next_forward_token {
            params["next_forward_token"] = serde_json::json!(token);
        }

        // Call the SDK
        match crate::resource::invoke_sdk(
            "cloudwatchlogs",
            "get_log_events",
            &self.clients,
            &params,
        )
        .await
        {
            Ok(response) => {
                state.error = None;

                // Extract events
                if let Some(events) = response
                    .get("events")
                    .and_then(|v: &serde_json::Value| v.as_array())
                {
                    for event in events {
                        let timestamp = event
                            .get("timestamp")
                            .and_then(|v: &serde_json::Value| v.as_i64())
                            .unwrap_or(0);
                        let message = event
                            .get("message")
                            .and_then(|v: &serde_json::Value| v.as_str())
                            .unwrap_or("")
                            .to_string();

                        state.events.push(LogEvent { timestamp, message });
                    }

                    // Keep only last 1000 events
                    if state.events.len() > 1000 {
                        let drain_count = state.events.len() - 1000;
                        state.events.drain(0..drain_count);
                    }
                }

                // Update next forward token
                if let Some(token) = response.get("nextForwardToken").and_then(|v| v.as_str()) {
                    state.next_forward_token = Some(token.to_string());
                }

                // Auto-scroll to bottom if enabled
                if state.auto_scroll && !state.events.is_empty() {
                    state.scroll = state.events.len().saturating_sub(1);
                }
            }
            Err(e) => {
                state.error = Some(format!("Failed to fetch logs: {}", e));
            }
        }

        state.last_poll = std::time::Instant::now();
        Ok(())
    }

    /// Toggle pause state for log tailing
    pub fn toggle_log_tail_pause(&mut self) {
        if let Some(ref mut state) = self.log_tail_state {
            state.paused = !state.paused;
        }
    }

    /// Scroll log tail view up
    pub fn log_tail_scroll_up(&mut self, amount: usize) {
        if let Some(ref mut state) = self.log_tail_state {
            state.scroll = state.scroll.saturating_sub(amount);
            state.auto_scroll = false;
        }
    }

    /// Scroll log tail view down
    pub fn log_tail_scroll_down(&mut self, amount: usize) {
        if let Some(ref mut state) = self.log_tail_state {
            let max_scroll = state.events.len().saturating_sub(1);
            state.scroll = (state.scroll + amount).min(max_scroll);
        }
    }

    /// Scroll log tail view to top
    pub fn log_tail_scroll_to_top(&mut self) {
        if let Some(ref mut state) = self.log_tail_state {
            state.scroll = 0;
            state.auto_scroll = false;
        }
    }

    /// Scroll log tail view to bottom and enable auto-scroll
    pub fn log_tail_scroll_to_bottom(&mut self) {
        if let Some(ref mut state) = self.log_tail_state {
            state.scroll = state.events.len().saturating_sub(1);
            state.auto_scroll = true;
        }
    }

    /// Exit log tail mode
    pub fn exit_log_tail_mode(&mut self) {
        self.log_tail_state = None;
        self.mode = Mode::Normal;
    }

    // =========================================================================
    // SSM Connect
    // =========================================================================

    /// Request SSM connect to the selected EC2 instance
    /// Returns true if a connect request was made, false otherwise
    pub fn request_ssm_connect(&mut self) -> bool {
        // Get the selected item
        let Some(item) = self.selected_item().cloned() else {
            return false;
        };

        // Extract instance ID
        let instance_id = extract_json_value(&item, "InstanceId");
        if instance_id == "-" || instance_id.is_empty() {
            self.show_warning("Could not get instance ID");
            return false;
        }

        // Check if instance is running
        let state = extract_json_value(&item, "State");
        if state != "running" {
            self.show_warning(&format!(
                "Cannot connect: instance is '{}'. Instance must be running.",
                state
            ));
            return false;
        }

        // Check if session-manager-plugin is installed
        if !Self::is_ssm_plugin_installed() {
            self.show_warning("session-manager-plugin is not installed.\n\nhttps://docs.aws.amazon.com/systems-manager/latest/userguide/session-manager-working-with-install-plugin.html");
            return false;
        }

        // Set the connect request - will be handled by main loop
        self.ssm_connect_request = Some(SsmConnectRequest {
            instance_id,
            region: self.region.clone(),
            profile: self.profile.clone(),
        });

        true
    }

    /// Check if session-manager-plugin is installed
    fn is_ssm_plugin_installed() -> bool {
        std::process::Command::new("session-manager-plugin")
            .arg("--version")
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    /// Take the SSM connect request (clears it)
    pub fn take_ssm_connect_request(&mut self) -> Option<SsmConnectRequest> {
        self.ssm_connect_request.take()
    }

    // =========================================================================
    // S3 object browsing
    // =========================================================================

    /// Handle Enter: drill into the selected row where the resource opts into it
    /// (S3 buckets and folders), otherwise open the details panel as before.
    pub async fn enter_primary_action(&mut self) -> Result<()> {
        let action = match (self.current_resource(), self.selected_item()) {
            (Some(resource), Some(item)) => enter_action(
                resource.enter_sub_resource.as_deref(),
                &self.current_resource_key,
                item,
            ),
            _ => EnterAction::Describe,
        };

        match action {
            EnterAction::Navigate(sub_resource_key) => {
                self.navigate_to_sub_resource(&sub_resource_key).await
            }
            EnterAction::Describe => {
                self.enter_describe_mode().await;
                Ok(())
            }
        }
    }

    /// Download the selected S3 object to the user's Downloads directory.
    ///
    /// Runs on the event-loop thread, so the UI is unresponsive until it finishes.
    /// `MAX_DOWNLOAD_BYTES` keeps that pause bounded.
    pub async fn download_selected_object(&mut self) {
        let Some(item) = self.selected_item().cloned() else {
            return;
        };

        if item
            .get("IsFolder")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            self.error_message = Some("Folders cannot be downloaded".to_string());
            return;
        }

        let key = extract_json_value(&item, "Key");
        let size_bytes = item.get("SizeBytes").and_then(|v| v.as_u64()).unwrap_or(0);
        if let Some(error) = download_size_error(size_bytes) {
            self.error_message = Some(error);
            return;
        }

        let Some(bucket) =
            s3_bucket_from_context(&self.navigation_stack, self.parent_context.as_ref())
        else {
            self.error_message =
                Some("Cannot tell which bucket this object belongs to".to_string());
            return;
        };

        let path = match resolve_download_path(&download_dir(), &key) {
            Ok(path) => path,
            Err(e) => {
                self.error_message = Some(e.to_string());
                return;
            }
        };

        self.status_message = Some(format!("Downloading {}...", key));

        match Self::fetch_object_to_file(&self.clients, &bucket, &key, &path).await {
            Ok(bytes) => {
                self.status_message = Some(format!("Saved {} bytes to {}", bytes, path.display()));
            }
            Err(e) => {
                self.status_message = None;
                self.error_message = Some(format!("Download failed: {}", e));
            }
        }
    }

    /// Fetch an object and write it to disk, refusing to replace an existing file.
    async fn fetch_object_to_file(
        clients: &AwsClients,
        bucket: &str,
        key: &str,
        path: &Path,
    ) -> Result<usize> {
        let region = clients.http.get_bucket_region(bucket).await?;
        let bytes = clients.http.get_s3_object(bucket, key, &region).await?;

        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        // create_new so a file that appeared since the earlier check still isn't clobbered
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)?;
        std::io::Write::write_all(&mut file, &bytes)?;

        Ok(bytes.len())
    }
}

/// Where downloads land. Falls back to ~/Downloads, then the working directory.
fn download_dir() -> PathBuf {
    dirs::download_dir()
        .or_else(|| dirs::home_dir().map(|home| home.join("Downloads")))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Largest object we will pull down. Downloads block the event loop, so an
/// unbounded fetch would freeze the TUI with no progress bar and no way to cancel.
const MAX_DOWNLOAD_BYTES: u64 = 2 * 1024 * 1024 * 1024; // 2 GB

/// What Enter should do on the selected row.
#[derive(Debug, Clone, PartialEq)]
pub enum EnterAction {
    /// Drill into this sub-resource
    Navigate(String),
    /// Open the details panel (the historical behaviour of Enter)
    Describe,
}

/// Whether the selected row has children to drill into.
///
/// S3 lists folders and files side by side under one resource, so a
/// self-referential sub-resource can only descend on the folder rows.
fn is_navigable_row(current_key: &str, sub_key: &str, item: &Value) -> bool {
    if current_key == "s3-objects" && sub_key == "s3-objects" {
        return item
            .get("IsFolder")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
    }
    true
}

/// Route Enter: drill in where the resource opted into it via `enter_sub_resource`,
/// otherwise fall back to the details panel. A file in S3 lands on Describe, so
/// Enter is never a dead key.
fn enter_action(enter_sub_resource: Option<&str>, current_key: &str, item: &Value) -> EnterAction {
    match enter_sub_resource {
        Some(sub) if is_navigable_row(current_key, sub, item) => {
            EnterAction::Navigate(sub.to_string())
        }
        _ => EnterAction::Describe,
    }
}

/// Find the bucket that owns the current object listing.
///
/// Nested folders stack more `s3-objects` contexts on top of the bucket, so walk
/// from the oldest context down to the current one and take the bucket entry.
fn s3_bucket_from_context(
    stack: &[ParentContext],
    current: Option<&ParentContext>,
) -> Option<String> {
    stack
        .iter()
        .chain(current)
        .find(|ctx| ctx.resource_key == "s3-buckets")
        .and_then(|ctx| ctx.item.get("Name"))
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
}

/// Local filename for an S3 key, or None when the key names no file.
///
/// Only the last path segment is kept, so a key crafted with `..` segments cannot
/// walk out of the download directory.
fn download_filename(key: &str) -> Option<String> {
    let name = key.rsplit('/').next().unwrap_or("");
    if name.is_empty() || name == "." || name == ".." {
        return None;
    }
    Some(name.to_string())
}

/// Refuse objects too large to fetch on the UI thread.
fn download_size_error(size_bytes: u64) -> Option<String> {
    if size_bytes <= MAX_DOWNLOAD_BYTES {
        return None;
    }
    Some(format!(
        "Object is {} MB - too large to download (limit {} MB)",
        size_bytes / (1024 * 1024),
        MAX_DOWNLOAD_BYTES / (1024 * 1024)
    ))
}

/// Where to write an object, erroring rather than overwriting an existing file.
fn resolve_download_path(dir: &Path, key: &str) -> Result<PathBuf> {
    let filename = download_filename(key)
        .ok_or_else(|| anyhow::anyhow!("Cannot work out a filename for '{}'", key))?;
    let path = dir.join(filename);
    if path.exists() {
        return Err(anyhow::anyhow!("{} already exists", path.display()));
    }
    Ok(path)
}

/// Extract the copyable value from describe data based on resource type.
/// Returns None for unsupported resource types.
fn extract_copyable_value(resource_key: &str, data: &Value) -> Option<String> {
    match resource_key {
        "ssm-parameters" => {
            // SSM GetParameter response: { "Parameter": { "Value": "..." } }
            data.pointer("/Parameter/Value")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
        "secretsmanager-secrets" => {
            // GetSecretValue response: { "SecretString": "..." }
            data.pointer("/SecretString")
                .and_then(|v| v.as_str())
                .map(|s| s.to_string())
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_aws_filters_valid() {
        let result = AwsFilters::parse("Filters: owner=amazon, architecture=arm64");
        assert!(result.is_some());
        let filters = result.unwrap();
        assert_eq!(filters.filters.len(), 2);
        assert_eq!(
            filters.filters[0],
            ("owner".to_string(), "amazon".to_string())
        );
        assert_eq!(
            filters.filters[1],
            ("architecture".to_string(), "arm64".to_string())
        );
    }

    #[test]
    fn test_parse_aws_filters_lowercase() {
        let result = AwsFilters::parse("filters: state=available");
        assert!(result.is_some());
        let filters = result.unwrap();
        assert_eq!(filters.filters.len(), 1);
        assert_eq!(
            filters.filters[0],
            ("state".to_string(), "available".to_string())
        );
    }

    #[test]
    fn test_parse_aws_filters_with_tag() {
        let result = AwsFilters::parse("Filters: tag:Environment=prod");
        assert!(result.is_some());
        let filters = result.unwrap();
        assert_eq!(filters.filters.len(), 1);
        assert_eq!(
            filters.filters[0],
            ("tag:Environment".to_string(), "prod".to_string())
        );
    }

    #[test]
    fn test_parse_aws_filters_invalid_no_value() {
        let result = AwsFilters::parse("Filters: owner=");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_aws_filters_invalid_no_key() {
        let result = AwsFilters::parse("Filters: =amazon");
        assert!(result.is_none());
    }

    #[test]
    fn test_parse_aws_filters_not_filters_prefix() {
        let result = AwsFilters::parse("owner=amazon");
        assert!(result.is_none());
    }

    #[test]
    fn test_aws_filters_display() {
        let filters = AwsFilters {
            filters: vec![
                ("owner".to_string(), "amazon".to_string()),
                ("architecture".to_string(), "arm64".to_string()),
            ],
        };
        assert_eq!(
            filters.display(),
            "Filters: owner=amazon, architecture=arm64"
        );
    }

    #[test]
    fn test_extract_copyable_value_ssm_parameter() {
        let data = serde_json::json!({
            "Parameter": {
                "Name": "/app/config-key",
                "Type": "SecureString",
                "Value": "fake-data",
                "Version": 1
            }
        });
        let result = extract_copyable_value("ssm-parameters", &data);
        assert_eq!(result, Some("fake-data".to_string()));
    }

    #[test]
    fn test_extract_copyable_value_secretsmanager() {
        let data = serde_json::json!({
            "ARN": "arn:aws:secretsmanager:us-east-1:123456789:secret:my-secret",
            "Name": "my-secret",
            "SecretString": "{\"key\":\"fake-data\"}",
            "VersionId": "abc-123"
        });
        let result = extract_copyable_value("secretsmanager-secrets", &data);
        assert_eq!(result, Some("{\"key\":\"fake-data\"}".to_string()));
    }

    #[test]
    fn test_extract_copyable_value_unsupported_resource() {
        let data = serde_json::json!({"InstanceId": "i-12345"});
        let result = extract_copyable_value("ec2-instances", &data);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_copyable_value_ssm_missing_value_field() {
        let data = serde_json::json!({
            "Parameter": {
                "Name": "/app/config",
                "Type": "String"
            }
        });
        let result = extract_copyable_value("ssm-parameters", &data);
        assert_eq!(result, None);
    }

    #[test]
    fn test_extract_copyable_value_secret_missing_secret_string() {
        let data = serde_json::json!({
            "ARN": "arn:aws:secretsmanager:us-east-1:123456789:secret:binary-secret",
            "Name": "binary-secret",
            "SecretBinary": "base64data"
        });
        let result = extract_copyable_value("secretsmanager-secrets", &data);
        assert_eq!(result, None);
    }

    // =========================================================================
    // Sorting
    // =========================================================================

    /// A sort active on `column` in the given direction, cursor parked on it
    fn sorted_on(column: usize, descending: bool) -> SortState {
        SortState {
            cursor: column,
            column: Some(column),
            descending,
        }
    }

    #[test]
    fn test_sort_state_cursor_starts_on_first_column_unsorted() {
        let sort = SortState::default();
        assert_eq!(sort.cursor, 0);
        assert_eq!(sort.column, None);
    }

    #[test]
    fn test_sort_state_cursor_right_moves_without_sorting() {
        let mut sort = SortState::default();
        sort.cursor_right(4);
        assert_eq!(sort.cursor, 1);
        assert_eq!(
            sort.column, None,
            "moving the cursor must not start a sort on its own"
        );
    }

    #[test]
    fn test_sort_state_cursor_right_wraps_to_first() {
        let mut sort = SortState {
            cursor: 3,
            ..SortState::default()
        };
        sort.cursor_right(4);
        assert_eq!(sort.cursor, 0);
    }

    #[test]
    fn test_sort_state_cursor_left_wraps_to_last() {
        let mut sort = SortState::default();
        sort.cursor_left(4);
        assert_eq!(sort.cursor, 3);
    }

    #[test]
    fn test_sort_state_cursor_moves_leave_active_sort_alone() {
        let mut sort = sorted_on(0, true);
        sort.cursor_right(4);
        sort.cursor_right(4);
        assert_eq!(sort.cursor, 2);
        assert_eq!(sort.column, Some(0), "the sorted column must not change");
        assert!(sort.descending);
    }

    #[test]
    fn test_sort_state_cursor_moves_are_noop_when_no_columns() {
        let mut sort = SortState::default();
        sort.cursor_right(0);
        sort.cursor_left(0);
        assert_eq!(sort.cursor, 0);
        assert_eq!(sort.column, None);
    }

    #[test]
    fn test_sort_state_sort_by_cursor_starts_ascending() {
        let mut sort = SortState {
            cursor: 3,
            ..SortState::default()
        };
        sort.sort_by_cursor();
        assert_eq!(sort.column, Some(3));
        assert!(!sort.descending);
    }

    #[test]
    fn test_sort_state_sort_by_cursor_flips_when_already_sorted() {
        let mut sort = sorted_on(2, false);
        sort.sort_by_cursor();
        assert!(sort.descending);
        sort.sort_by_cursor();
        assert!(!sort.descending);
    }

    #[test]
    fn test_sort_state_sort_by_cursor_on_new_column_restarts_ascending() {
        let mut sort = sorted_on(0, true);
        sort.cursor = 2;
        sort.sort_by_cursor();
        assert_eq!(sort.column, Some(2));
        assert!(
            !sort.descending,
            "a newly picked column should start ascending, not inherit descending"
        );
    }

    #[test]
    fn test_sort_state_clear_resets_sort_but_keeps_cursor() {
        let mut sort = sorted_on(2, true);
        sort.clear();
        assert_eq!(sort.column, None);
        assert!(!sort.descending);
        assert_eq!(sort.cursor, 2, "Shift+Tab clears the sort, not your place");
    }

    #[test]
    fn test_sort_state_reset_clears_cursor_too() {
        let mut sort = sorted_on(2, true);
        sort.reset();
        assert_eq!(sort.column, None);
        assert_eq!(sort.cursor, 0);
    }

    #[test]
    fn test_sort_state_indicator_reflects_direction() {
        assert_eq!(sorted_on(0, false).indicator(), "↑");
        assert_eq!(sorted_on(0, true).indicator(), "↓");
    }

    #[test]
    fn test_compare_cells_is_case_insensitive() {
        assert_eq!(compare_cells("apple", "Banana", false), Ordering::Less);
        assert_eq!(compare_cells("Banana", "apple", false), Ordering::Greater);
    }

    #[test]
    fn test_compare_cells_numeric_not_lexicographic() {
        // Lexicographic comparison would wrongly put "10" before "9"
        assert_eq!(compare_cells("9", "10", false), Ordering::Less);
        assert_eq!(compare_cells("10", "9", false), Ordering::Greater);
    }

    #[test]
    fn test_compare_cells_descending_reverses_values() {
        assert_eq!(compare_cells("apple", "banana", true), Ordering::Greater);
    }

    #[test]
    fn test_compare_cells_blanks_sort_last_ascending() {
        assert_eq!(compare_cells("-", "apple", false), Ordering::Greater);
        assert_eq!(compare_cells("", "apple", false), Ordering::Greater);
    }

    #[test]
    fn test_compare_cells_blanks_sort_last_descending() {
        // Blanks stay at the bottom in both directions so they never crowd the top
        assert_eq!(compare_cells("-", "apple", true), Ordering::Greater);
        assert_eq!(compare_cells("apple", "-", true), Ordering::Less);
    }

    #[test]
    fn test_compare_cells_two_blanks_are_equal() {
        assert_eq!(compare_cells("-", "", false), Ordering::Equal);
    }

    fn role(name: &str, created: &str) -> Value {
        serde_json::json!({ "RoleName": name, "CreateDate": created })
    }

    fn names(items: &[Value]) -> Vec<String> {
        items
            .iter()
            .map(|i| extract_json_value(i, "RoleName"))
            .collect()
    }

    #[test]
    fn test_sort_items_ascending_by_name() {
        let mut items = vec![
            role("charlie", "2024-01-01T00:00:00Z"),
            role("alpha", "2025-01-01T00:00:00Z"),
            role("bravo", "2023-01-01T00:00:00Z"),
        ];
        sort_items(&mut items, "RoleName", false, 0);
        assert_eq!(names(&items), vec!["alpha", "bravo", "charlie"]);
    }

    #[test]
    fn test_sort_items_descending_by_iso8601_date() {
        let mut items = vec![
            role("charlie", "2024-01-01T00:00:00Z"),
            role("alpha", "2025-11-13T12:17:33Z"),
            role("bravo", "2023-01-01T00:00:00Z"),
        ];
        sort_items(&mut items, "CreateDate", true, 0);
        assert_eq!(names(&items), vec!["alpha", "charlie", "bravo"]);
    }

    #[test]
    fn test_sort_items_keeps_api_order_for_ties() {
        let mut items = vec![
            role("zulu", "2024-01-01T00:00:00Z"),
            role("yankee", "2024-01-01T00:00:00Z"),
            role("xray", "2024-01-01T00:00:00Z"),
        ];
        sort_items(&mut items, "CreateDate", false, 0);
        assert_eq!(
            names(&items),
            vec!["zulu", "yankee", "xray"],
            "equal keys must preserve original API order"
        );
    }

    #[test]
    fn test_sort_items_missing_values_go_last() {
        let mut items = vec![
            serde_json::json!({ "RoleName": "no-date" }),
            role("bravo", "2023-01-01T00:00:00Z"),
            role("alpha", "2025-01-01T00:00:00Z"),
        ];
        sort_items(&mut items, "CreateDate", false, 0);
        assert_eq!(names(&items), vec!["bravo", "alpha", "no-date"]);
    }

    #[test]
    fn test_sort_items_returns_new_index_of_selected_item() {
        let mut items = vec![
            role("charlie", "2024-01-01T00:00:00Z"),
            role("alpha", "2025-01-01T00:00:00Z"),
            role("bravo", "2023-01-01T00:00:00Z"),
        ];
        // "bravo" is selected at index 2; after sorting by name it lands at index 1
        let new_index = sort_items(&mut items, "RoleName", false, 2);
        assert_eq!(new_index, 1);
    }

    #[test]
    fn test_sort_items_selection_falls_back_to_top_when_out_of_range() {
        let mut items = vec![role("alpha", "2025-01-01T00:00:00Z")];
        let new_index = sort_items(&mut items, "RoleName", false, 7);
        assert_eq!(new_index, 0);
    }

    #[test]
    fn test_sort_items_on_empty_list_returns_zero() {
        let mut items: Vec<Value> = Vec::new();
        let new_index = sort_items(&mut items, "RoleName", false, 0);
        assert_eq!(new_index, 0);
    }

    // === Enter key routing ===

    fn folder_row(key: &str) -> Value {
        serde_json::json!({ "Key": key, "IsFolder": true })
    }

    fn file_row(key: &str) -> Value {
        serde_json::json!({ "Key": key, "IsFolder": false })
    }

    #[test]
    fn test_enter_action_navigates_when_resource_opts_in() {
        let item = serde_json::json!({ "Name": "my-bucket" });
        assert_eq!(
            enter_action(Some("s3-objects"), "s3-buckets", &item),
            EnterAction::Navigate("s3-objects".to_string())
        );
    }

    #[test]
    fn test_enter_action_describes_when_resource_has_no_enter_target() {
        let item = serde_json::json!({ "RoleName": "admin" });
        assert_eq!(
            enter_action(None, "iam-roles", &item),
            EnterAction::Describe
        );
    }

    #[test]
    fn test_enter_action_descends_into_s3_folder() {
        assert_eq!(
            enter_action(Some("s3-objects"), "s3-objects", &folder_row("logs/")),
            EnterAction::Navigate("s3-objects".to_string())
        );
    }

    #[test]
    fn test_enter_action_on_s3_file_falls_back_to_describe() {
        assert_eq!(
            enter_action(Some("s3-objects"), "s3-objects", &file_row("logs/app.log")),
            EnterAction::Describe
        );
    }

    #[test]
    fn test_is_navigable_row_allows_non_self_referential_targets() {
        let item = serde_json::json!({ "VpcId": "vpc-123" });
        assert!(is_navigable_row("vpc", "vpc-subnets", &item));
    }

    #[test]
    fn test_is_navigable_row_rejects_s3_file() {
        assert!(!is_navigable_row(
            "s3-objects",
            "s3-objects",
            &file_row("a.txt")
        ));
    }

    #[test]
    fn test_is_navigable_row_rejects_s3_row_missing_folder_flag() {
        let item = serde_json::json!({ "Key": "a.txt" });
        assert!(!is_navigable_row("s3-objects", "s3-objects", &item));
    }

    // === S3 download ===

    fn parent_ctx(resource_key: &str, item: Value) -> ParentContext {
        ParentContext {
            resource_key: resource_key.to_string(),
            item,
            display_name: "ctx".to_string(),
            saved_selected: 0,
        }
    }

    #[test]
    fn test_bucket_from_context_reads_immediate_parent() {
        let current = parent_ctx("s3-buckets", serde_json::json!({ "Name": "my-bucket" }));
        assert_eq!(
            s3_bucket_from_context(&[], Some(&current)),
            Some("my-bucket".to_string())
        );
    }

    #[test]
    fn test_bucket_from_context_reads_through_nested_folders() {
        let stack = vec![
            parent_ctx("s3-buckets", serde_json::json!({ "Name": "my-bucket" })),
            parent_ctx("s3-objects", folder_row("logs/")),
        ];
        let current = parent_ctx("s3-objects", folder_row("logs/2026/"));
        assert_eq!(
            s3_bucket_from_context(&stack, Some(&current)),
            Some("my-bucket".to_string())
        );
    }

    #[test]
    fn test_bucket_from_context_none_without_s3_ancestor() {
        let current = parent_ctx("vpc", serde_json::json!({ "VpcId": "vpc-123" }));
        assert_eq!(s3_bucket_from_context(&[], Some(&current)), None);
    }

    #[test]
    fn test_bucket_from_context_none_at_top_level() {
        assert_eq!(s3_bucket_from_context(&[], None), None);
    }

    #[test]
    fn test_download_filename_strips_key_prefix() {
        assert_eq!(
            download_filename("logs/2026/app.log"),
            Some("app.log".to_string())
        );
    }

    #[test]
    fn test_download_filename_key_without_prefix() {
        assert_eq!(download_filename("app.log"), Some("app.log".to_string()));
    }

    #[test]
    fn test_download_filename_rejects_folder_key() {
        assert_eq!(download_filename("logs/"), None);
    }

    #[test]
    fn test_download_filename_rejects_empty_key() {
        assert_eq!(download_filename(""), None);
    }

    #[test]
    fn test_download_filename_rejects_relative_path_segments() {
        // S3 keys are arbitrary strings, so ".." must never become a directory hop
        assert_eq!(download_filename("logs/.."), None);
        assert_eq!(download_filename(".."), None);
        assert_eq!(download_filename("."), None);
    }

    #[test]
    fn test_download_filename_keeps_traversal_attempt_inside_target_dir() {
        assert_eq!(
            download_filename("../../etc/passwd"),
            Some("passwd".to_string())
        );
    }

    #[test]
    fn test_download_size_error_allows_size_at_limit() {
        assert_eq!(download_size_error(MAX_DOWNLOAD_BYTES), None);
    }

    #[test]
    fn test_download_size_error_rejects_oversized_object() {
        let error = download_size_error(MAX_DOWNLOAD_BYTES + 1);
        assert!(error.is_some(), "objects above the cap must be refused");
        assert!(
            error.unwrap().contains("2048"),
            "message should name the limit so the user knows why"
        );
    }

    #[test]
    fn test_resolve_download_path_joins_dir_and_filename() {
        let dir = std::env::temp_dir().join(format!("orbit-dl-{}", std::process::id()));
        let path = resolve_download_path(&dir, "logs/app.log").unwrap();
        assert_eq!(path, dir.join("app.log"));
    }

    #[test]
    fn test_resolve_download_path_refuses_to_overwrite() {
        let dir = std::env::temp_dir().join(format!("orbit-overwrite-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let existing = dir.join("app.log");
        std::fs::write(&existing, b"keep me").unwrap();

        let result = resolve_download_path(&dir, "logs/app.log");

        std::fs::remove_dir_all(&dir).ok();
        assert!(result.is_err(), "must not clobber an existing file");
    }

    #[test]
    fn test_resolve_download_path_rejects_unusable_key() {
        let dir = std::env::temp_dir();
        assert!(resolve_download_path(&dir, "logs/").is_err());
    }
}
