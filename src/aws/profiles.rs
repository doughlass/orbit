use anyhow::Result;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

/// List all AWS profiles from ~/.aws/credentials and ~/.aws/config
pub fn list_profiles() -> Result<Vec<String>> {
    let mut profiles = HashSet::new();

    // Always include default
    profiles.insert("default".to_string());

    // Read from ~/.aws/credentials
    if let Some(creds_path) = get_aws_credentials_path() {
        if let Ok(content) = fs::read_to_string(&creds_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    let profile = line[1..line.len() - 1].to_string();
                    profiles.insert(profile);
                }
            }
        }
    }

    // Read from ~/.aws/config
    if let Some(config_path) = get_aws_config_path() {
        if let Ok(content) = fs::read_to_string(&config_path) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('[') && line.ends_with(']') {
                    let section = &line[1..line.len() - 1];
                    // Config file uses "profile <name>" format, except for default
                    let profile = if let Some(p) = section.strip_prefix("profile ") {
                        p.to_string()
                    } else {
                        section.to_string()
                    };
                    profiles.insert(profile);
                }
            }
        }
    }

    let mut profiles: Vec<String> = profiles.into_iter().collect();
    profiles.sort();

    Ok(profiles)
}

/// List common AWS regions
pub fn list_regions() -> Vec<String> {
    vec![
        "us-east-1".to_string(),
        "us-east-2".to_string(),
        "us-west-1".to_string(),
        "us-west-2".to_string(),
        "af-south-1".to_string(),
        "ap-east-1".to_string(),
        "ap-south-1".to_string(),
        "ap-south-2".to_string(),
        "ap-southeast-1".to_string(),
        "ap-southeast-2".to_string(),
        "ap-southeast-3".to_string(),
        "ap-southeast-4".to_string(),
        "ap-northeast-1".to_string(),
        "ap-northeast-2".to_string(),
        "ap-northeast-3".to_string(),
        "ca-central-1".to_string(),
        "eu-central-1".to_string(),
        "eu-central-2".to_string(),
        "eu-west-1".to_string(),
        "eu-west-2".to_string(),
        "eu-west-3".to_string(),
        "eu-south-1".to_string(),
        "eu-south-2".to_string(),
        "eu-north-1".to_string(),
        "me-south-1".to_string(),
        "me-central-1".to_string(),
        "sa-east-1".to_string(),
    ]
}

fn get_aws_credentials_path() -> Option<PathBuf> {
    // Check AWS_SHARED_CREDENTIALS_FILE env var first
    if let Ok(path) = std::env::var("AWS_SHARED_CREDENTIALS_FILE") {
        return Some(PathBuf::from(path));
    }

    // Fall back to ~/.aws/credentials
    dirs::home_dir().map(|h| h.join(".aws").join("credentials"))
}

fn get_aws_config_path() -> Option<PathBuf> {
    // Check AWS_CONFIG_FILE env var first
    if let Ok(path) = std::env::var("AWS_CONFIG_FILE") {
        return Some(PathBuf::from(path));
    }

    // Fall back to ~/.aws/config
    dirs::home_dir().map(|h| h.join(".aws").join("config"))
}
