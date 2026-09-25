//! Configuration management for orbit
//!
//! Stores user preferences in ~/.config/orbit/config.yaml (XDG compliant)
//! Falls back to ~/.orbit/config.yaml if XDG dirs not available

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::PathBuf;
use tracing::{debug, warn};

/// User configuration stored on disk
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct Config {
    /// Last used AWS profile
    #[serde(default)]
    pub profile: Option<String>,

    /// Last used AWS region
    #[serde(default)]
    pub region: Option<String>,

    /// Last viewed resource type
    #[serde(default)]
    pub last_resource: Option<String>,

    /// Recently used regions (most recent first, max 6)
    #[serde(default)]
    pub recently_used_regions: Vec<String>,

    /// Per-resource visible column headers, as saved from the column picker
    /// (p). Keyed by resource key; a resource with no entry shows the columns
    /// the resource JSON marks visible.
    #[serde(default)]
    pub column_preferences: std::collections::HashMap<String, Vec<String>>,

    /// Per-dashboard user customization, keyed by dashboard key (e.g.
    /// "billing"): extra panels the user defined and the picker's shown/
    /// hidden overrides, persisted so the layout survives restarts.
    #[serde(default)]
    pub dashboards: std::collections::HashMap<String, DashboardUserConfig>,
}

/// User-level dashboard customization. `panels` appends custom panels;
/// `shown`/`hidden` are panel *titles* recorded by the panel picker. A title
/// in `hidden` hides the panel, in `shown` shows it (overriding a JSON
/// `default_hidden`), and a title in neither uses the definition default.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct DashboardUserConfig {
    #[serde(default)]
    pub panels: Vec<crate::resource::CustomPanel>,
    #[serde(default)]
    pub shown: Vec<String>,
    #[serde(default)]
    pub hidden: Vec<String>,
    /// Named report specs, reusable from any panel's customize popup. Each
    /// is a cost_table spec (`kind` must be cost_table); the pane it ends up
    /// on supplies the display title via `assignments`.
    #[serde(default)]
    pub reports: Vec<crate::resource::CustomPanel>,
    /// Per-pane report choice from the customize popup: panel title ->
    /// report title. Absent (or empty) means the pane keeps its JSON
    /// definition.
    #[serde(default)]
    pub assignments: std::collections::HashMap<String, String>,
}

impl DashboardUserConfig {
    /// Whether a panel with this title currently shows. `default_hidden` is
    /// the definition's own preference for when neither list mentions it.
    pub fn is_visible(&self, title: &str, default_hidden: bool) -> bool {
        if self.hidden.iter().any(|t| t == title) {
            return false;
        }
        if self.shown.iter().any(|t| t == title) {
            return true;
        }
        !default_hidden
    }

    /// Record an explicit show/hide for a title, replacing the opposite
    /// override so the latest picker choice always wins. Pure: the caller
    /// decides whether to persist.
    pub fn set_visible(&mut self, title: &str, visible: bool) {
        let (target, other) = if visible {
            (&mut self.shown, &mut self.hidden)
        } else {
            (&mut self.hidden, &mut self.shown)
        };
        other.retain(|t| t != title);
        if !target.iter().any(|t| t == title) {
            target.push(title.to_string());
        }
    }
}

impl Config {
    /// Load config from disk, or return default if not found
    pub fn load() -> Self {
        let path = Self::config_path();
        debug!("Loading config from {:?}", path);

        if path.exists() {
            match fs::read_to_string(&path) {
                Ok(contents) => match serde_yaml::from_str(&contents) {
                    Ok(config) => {
                        debug!("Config loaded successfully: {:?}", config);
                        return config;
                    }
                    Err(e) => {
                        warn!("Failed to parse config: {}", e);
                    }
                },
                Err(e) => {
                    warn!("Failed to read config: {}", e);
                }
            }
        } else {
            debug!("Config file does not exist, using defaults");
        }

        Self::default()
    }

    /// Save config to disk
    pub fn save(&self) -> Result<()> {
        let path = Self::config_path();
        debug!("Saving config to {:?}", path);

        // Ensure parent directory exists
        if let Some(parent) = path.parent() {
            debug!("Creating parent directory: {:?}", parent);
            fs::create_dir_all(parent)?;
        }

        let contents = serde_yaml::to_string(self)?;
        fs::write(&path, contents)?;
        debug!("Config saved successfully: {:?}", self);

        Ok(())
    }

    /// Get the config file path
    /// Uses XDG config directory if available, otherwise ~/.orbit/
    fn config_path() -> PathBuf {
        // Try XDG config dir first (e.g., ~/.config/orbit/config.yaml)
        if let Some(config_dir) = dirs::config_dir() {
            return config_dir.join("orbit").join("config.yaml");
        }

        // Fallback to home directory
        if let Some(home) = dirs::home_dir() {
            return home.join(".orbit").join("config.yaml");
        }

        // Last resort: current directory
        PathBuf::from(".orbit").join("config.yaml")
    }

    /// Update profile and save
    pub fn set_profile(&mut self, profile: &str) -> Result<()> {
        debug!("Setting profile to: {}", profile);
        self.profile = Some(profile.to_string());
        self.save()
    }

    /// Update region and save
    pub fn set_region(&mut self, region: &str) -> Result<()> {
        debug!("Setting region to: {}", region);
        self.region = Some(region.to_string());
        self.add_recent_region(region);
        self.save()
    }

    /// Add region to recently used list (most recent first, max 6)
    fn add_recent_region(&mut self, region: &str) {
        // Remove if already exists
        self.recently_used_regions.retain(|r| r != region);
        // Add to front
        self.recently_used_regions.insert(0, region.to_string());
        // Keep max 6
        self.recently_used_regions.truncate(6);
    }

    /// Get recently used regions for display (returns up to 6)
    pub fn get_recent_regions(&self) -> Vec<String> {
        self.recently_used_regions.clone()
    }

    /// Saved visible column headers for a resource, if the user has
    /// customised them. None means "use the JSON defaults".
    pub fn column_preferences(&self, resource_key: &str) -> Option<&Vec<String>> {
        self.column_preferences.get(resource_key)
    }

    /// Update last resource and save
    #[allow(dead_code)]
    pub fn set_last_resource(&mut self, resource: &str) -> Result<()> {
        self.last_resource = Some(resource.to_string());
        self.save()
    }

    /// Get effective profile (config -> env -> default)
    pub fn effective_profile(&self) -> String {
        // Priority: 1. Environment variable, 2. Config file, 3. Default
        std::env::var("AWS_PROFILE")
            .ok()
            .or_else(|| self.profile.clone())
            .unwrap_or_else(|| "default".to_string())
    }

    /// Get effective region (config -> env -> default)
    pub fn effective_region(&self) -> String {
        // Priority: 1. Environment variable, 2. Config file, 3. Default
        std::env::var("AWS_REGION")
            .ok()
            .or_else(|| std::env::var("AWS_DEFAULT_REGION").ok())
            .or_else(|| self.region.clone())
            .unwrap_or_else(|| "eu-west-1".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The on-disk directory carries the brand name. A leftover "taws" here
    /// silently splits config between two locations after the rename.
    #[test]
    fn test_config_path_uses_orbit_directory() {
        let path = Config::config_path();
        let path_str = path.to_string_lossy();

        assert!(
            path.ends_with("orbit/config.yaml") || path.ends_with(".orbit/config.yaml"),
            "config path should live under an orbit directory, got {}",
            path_str
        );
        assert!(
            !path_str.contains("taws"),
            "config path should not reference the old name, got {}",
            path_str
        );
    }

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert!(config.profile.is_none());
        assert!(config.region.is_none());
    }

    #[test]
    fn test_serialize_deserialize() {
        let config = Config {
            profile: Some("my-profile".to_string()),
            region: Some("eu-west-1".to_string()),
            last_resource: Some("ec2-instances".to_string()),
            recently_used_regions: vec!["eu-west-1".to_string(), "us-east-1".to_string()],
            column_preferences: std::collections::HashMap::new(),
            dashboards: std::collections::HashMap::new(),
        };

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        assert_eq!(parsed.profile, config.profile);
        assert_eq!(parsed.region, config.region);
        assert_eq!(parsed.last_resource, config.last_resource);
        assert_eq!(parsed.recently_used_regions, config.recently_used_regions);
    }

    /// Column preferences must survive a serialise round-trip so a config
    /// copied between machines restores the same table views.
    #[test]
    fn test_column_preferences_round_trip() {
        let mut config = Config::default();
        config.column_preferences.insert(
            "ec2-instances".to_string(),
            vec!["NAME".to_string(), "STATE".to_string()],
        );

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();

        let prefs = parsed.column_preferences("ec2-instances").unwrap();
        assert_eq!(prefs, &vec!["NAME".to_string(), "STATE".to_string()]);
        assert!(parsed.column_preferences("eks-clusters").is_none());
    }

    /// The picker's visibility overrides must round-trip and behave like the
    /// picker expects: hidden wins, shown overrides default_hidden, and the
    /// latest toggle replaces the opposite record.
    #[test]
    fn test_dashboard_visibility_overrides_round_trip() {
        let mut config = Config::default();
        let ucfg = config.dashboards.entry("billing".to_string()).or_default();
        ucfg.set_visible("Past 3 Months by Service", true);
        ucfg.set_visible("Top Trends", false);

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        let ucfg = parsed.dashboards.get("billing").unwrap();

        // default_hidden preset explicitly shown
        assert!(ucfg.is_visible("Past 3 Months by Service", true));
        // default-visible panel explicitly hidden
        assert!(!ucfg.is_visible("Top Trends", false));
        // untouched panel follows the definition default
        assert!(ucfg.is_visible("Cost Summary", false));
        assert!(!ucfg.is_visible("Cost Monitor", true));

        // Toggling back replaces the opposite record instead of fighting it.
        let mut ucfg = ucfg.clone();
        ucfg.set_visible("Past 3 Months by Service", false);
        assert!(!ucfg.is_visible("Past 3 Months by Service", true));
        assert!(
            !ucfg.shown.contains(&"Past 3 Months by Service".to_string()),
            "a later hide must remove the earlier show record"
        );
    }

    /// User-defined custom panels must survive a config round-trip.
    #[test]
    fn test_dashboard_custom_panels_round_trip() {
        let mut config = Config::default();
        let ucfg = config.dashboards.entry("billing".to_string()).or_default();
        ucfg.panels.push(crate::resource::CustomPanel {
            title: "Cost Centers".into(),
            kind: crate::resource::dashboard::PanelKind::CostTable,
            months: 4,
            group_by: crate::resource::dashboard::GroupBys::One(
                crate::resource::dashboard::GroupBySpec {
                    group_type: crate::resource::dashboard::GroupByType::CostCategory,
                    key: "CostCenter".into(),
                },
            ),
            metric: None,
            filter: None,
        });

        let yaml = serde_yaml::to_string(&config).unwrap();
        assert!(
            yaml.contains("cost_table") && yaml.contains("CostCenter"),
            "the YAML must carry the group-by spec: {yaml}"
        );
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        let panels = &parsed.dashboards.get("billing").unwrap().panels;
        assert_eq!(panels.len(), 1);
        assert_eq!(panels[0].title, "Cost Centers");
        assert_eq!(panels[0].months, 4);
        let panel = panels[0].to_dashboard_panel().expect("converts");
        assert_eq!(panel.months, Some(4));
    }

    /// Named reports and the customize popup's assignments must round-trip:
    /// the popup reads reports to build its options and assignments to mark
    /// the current choice.
    #[test]
    fn test_dashboard_reports_and_assignments_round_trip() {
        let mut config = Config::default();
        let ucfg = config.dashboards.entry("billing".to_string()).or_default();
        ucfg.reports.push(crate::resource::CustomPanel {
            title: "Cost Centers 3mo".into(),
            kind: crate::resource::dashboard::PanelKind::CostTable,
            months: 3,
            group_by: crate::resource::dashboard::GroupBys::One(
                crate::resource::dashboard::GroupBySpec {
                    group_type: crate::resource::dashboard::GroupByType::Tag,
                    key: "CostCenter".into(),
                },
            ),
            metric: None,
            filter: None,
        });
        ucfg.assignments
            .insert("Cost Breakdown".to_string(), "Cost Centers 3mo".to_string());

        let yaml = serde_yaml::to_string(&config).unwrap();
        let parsed: Config = serde_yaml::from_str(&yaml).unwrap();
        let ucfg = parsed.dashboards.get("billing").unwrap();
        assert_eq!(ucfg.reports.len(), 1);
        assert_eq!(ucfg.reports[0].title, "Cost Centers 3mo");
        assert_eq!(
            ucfg.assignments.get("Cost Breakdown").map(|s| s.as_str()),
            Some("Cost Centers 3mo")
        );
    }

    #[test]
    fn test_add_recent_region() {
        let mut config = Config::default();

        config.add_recent_region("us-east-1");
        assert_eq!(config.recently_used_regions, vec!["us-east-1"]);

        config.add_recent_region("eu-west-1");
        assert_eq!(config.recently_used_regions, vec!["eu-west-1", "us-east-1"]);

        // Adding existing region moves it to front
        config.add_recent_region("us-east-1");
        assert_eq!(config.recently_used_regions, vec!["us-east-1", "eu-west-1"]);

        // Max 6 regions
        config.add_recent_region("r1");
        config.add_recent_region("r2");
        config.add_recent_region("r3");
        config.add_recent_region("r4");
        config.add_recent_region("r5");
        assert_eq!(config.recently_used_regions.len(), 6);
        assert_eq!(config.recently_used_regions[0], "r5");
    }
}
