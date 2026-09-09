//! Billing dashboards: composite pages assembled from several API calls.
//!
//! A dashboard is a new kind of view (unlike the one-resource-per-list views)
//! driven by its own JSON definition: each panel declares named fetches and a
//! `kind` selects a Rust renderer/computor. Adding a dashboard needs no new
//! Rust; the four kinds here are capabilities, the same split the resource
//! JSONs use for transforms.

use crate::aws::client::AwsClients;
use crate::resource::handlers::get_protocol_handler;
use crate::resource::protocol::{ApiConfig, ApiProtocol};
use anyhow::Result;
use chrono::Datelike;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// =============================================================================
// Definition (parsed from dashboards JSON)
// =============================================================================

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardFile {
    pub dashboards: HashMap<String, DashboardDef>,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardDef {
    pub display_name: String,
    #[serde(default)]
    pub panels: Vec<DashboardPanel>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DashboardPanel {
    pub kind: PanelKind,
    pub title: String,
    #[serde(default)]
    pub fetches: HashMap<String, PanelFetch>,
    /// Cost-table window length in months. Only read by the cost_table kind.
    #[serde(default)]
    pub months: Option<u32>,
    /// Cost-table grouping (dimension / tag / cost category). Only read by
    /// the cost_table kind.
    #[serde(default)]
    pub group_by: Option<GroupBySpec>,
    /// A default-hidden panel stays off the page until the panel picker
    /// shows it; the picker's choice is remembered in the user config.
    #[serde(default)]
    pub default_hidden: bool,
}

/// A custom panel as the user writes it in `~/.orbit/config.yaml`. Declarative
/// on purpose: the fetch is generated from window + group-by, so users never
/// touch the fetch machinery. Currently only cost_table panels are supported.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CustomPanel {
    pub title: String,
    pub kind: PanelKind,
    #[serde(default = "default_months")]
    pub months: u32,
    pub group_by: GroupBySpec,
}

fn default_months() -> u32 {
    3
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupByType {
    Dimension,
    Tag,
    CostCategory,
}

/// What to group a cost table by. `key` is the dimension name (SERVICE,
/// LINKED_ACCOUNT, ...), the tag key, or the cost category name.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupBySpec {
    #[serde(rename = "type")]
    pub group_type: GroupByType,
    pub key: String,
}

impl<'de> Deserialize<'de> for GroupByType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "dimension" => Ok(GroupByType::Dimension),
            "tag" => Ok(GroupByType::Tag),
            "cost_category" => Ok(GroupByType::CostCategory),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &["dimension", "tag", "cost_category"],
            )),
        }
    }
}

impl Serialize for GroupByType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        let s = match self {
            GroupByType::Dimension => "dimension",
            GroupByType::Tag => "tag",
            GroupByType::CostCategory => "cost_category",
        };
        serializer.serialize_str(s)
    }
}

impl GroupBySpec {
    /// The GetCostAndUsage GroupBy entry this spec describes.
    fn to_group_by(&self) -> Value {
        let api_type = match self.group_type {
            GroupByType::Dimension => "DIMENSION",
            GroupByType::Tag => "TAG",
            GroupByType::CostCategory => "COST_CATEGORY",
        };
        serde_json::json!([{ "Type": api_type, "Key": self.key }])
    }
}

impl CustomPanel {
    /// Turn a user-defined panel into the same shape the JSON dashboards use.
    /// Only cost_table is supported: the other kinds are hardcoded computors
    /// whose definitions would not survive user editing.
    pub fn to_dashboard_panel(&self) -> Result<DashboardPanel> {
        if self.kind != PanelKind::CostTable {
            return Err(anyhow::anyhow!(
                "custom panel '{}' uses kind '{}': user-defined panels currently support only 'cost_table'",
                self.title,
                self.kind.as_str()
            ));
        }
        Ok(DashboardPanel {
            kind: self.kind,
            title: self.title.clone(),
            fetches: HashMap::new(),
            months: Some(self.months),
            group_by: Some(self.group_by.clone()),
            default_hidden: false,
        })
    }
}

/// Merge user-defined panels into a dashboard's defaults. A custom panel
/// whose title matches a default panel replaces it in place — that is how an
/// individual panel is re-pointed at a custom report (same slot, new spec);
/// every other title appends. Titles are the panel identity across the JSON,
/// the config and the picker, so they must stay unique. Returns the merged
/// list plus one error message per unusable custom panel.
pub fn merge_panels(
    base: Vec<DashboardPanel>,
    customs: &[CustomPanel],
) -> (Vec<DashboardPanel>, Vec<String>) {
    let mut panels = base;
    let mut errors = Vec::new();
    for custom in customs {
        match custom.to_dashboard_panel() {
            Ok(p) => {
                if let Some(idx) = panels.iter().position(|d| d.title == p.title) {
                    panels[idx] = p;
                } else {
                    panels.push(p);
                }
            }
            Err(e) => errors.push(e.to_string()),
        }
    }
    (panels, errors)
}

/// Apply the customize popup's per-pane report choices: for each
/// `panel title -> report title` assignment, swap that pane's definition for
/// the named report's spec (same slot). Unknown report names and unknown
/// panel titles (stale assignments after a rename) are reported, not
/// silently dropped. Runs after merge_panels so an assignment to a custom
/// panel also works.
pub fn apply_assignments(
    panels: Vec<DashboardPanel>,
    reports: &[CustomPanel],
    assignments: &HashMap<String, String>,
) -> (Vec<DashboardPanel>, Vec<String>) {
    let mut panels = panels;
    let mut errors = Vec::new();
    for (panel_title, report_title) in assignments {
        let Some(report) = reports.iter().find(|r| &r.title == report_title) else {
            errors.push(format!(
                "panel '{}' is assigned unknown report '{}'",
                panel_title, report_title
            ));
            continue;
        };
        let mut spec = match report.to_dashboard_panel() {
            Ok(p) => p,
            Err(e) => {
                errors.push(e.to_string());
                continue;
            }
        };
        // The pane keeps its own title; the report only supplies the spec.
        spec.title = panel_title.clone();
        match panels.iter().position(|d| &d.title == panel_title) {
            Some(idx) => panels[idx] = spec,
            None => errors.push(format!(
                "assignment targets panel '{}' which this dashboard does not have",
                panel_title
            )),
        }
    }
    (panels, errors)
}

/// The dashboard's own cost_table presets ("Past 3 Months by Service" and
/// any like it) double as named reports: every pane's customize popup offers
/// them, no config needed. They are already spec-driven, so the conversion
/// is mechanical.
pub fn builtin_reports(def: &DashboardDef) -> Vec<CustomPanel> {
    def.panels
        .iter()
        .filter(|p| p.kind == PanelKind::CostTable && p.group_by.is_some())
        .map(|p| CustomPanel {
            title: p.title.clone(),
            kind: PanelKind::CostTable,
            months: p.months.unwrap_or(3),
            group_by: p.group_by.clone().expect("filtered above"),
        })
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PanelKind {
    CostSummary,
    CostMonitor,
    CostBreakdown,
    TopTrends,
    CostTable,
}

impl PanelKind {
    /// The named fetches a kind's computor reads. A missing or extra name is
    /// a definition error — fail loudly rather than render an empty panel.
    /// CostTable builds its fetch from months/group_by instead.
    pub fn required_fetches(&self) -> &'static [&'static str] {
        match self {
            PanelKind::CostSummary => &["mtd", "last_month", "forecast"],
            PanelKind::CostMonitor => &["budgets", "anomalies"],
            PanelKind::CostBreakdown => &["monthly_by_service"],
            PanelKind::TopTrends => &["monthly_by_service"],
            PanelKind::CostTable => &[],
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            PanelKind::CostSummary => "cost_summary",
            PanelKind::CostMonitor => "cost_monitor",
            PanelKind::CostBreakdown => "cost_breakdown",
            PanelKind::TopTrends => "top_trends",
            PanelKind::CostTable => "cost_table",
        }
    }
}

impl Serialize for PanelKind {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

// `deny_unknown_fields` on the definition structs so a misspelled panel key
// fails at startup instead of silently dropping a widget.
impl<'de> Deserialize<'de> for PanelKind {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        match s.as_str() {
            "cost_summary" => Ok(PanelKind::CostSummary),
            "cost_monitor" => Ok(PanelKind::CostMonitor),
            "cost_breakdown" => Ok(PanelKind::CostBreakdown),
            "top_trends" => Ok(PanelKind::TopTrends),
            "cost_table" => Ok(PanelKind::CostTable),
            other => Err(serde::de::Error::unknown_variant(
                other,
                &[
                    "cost_summary",
                    "cost_monitor",
                    "cost_breakdown",
                    "top_trends",
                    "cost_table",
                ],
            )),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PanelFetch {
    pub service: String,
    pub action: String,
    #[serde(default)]
    pub static_params: Value,
    #[serde(default)]
    pub response_root: Option<String>,
}

// =============================================================================
// Panel output (computed data the UI renders)
// =============================================================================

#[derive(Debug, Clone)]
pub enum PanelData {
    Stats(Vec<StatItem>),
    Monitor(MonitorData),
    Breakdown(BreakdownData),
    Trends(Vec<TrendRow>),
    Table(Vec<TableRow>),
    Error(String),
}

#[derive(Debug, Clone)]
pub struct StatItem {
    pub label: String,
    pub value: String,
    pub note: Option<String>,
}

#[derive(Debug, Clone)]
pub struct MonitorData {
    pub budgets_line: String,
    pub anomalies_line: String,
    /// True when a budget is over or anomalies exist; renders red.
    pub alert: bool,
}

#[derive(Debug, Clone)]
pub struct BreakdownData {
    /// One row per month, oldest first.
    pub months: Vec<MonthRow>,
    /// Service names, index == colour index used by MonthRow segments.
    pub legend: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct MonthRow {
    pub label: String,
    pub total: f64,
    /// (legend index, amount) pairs, amounts sum to `total`.
    pub segments: Vec<(usize, f64)>,
}

#[derive(Debug, Clone)]
pub struct TrendRow {
    pub service: String,
    pub delta: f64,
    /// Percent change when the base month is non-zero.
    pub pct: Option<f64>,
}

/// One row of a cost table: the group name and its summed cost.
#[derive(Debug, Clone)]
pub struct TableRow {
    pub label: String,
    pub total: f64,
}

// =============================================================================
// Fetch + compute
// =============================================================================

/// Run every panel of a dashboard and return the computed data aligned with
/// `def.panels`. A failing fetch degrades its own panel to `PanelData::Error`
/// so one throttled call does not blank the page.
pub async fn fetch_dashboard(def: &DashboardDef, clients: &AwsClients) -> Vec<PanelData> {
    let mut out = Vec::with_capacity(def.panels.len());
    for panel in &def.panels {
        out.push(run_panel(panel, clients).await);
    }
    out
}

/// Fetch + compute a single panel. The panel picker uses this to populate a
/// newly-shown panel without refetching the whole page.
pub async fn fetch_panel(panel: &DashboardPanel, clients: &AwsClients) -> PanelData {
    run_panel(panel, clients).await
}

async fn run_panel(panel: &DashboardPanel, clients: &AwsClients) -> PanelData {
    // CostTable derives its fetch from months/group_by, not named fetches.
    if panel.kind == PanelKind::CostTable {
        let fetch = match cost_table_fetch(panel) {
            Ok(f) => f,
            Err(e) => return PanelData::Error(format!("{}: {e}", panel.title)),
        };
        return match run_fetch(&fetch, clients).await {
            Ok(response) => compute_cost_table(&response),
            Err(e) => PanelData::Error(format!("{}: {e}", panel.title)),
        };
    }

    let mut responses: HashMap<String, Value> = HashMap::new();
    for name in panel.kind.required_fetches() {
        let Some(fetch) = panel.fetches.get(*name) else {
            return PanelData::Error(format!(
                "panel '{}' kind {:?} is missing its '{}' fetch",
                panel.title, panel.kind, name
            ));
        };
        match run_fetch(fetch, clients).await {
            Ok(v) => {
                responses.insert(name.to_string(), v);
            }
            Err(e) => return PanelData::Error(format!("{}: {e}", panel.title)),
        }
    }
    match panel.kind {
        PanelKind::CostSummary => compute_cost_summary(&responses),
        PanelKind::CostMonitor => compute_cost_monitor(&responses),
        PanelKind::CostBreakdown => compute_breakdown(&responses),
        PanelKind::TopTrends => compute_trends(&responses),
        PanelKind::CostTable => unreachable!("cost_table handled above"),
    }
}

/// Build the single GetCostAndUsage call a cost table needs from its
/// window + group-by spec.
fn cost_table_fetch(panel: &DashboardPanel) -> Result<PanelFetch> {
    let group_by = panel
        .group_by
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("cost table '{}' needs a group_by", panel.title))?;
    let months = panel.months.unwrap_or(3).max(1);
    let start = if months == 1 {
        "{{month_start}}".to_string()
    } else {
        format!("{{{{month_start-{}M}}}}", months - 1)
    };
    Ok(PanelFetch {
        service: "ce".to_string(),
        action: "GetCostAndUsage".to_string(),
        static_params: serde_json::json!({
            "TimePeriod": { "Start": start, "End": "{{today+1d}}" },
            "Granularity": "MONTHLY",
            "Metrics": ["UnblendedCost"],
            "GroupBy": group_by.to_group_by()
        }),
        response_root: Some("/ResultsByTime".to_string()),
    })
}

async fn run_fetch(fetch: &PanelFetch, clients: &AwsClients) -> Result<Value> {
    // ApiConfig stores static params as a map; the JSON definition carries an
    // object, so convert (a non-object is an error — fail loudly).
    let static_params: HashMap<String, Value> = match fetch.static_params.clone() {
        Value::Object(map) => map.into_iter().collect(),
        Value::Null => HashMap::new(),
        other => {
            return Err(anyhow::anyhow!(
                "{}.{} static_params must be an object, got {other}",
                fetch.service,
                fetch.action
            ))
        }
    };
    let config = ApiConfig {
        protocol: ApiProtocol::Json,
        service_name: Some(fetch.service.clone()),
        action: Some(fetch.action.clone()),
        static_params,
        ..Default::default()
    };
    let handler = get_protocol_handler(ApiProtocol::Json);
    let raw = handler
        .execute(
            clients,
            &fetch.service,
            &config,
            &Value::Object(Default::default()),
        )
        .await?;
    let parsed: Value = serde_json::from_str(&raw)
        .map_err(|e| anyhow::anyhow!("bad JSON from {}: {e}", fetch.action))?;
    Ok(match &fetch.response_root {
        Some(p) => parsed.pointer(p).cloned().unwrap_or(Value::Null),
        None => parsed,
    })
}

/// Cost Explorer amounts arrive as strings ("354237.04000000004"); parse
/// tolerantly and treat anything unreadable as zero rather than NaN-poisoning
/// the sums.
fn amount(v: Option<&Value>) -> f64 {
    v.and_then(|v| match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.parse::<f64>().ok(),
        _ => None,
    })
    .unwrap_or(0.0)
}

fn compute_cost_summary(responses: &HashMap<String, Value>) -> PanelData {
    let Some(mtd_rows) = responses.get("mtd").and_then(|v| v.as_array()) else {
        return PanelData::Error("cost summary: no mtd results".into());
    };
    let mtd_total = amount(
        mtd_rows
            .first()
            .and_then(|r| r.pointer("/Total/UnblendedCost/Amount")),
    );

    let Some(last_rows) = responses.get("last_month").and_then(|v| v.as_array()) else {
        return PanelData::Error("cost summary: no last-month results".into());
    };
    // Daily rows: sum the days up to today's day-of-month for "same period",
    // all rows for the full month. Day-of-month alignment is what the console
    // means by "same time period" (Aug 1-8 vs Sep 1-8).
    let today_day = chrono::Utc::now().day() as i64;
    let mut same_period = 0.0;
    let mut last_total = 0.0;
    for row in last_rows {
        let day = row
            .pointer("/TimePeriod/Start")
            .and_then(|v| v.as_str())
            .and_then(|s| s.get(8..10))
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let cost = amount(row.pointer("/Total/UnblendedCost/Amount"));
        last_total += cost;
        if day <= today_day {
            same_period += cost;
        }
    }

    // response_root is /Total, so this value is the Total object itself.
    // GetCostForecast rejects a Start earlier than today and its amount
    // already matches the console's "Total forecasted cost for current
    // month" (measured live), so it is used as-is — no MTD double-add.
    let forecast_month = amount(responses.get("forecast").and_then(|v| v.pointer("/Amount")));

    let pct = |a: f64, b: f64| -> Option<String> {
        if b.abs() < 0.005 {
            return None;
        }
        let p = (a - b) / b * 100.0;
        if p.abs() < 0.5 {
            return None;
        }
        let arrow = if p > 0.0 { "↑" } else { "↓" };
        Some(format!("{arrow} {:.0}% vs", p.abs()))
    };

    PanelData::Stats(vec![
        StatItem {
            label: "Month-to-date cost".into(),
            value: money(mtd_total),
            note: pct(mtd_total, same_period).map(|n| format!("{n} last month same period")),
        },
        StatItem {
            label: "Last month, same period".into(),
            value: money(same_period),
            note: None,
        },
        StatItem {
            label: "Forecast, current month".into(),
            value: money(forecast_month),
            note: pct(forecast_month, last_total).map(|n| format!("{n} last month total")),
        },
        StatItem {
            label: "Last month's total".into(),
            value: money(last_total),
            note: None,
        },
    ])
}

fn compute_cost_monitor(responses: &HashMap<String, Value>) -> PanelData {
    let budgets = responses
        .get("budgets")
        .and_then(|v| v.pointer("/Budgets"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let over: Vec<&str> = budgets
        .iter()
        .filter(|b| {
            let limit = amount(b.pointer("/BudgetLimit/Amount"));
            let actual = amount(b.pointer("/CalculatedSpend/ActualSpend/Amount"));
            let forecasted = amount(b.pointer("/CalculatedSpend/ForecastedSpend/Amount"));
            limit > 0.0 && (actual > limit || forecasted > limit)
        })
        .filter_map(|b| b.pointer("/BudgetName").and_then(|v| v.as_str()))
        .collect();

    let anomalies = responses
        .get("anomalies")
        .and_then(|v| v.pointer("/Anomalies"))
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let anomaly_count = anomalies.len();
    let anomaly_impact: f64 = anomalies
        .iter()
        .map(|a| amount(a.pointer("/Impact/TotalImpact")))
        .sum();

    let budgets_line = if budgets.is_empty() {
        "No budgets created".to_string()
    } else if over.is_empty() {
        format!("{} budget(s), none over limit", budgets.len())
    } else {
        format!(
            "{} budget(s) over limit or forecast: {}",
            over.len(),
            over.join(", ")
        )
    };
    let anomalies_line = if anomaly_count == 0 {
        "No anomalies this month".to_string()
    } else {
        format!(
            "{} anomaly(ies) detected (MTD), {} impact",
            anomaly_count,
            money(anomaly_impact)
        )
    };

    PanelData::Monitor(MonitorData {
        budgets_line,
        anomalies_line,
        alert: !over.is_empty() || anomaly_count > 0,
    })
}

/// Reduce GetCostAndUsage ResultsByTime (monthly, grouped) into month rows
/// plus unsorted per-group totals for the whole window.
type MonthlyRows = (Vec<(String, Vec<(String, f64)>)>, Vec<(String, f64)>);

/// Reduce GetCostAndUsage ResultsByTime (monthly, grouped by SERVICE) into
/// month rows plus unsorted per-group totals.
fn monthly_rows(results: &Value) -> MonthlyRows {
    let mut months: Vec<(String, Vec<(String, f64)>)> = Vec::new();
    let mut totals: HashMap<String, f64> = HashMap::new();
    for row in results.as_array().cloned().unwrap_or_default() {
        let start = row
            .pointer("/TimePeriod/Start")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let mut groups: Vec<(String, f64)> = Vec::new();
        for g in row
            .pointer("/Groups")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default()
        {
            let key = g
                .pointer("/Keys/0")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown")
                .to_string();
            let cost = amount(g.pointer("/Metrics/UnblendedCost/Amount"));
            if cost.abs() < 0.005 {
                continue;
            }
            *totals.entry(key.clone()).or_default() += cost;
            groups.push((key, cost));
        }
        months.push((month_label(&start), groups));
    }
    // Raw totals, unsorted: callers decide their own order and whether
    // credits (negative totals) belong in the view at all.
    (months, totals.into_iter().collect())
}

/// A cost table sums the whole window per group and shows every group —
/// credits included, since they are real money — biggest impact first.
fn compute_cost_table(results: &Value) -> PanelData {
    let (months, mut totals) = monthly_rows(results);
    if months.is_empty() {
        return PanelData::Error("cost table: no months returned".into());
    }
    totals.sort_by(|a, b| {
        b.1.abs()
            .partial_cmp(&a.1.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    totals.truncate(20);
    PanelData::Table(
        totals
            .into_iter()
            .map(|(label, total)| TableRow { label, total })
            .collect(),
    )
}

fn compute_breakdown(responses: &HashMap<String, Value>) -> PanelData {
    let Some(results) = responses.get("monthly_by_service") else {
        return PanelData::Error("cost breakdown: no monthly results".into());
    };
    let (months, totals) = monthly_rows(results);
    if months.is_empty() {
        return PanelData::Error("cost breakdown: no months returned".into());
    }
    let mut ranked: Vec<(String, f64)> = totals
        .into_iter()
        // Negative totals are credits/refunds, not spend categories: they
        // belong to no legend row (their segments are invisible anyway) and
        // would otherwise crowd out real services. The month total rows still
        // include them, so the bars stay honest.
        .filter(|(_, total)| *total > 0.0)
        .collect();
    ranked.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
    // Top five services get their own colour; everything else — long tail and
    // credits alike — folds into one "Others" amount, the way the console does.
    let top: Vec<String> = ranked.iter().take(5).map(|(s, _)| s.clone()).collect();
    let mut legend = top.clone();
    let has_others = ranked.len() > 5;
    if has_others {
        legend.push("Others".into());
    }
    let rows = months
        .into_iter()
        .map(|(label, groups)| {
            let total: f64 = groups.iter().map(|(_, c)| c).sum();
            // Accumulate into one slot per legend entry plus a fold slot for
            // everything else (long tail and credits alike). The fold slot is
            // only kept when an "Others" legend entry exists.
            let mut per_index = vec![0.0; top.len() + 1];
            for (name, cost) in groups {
                let idx = top.iter().position(|t| *t == name).unwrap_or(top.len());
                per_index[idx] += cost;
            }
            let mut segments: Vec<(usize, f64)> = per_index
                .iter()
                .enumerate()
                .filter(|(_, cost)| cost.abs() >= 0.005)
                .map(|(idx, cost)| (idx, *cost))
                .collect();
            segments.sort_by_key(|(idx, _)| *idx);
            if !has_others {
                segments.retain(|(idx, _)| *idx < top.len());
            }
            MonthRow {
                label,
                total,
                segments,
            }
        })
        .collect();
    PanelData::Breakdown(BreakdownData {
        months: rows,
        legend,
    })
}

fn compute_trends(responses: &HashMap<String, Value>) -> PanelData {
    let Some(results) = responses.get("monthly_by_service") else {
        return PanelData::Error("top trends: no monthly results".into());
    };
    let (months, _) = monthly_rows(results);
    if months.len() < 2 {
        return PanelData::Error("top trends: need at least two months".into());
    }
    // The console compares the last two complete months, so a row for the
    // in-progress month (its label is the current month name) is dropped
    // before diffing.
    let now_month = month_name(chrono::Utc::now().month());
    let last_is_partial = months
        .last()
        .map(|(label, _)| label.starts_with(now_month))
        .unwrap_or(false);
    let (base_month, recent_month) = if last_is_partial && months.len() >= 3 {
        (&months[months.len() - 3], &months[months.len() - 2])
    } else {
        (&months[months.len() - 2], &months[months.len() - 1])
    };

    let mut base_map: HashMap<&str, f64> = HashMap::new();
    for (name, cost) in &base_month.1 {
        base_map.insert(name.as_str(), *cost);
    }
    let mut recent_map: HashMap<&str, f64> = HashMap::new();
    for (name, cost) in &recent_month.1 {
        recent_map.insert(name.as_str(), *cost);
    }
    let mut rows: Vec<TrendRow> = Vec::new();
    for (name, recent_cost) in &recent_map {
        let base_cost = base_map.get(name).copied().unwrap_or(0.0);
        let delta = recent_cost - base_cost;
        if delta.abs() < 0.005 {
            continue;
        }
        let pct = if base_cost.abs() > 0.005 {
            // Base magnitude keeps the sign of the delta on the percentage:
            // against a negative base (credits) a plain division would report
            // a cost decrease as a positive percent, which reads backwards.
            Some(delta / base_cost.abs() * 100.0)
        } else {
            None
        };
        rows.push(TrendRow {
            service: name.to_string(),
            delta,
            pct,
        });
    }
    for (name, base_cost) in &base_map {
        if !recent_map.contains_key(name) && base_cost.abs() >= 0.005 {
            rows.push(TrendRow {
                service: name.to_string(),
                delta: -base_cost,
                pct: Some(-100.0),
            });
        }
    }
    rows.sort_by(|a, b| {
        b.delta
            .abs()
            .partial_cmp(&a.delta.abs())
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    rows.truncate(10);
    PanelData::Trends(rows)
}

fn money(v: f64) -> String {
    match crate::resource::field_mapper::transform_format_money(&Value::from(v)) {
        Value::String(s) => s,
        _ => "-".to_string(),
    }
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

fn month_name(m: u32) -> &'static str {
    MONTHS[(m as usize - 1).min(11)]
}

/// "2026-09-01" (TimePeriod.Start) -> "Sep 2026".
fn month_label(start: &str) -> String {
    if start.len() >= 7 {
        if let Ok(m) = start[5..7].parse::<u32>() {
            if (1..=12).contains(&m) {
                return format!("{} {}", month_name(m), &start[..4]);
            }
        }
    }
    start.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    /// A GetCostAndUsage ResultsByTime fixture: one monthly MTD row and a
    /// daily previous month (3 rows), plus a GetCostForecast total.
    fn fixture_summary_responses() -> HashMap<String, Value> {
        HashMap::from([
            (
                "mtd".to_string(),
                json!([{
                    "TimePeriod": { "Start": "2026-09-01", "End": "2026-10-01" },
                    "Total": { "UnblendedCost": { "Amount": "354237.04", "Unit": "USD" } }
                }]),
            ),
            (
                "last_month".to_string(),
                json!([
                    { "TimePeriod": { "Start": "2026-08-01" },
                      "Total": { "UnblendedCost": { "Amount": "40000" } } },
                    { "TimePeriod": { "Start": "2026-08-08" },
                      "Total": { "UnblendedCost": { "Amount": "50000" } } },
                    { "TimePeriod": { "Start": "2026-08-20" },
                      "Total": { "UnblendedCost": { "Amount": "74484.01" } } }
                ]),
            ),
            (
                // Post-extraction: response_root is /Total, so the value is
                // the Total object itself.
                "forecast".to_string(),
                json!({ "Amount": "288683.53", "Unit": "USD" }),
            ),
        ])
    }

    /// The summary panel must produce the four console stats, with the
    /// "same period" slice counting only days up to today's day-of-month and
    /// the forecast taken straight from GetCostForecast (its amount already
    /// covers the month total, measured live against the console).
    #[test]
    fn cost_summary_computes_the_four_console_stats() {
        let data = compute_cost_summary(&fixture_summary_responses());
        let PanelData::Stats(items) = data else {
            panic!("summary must compute Stats, got {data:?}");
        };
        let values: Vec<&str> = items.iter().map(|s| s.value.as_str()).collect();
        assert_eq!(
            values,
            vec!["$354,237.04", "$90,000", "$288,683.53", "$164,484.01"],
            "mtd / same-period / forecast / last-month-total"
        );
        let mtd_note = items[0].note.clone().unwrap_or_default();
        assert!(
            mtd_note.contains("last month same period"),
            "the MTD stat must compare against the same period: {mtd_note}"
        );
        let forecast_note = items[2].note.clone().unwrap_or_default();
        assert!(
            forecast_note.contains("last month total"),
            "the forecast stat must compare against last month's total: {forecast_note}"
        );
    }

    /// Budgets with actual or forecasted spend past the limit are flagged,
    /// and anomaly count plus summed impact make up the second line.
    #[test]
    fn cost_monitor_flags_over_budgets_and_sums_anomaly_impact() {
        let responses = HashMap::from([
            (
                "budgets".to_string(),
                json!({ "Budgets": [
                    { "BudgetName": "prod", "BudgetLimit": { "Amount": "1000", "Unit": "USD" },
                      "CalculatedSpend": {
                          "ActualSpend": { "Amount": "1200" },
                          "ForecastedSpend": { "Amount": "3000" } } },
                    { "BudgetName": "dev", "BudgetLimit": { "Amount": "1000", "Unit": "USD" },
                      "CalculatedSpend": {
                          "ActualSpend": { "Amount": "10" },
                          "ForecastedSpend": { "Amount": "20" } } }
                ] }),
            ),
            (
                "anomalies".to_string(),
                json!({ "Anomalies": [
                    { "Impact": { "TotalImpact": "100.5" } },
                    { "Impact": { "TotalImpact": "-0.5" } }
                ] }),
            ),
        ]);
        let data = compute_cost_monitor(&responses);
        let PanelData::Monitor(m) = data else {
            panic!("monitor must compute Monitor, got {data:?}");
        };
        assert!(
            m.alert,
            "an over-limit budget and anomalies must trip the alert"
        );
        assert!(
            m.budgets_line.contains("prod") && !m.budgets_line.contains("dev"),
            "only the over-limit budget is named: {}",
            m.budgets_line
        );
        assert!(
            m.anomalies_line.contains("2 anomaly") && m.anomalies_line.contains("$100"),
            "anomaly line must count and sum impact: {}",
            m.anomalies_line
        );
    }

    /// An account with no budgets and no anomalies renders the "setup
    /// required" style message without an alert.
    #[test]
    fn cost_monitor_reports_setup_required_when_empty() {
        let responses = HashMap::from([
            ("budgets".to_string(), json!({ "Budgets": [] })),
            ("anomalies".to_string(), json!({ "Anomalies": [] })),
        ]);
        let data = compute_cost_monitor(&responses);
        let PanelData::Monitor(m) = data else {
            panic!("monitor must compute Monitor, got {data:?}");
        };
        assert!(
            !m.alert,
            "nothing over budget and no anomalies is not an alert"
        );
        assert_eq!(m.budgets_line, "No budgets created");
    }

    /// A six-month grouped fixture: months oldest-first, top five services
    /// ranked by spend across the window, the tail folded into Others.
    fn fixture_six_month_results() -> Value {
        json!([
            { "TimePeriod": { "Start": "2026-04-01" },
              "Groups": [
                { "Keys": ["Amazon S3"], "Metrics": { "UnblendedCost": { "Amount": "10" } } },
                { "Keys": ["EC2"], "Metrics": { "UnblendedCost": { "Amount": "90" } } }
              ] },
            { "TimePeriod": { "Start": "2026-05-01" },
              "Groups": [
                { "Keys": ["Amazon S3"], "Metrics": { "UnblendedCost": { "Amount": "10" } } },
                { "Keys": ["EC2"], "Metrics": { "UnblendedCost": { "Amount": "110" } } }
              ] },
            { "TimePeriod": { "Start": "2026-06-01" },
              "Groups": [
                { "Keys": ["EC2"], "Metrics": { "UnblendedCost": { "Amount": "100" } } }
              ] }
        ])
    }

    /// Breakdown must label months from TimePeriod.Start and rank services
    /// by spend across the whole window for the legend.
    #[test]
    fn cost_breakdown_labels_months_and_ranks_services() {
        let responses = HashMap::from([(
            "monthly_by_service".to_string(),
            fixture_six_month_results(),
        )]);
        let data = compute_breakdown(&responses);
        let PanelData::Breakdown(b) = data else {
            panic!("breakdown must compute Breakdown, got {data:?}");
        };
        assert_eq!(b.months.len(), 3);
        assert_eq!(b.months[0].label, "Apr 2026");
        assert_eq!(
            b.legend[0], "EC2",
            "EC2 has the most spend across the window"
        );
        assert_eq!(b.legend[1], "Amazon S3");
        assert_eq!(b.months[0].total, 100.0);
    }

    /// Trends diff the last two complete months (a partial current month row
    /// is dropped) and order by absolute delta.
    #[test]
    fn top_trends_deltas_the_last_two_complete_months() {
        let mut results = fixture_six_month_results();
        // A partial September row that must be ignored by the diff.
        results.as_array_mut().unwrap().push(json!({
            "TimePeriod": { "Start": "2026-09-01" },
            "Groups": [
                { "Keys": ["EC2"], "Metrics": { "UnblendedCost": { "Amount": "5" } } }
            ]
        }));
        let responses = HashMap::from([("monthly_by_service".to_string(), results)]);
        let data = compute_trends(&responses);
        let PanelData::Trends(rows) = data else {
            panic!("trends must compute Trends, got {data:?}");
        };
        // May->Jun: EC2 -10 (110 -> 100); S3 dropped off (-10). The September
        // partial row and its +5 must not appear anywhere.
        assert!(
            !rows.iter().any(|r| r.delta.abs() < 0.005),
            "no zero-delta rows expected"
        );
        assert!(rows.iter().all(|r| r.service != "Sep 2026"));
        assert_eq!(rows[0].service, "EC2");
        assert_eq!(rows[0].delta, -10.0);
        assert!(
            rows.iter().any(|r| r.pct == Some(-100.0)),
            "a service that vanished must show as -100%"
        );
    }

    /// A month label must render from the wire date; unparseable dates pass
    /// through verbatim rather than crashing the panel.
    #[test]
    fn month_label_formats_and_tolerates_garbage() {
        assert_eq!(month_label("2026-09-01"), "Sep 2026");
        assert_eq!(month_label("garbage"), "garbage");
    }

    /// A cost table sums the whole window per group and shows every group —
    /// credits included — biggest impact first, capped at 20 rows.
    #[test]
    fn cost_table_sums_per_group_and_ranks_by_impact() {
        let results = json!([
            { "TimePeriod": { "Start": "2026-07-01" },
              "Groups": [
                { "Keys": ["EC2"], "Metrics": { "UnblendedCost": { "Amount": "100" } } },
                { "Keys": ["S3"], "Metrics": { "UnblendedCost": { "Amount": "50" } } }
              ] },
            { "TimePeriod": { "Start": "2026-08-01" },
              "Groups": [
                { "Keys": ["EC2"], "Metrics": { "UnblendedCost": { "Amount": "120" } } },
                { "Keys": ["Refund"], "Metrics": { "UnblendedCost": { "Amount": "-90" } } },
                { "Keys": ["S3"], "Metrics": { "UnblendedCost": { "Amount": "10" } } }
              ] }
        ]);
        let data = compute_cost_table(&results);
        let PanelData::Table(rows) = data else {
            panic!("cost table must compute Table, got {data:?}");
        };
        let labels: Vec<&str> = rows.iter().map(|r| r.label.as_str()).collect();
        assert_eq!(
            labels,
            vec!["EC2", "Refund", "S3"],
            "ranked by absolute impact; credits keep their place"
        );
        assert_eq!(rows[0].total, 220.0);
        assert_eq!(rows[1].total, -90.0);
        assert_eq!(rows[2].total, 60.0);
    }

    /// The cost-table fetch is derived from the spec: window start N-1 months
    /// back (so N months are covered), End exclusive tomorrow, grouped by the
    /// requested dimension.
    #[test]
    fn cost_table_fetch_is_derived_from_the_spec() {
        let panel = DashboardPanel {
            kind: PanelKind::CostTable,
            title: "Past 3 Months by Service".into(),
            fetches: HashMap::new(),
            months: Some(3),
            group_by: Some(GroupBySpec {
                group_type: GroupByType::Dimension,
                key: "SERVICE".into(),
            }),
            default_hidden: true,
        };
        let fetch = cost_table_fetch(&panel).expect("fetch builds");
        assert_eq!(fetch.action, "GetCostAndUsage");
        assert_eq!(
            fetch.static_params["GroupBy"],
            json!([{ "Type": "DIMENSION", "Key": "SERVICE" }])
        );
        assert_eq!(
            fetch.static_params["TimePeriod"]["Start"], "{{month_start-2M}}",
            "3 months back starts 2 months before the current one"
        );
        assert_eq!(fetch.static_params["TimePeriod"]["End"], "{{today+1d}}");
        assert_eq!(fetch.response_root.as_deref(), Some("/ResultsByTime"));

        // One month is a valid window and must not emit a -0M template.
        let one = DashboardPanel {
            months: Some(1),
            ..panel
        };
        let fetch = cost_table_fetch(&one).expect("fetch builds");
        assert_eq!(
            fetch.static_params["TimePeriod"]["Start"],
            "{{month_start}}"
        );
    }

    /// A cost table without group_by is a definition error, not a silent
    /// ungrouped dump.
    #[test]
    fn cost_table_without_group_by_is_an_error() {
        let panel = DashboardPanel {
            kind: PanelKind::CostTable,
            title: "Broken".into(),
            fetches: HashMap::new(),
            months: Some(3),
            group_by: None,
            default_hidden: false,
        };
        assert!(cost_table_fetch(&panel).is_err());
    }

    /// User-defined panels must convert to the dashboard shape, and a kind
    /// other than cost_table must be rejected — the hardcoded computors'
    /// definitions would not survive user editing.
    #[test]
    fn custom_panels_convert_and_reject_unsupported_kinds() {
        let custom = CustomPanel {
            title: "EC2 spend".into(),
            kind: PanelKind::CostTable,
            months: 6,
            group_by: GroupBySpec {
                group_type: GroupByType::Dimension,
                key: "SERVICE".into(),
            },
        };
        let panel = custom.to_dashboard_panel().expect("cost_table converts");
        assert_eq!(panel.months, Some(6));
        assert!(panel.fetches.is_empty());

        custom.kind.as_str(); // PanelKind serializes for the config save path
        let yaml = serde_yaml::to_string(&custom).expect("custom panel serializes");
        let parsed: CustomPanel = serde_yaml::from_str(&yaml).expect("round-trips");
        assert_eq!(parsed.title, "EC2 spend");
        assert_eq!(parsed.months, 6);

        let wrong_kind = CustomPanel {
            title: "bad".into(),
            kind: PanelKind::CostSummary,
            ..custom
        };
        assert!(
            wrong_kind.to_dashboard_panel().is_err(),
            "non-cost_table custom panels are rejected"
        );
    }

    /// A custom panel whose title matches a default panel replaces it in
    /// place — that is how an individual panel is re-pointed at a custom
    /// report — while other titles append.
    #[test]
    fn merge_panels_replaces_by_title_and_appends_others() {
        let base = vec![
            panel_with_title("Cost Summary"),
            panel_with_title("Cost Breakdown"),
        ];
        let customs = vec![
            CustomPanel {
                title: "Cost Breakdown".into(),
                kind: PanelKind::CostTable,
                months: 3,
                group_by: GroupBySpec {
                    group_type: GroupByType::Tag,
                    key: "CostCenter".into(),
                },
            },
            CustomPanel {
                title: "Extra Report".into(),
                kind: PanelKind::CostTable,
                months: 6,
                group_by: GroupBySpec {
                    group_type: GroupByType::Dimension,
                    key: "SERVICE".into(),
                },
            },
        ];
        let (merged, errors) = merge_panels(base, &customs);
        assert!(errors.is_empty());
        assert_eq!(merged.len(), 3, "replace keeps position, append adds");
        assert_eq!(merged[0].title, "Cost Summary");
        assert_eq!(merged[1].title, "Cost Breakdown");
        assert_eq!(
            merged[1].kind,
            PanelKind::CostTable,
            "the replacement carries the custom spec"
        );
        assert_eq!(merged[1].group_by.as_ref().unwrap().key, "CostCenter");
        assert_eq!(merged[2].title, "Extra Report");
    }

    fn panel_with_title(title: &str) -> DashboardPanel {
        DashboardPanel {
            kind: PanelKind::CostBreakdown,
            title: title.into(),
            fetches: HashMap::new(),
            months: None,
            group_by: None,
            default_hidden: false,
        }
    }

    /// The customize popup's assignment swaps a pane's definition for the
    /// named report's spec while keeping the pane's own title and slot.
    #[test]
    fn apply_assignments_repoints_a_pane_and_reports_unknowns() {
        let base = vec![
            panel_with_title("Cost Breakdown"),
            panel_with_title("Top Trends"),
        ];
        let reports = vec![CustomPanel {
            title: "Cost Centers 3mo".into(),
            kind: PanelKind::CostTable,
            months: 3,
            group_by: GroupBySpec {
                group_type: GroupByType::Tag,
                key: "CostCenter".into(),
            },
        }];
        let assignments = HashMap::from([
            ("Cost Breakdown".to_string(), "Cost Centers 3mo".to_string()),
            ("Top Trends".to_string(), "Missing Report".to_string()),
        ]);
        let (resolved, errors) = apply_assignments(base, &reports, &assignments);
        assert_eq!(
            errors.len(),
            1,
            "the unknown report is a loud error, not a silent skip"
        );
        assert!(errors[0].contains("Missing Report"));
        assert_eq!(resolved[0].title, "Cost Breakdown", "pane keeps its title");
        assert_eq!(
            resolved[0].kind,
            PanelKind::CostTable,
            "but gets the report's spec"
        );
        assert_eq!(resolved[0].group_by.as_ref().unwrap().key, "CostCenter");
        assert_eq!(resolved[1].title, "Top Trends");
        assert_eq!(
            resolved[1].kind,
            PanelKind::CostBreakdown,
            "untouched pane unchanged"
        );
    }

    /// The dashboard's own cost_table presets double as named reports so any
    /// pane can adopt them via the customize popup without config.
    #[test]
    fn builtin_reports_expose_cost_table_presets_as_assignable_specs() {
        let def = DashboardDef {
            display_name: "Billing Overview".into(),
            panels: vec![
                panel_with_title("Cost Summary"),
                DashboardPanel {
                    kind: PanelKind::CostTable,
                    title: "Past 3 Months by Service".into(),
                    fetches: HashMap::new(),
                    months: Some(3),
                    group_by: Some(GroupBySpec {
                        group_type: GroupByType::Dimension,
                        key: "SERVICE".into(),
                    }),
                    default_hidden: true,
                },
            ],
        };
        let reports = builtin_reports(&def);
        assert_eq!(reports.len(), 1, "only cost_table panels are reports");
        assert_eq!(reports[0].title, "Past 3 Months by Service");
        assert_eq!(reports[0].months, 3);

        // Assigning it to a different pane keeps that pane's title.
        let base = vec![
            panel_with_title("Cost Monitor"),
            panel_with_title("Top Trends"),
        ];
        let assignments = HashMap::from([(
            "Cost Monitor".to_string(),
            "Past 3 Months by Service".to_string(),
        )]);
        let (resolved, errors) = apply_assignments(base, &reports, &assignments);
        assert!(errors.is_empty());
        assert_eq!(resolved[0].title, "Cost Monitor");
        assert_eq!(resolved[0].kind, PanelKind::CostTable);
        assert_eq!(resolved[0].group_by.as_ref().unwrap().key, "SERVICE");
    }

    /// An assignment naming a panel the dashboard does not have is stale and
    /// must surface too.
    #[test]
    fn apply_assignments_flags_stale_panel_titles() {
        let base = vec![panel_with_title("Cost Breakdown")];
        let reports = vec![CustomPanel {
            title: "Any".into(),
            kind: PanelKind::CostTable,
            months: 3,
            group_by: GroupBySpec {
                group_type: GroupByType::Dimension,
                key: "SERVICE".into(),
            },
        }];
        let (resolved, errors) = apply_assignments(
            base,
            &reports,
            &HashMap::from([("Gone".to_string(), "Any".to_string())]),
        );
        assert_eq!(resolved.len(), 1);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("does not have"), "{}", errors[0]);
    }
}
