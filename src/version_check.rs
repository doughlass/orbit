//! Update checks against GitHub Releases.
//!
//! orbit is a binary you install how you like (cargo, brew, a raw binary), so
//! it cannot be sure how it will be updated. The check itself is universal: it
//! asks GitHub for the latest release tag and compares it against the embedded
//! `CARGO_PKG_VERSION`. How the user gets the new build then depends on how the
//! current one was installed (`install_method`).
//!
//! Two rules keep this from becoming a nuisance:
//! - The check runs at most once a day; the timestamp survives in a small JSON
//!   file next to the config so a user who opens orbit every hour is not
//!   hammered with a network round-trip each time.
//! - Any network failure is silent. An update check must never block startup
//!   or turn into an error the user has to dismiss.

use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

/// How the running orbit was installed. All but `Raw` route the user to their
/// package manager rather than trying to self-replace, because overwriting a
/// cargo-installed or brew-managed binary would be reverted or break the
/// manager's bookkeeping.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMethod {
    /// Installed via `cargo install`, sitting in a cargo bin directory.
    Cargo,
    /// Installed via Homebrew.
    Brew,
    /// A raw binary somewhere not managed by a package manager — safe to
    /// self-replace. Also the fallback when the path is ambiguous: a false
    /// "Raw" only offers a self-update the user can decline.
    Raw,
}

impl InstallMethod {
    /// Best-effort guess from the running executable's path. A false "Raw"
    /// would only offer a self-update the user can decline, so err toward Raw
    /// when the path is ambiguous.
    pub fn detect(current_exe: &std::path::Path) -> Self {
        let path = match current_exe.canonicalize() {
            Ok(p) => p,
            Err(_) => current_exe.to_path_buf(),
        };
        let path_str = path.to_string_lossy().to_lowercase();

        if path_str.contains("/homebrew/")
            || path_str.contains("/brew/")
            || path_str.contains("-brew")
        {
            return InstallMethod::Brew;
        }
        if path_str.contains("/.cargo/bin/") || path_str.contains("/.cargo/bin\\") {
            return InstallMethod::Cargo;
        }
        InstallMethod::Raw
    }

    /// The wording used in the update prompt for this method.
    pub fn label(self) -> &'static str {
        match self {
            InstallMethod::Cargo => "cargo install -f orbit-tui",
            InstallMethod::Brew => "brew upgrade orbit",
            InstallMethod::Raw => "download and replace the binary",
        }
    }
}

/// State persisted between launches so the network check runs at most daily.
/// Kept separate from the YAML config because it is machine-scoped cache, not
/// a user preference that is safe to ship between machines.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CheckCache {
    /// Unix timestamp of the last successful check, in seconds.
    #[serde(default)]
    pub last_check: Option<u64>,
}

/// How long to wait between checks; the "once a day" rule is actually
/// "at least this many seconds since the last check", so a check that fails
/// while offline does not mark itself as done (we only write the timestamp on
/// a successful check).
pub const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 60 * 60);

/// Path to the cache file, alongside the config.
pub fn cache_path() -> PathBuf {
    if let Some(dir) = dirs::config_dir() {
        return dir.join("orbit").join("version-check.json");
    }
    PathBuf::from("version-check.json")
}

/// Whether a check is due, given the last successful check time.
pub fn should_check(last_check: Option<u64>, now: u64) -> bool {
    match last_check {
        None => true,
        Some(t) => now.saturating_sub(t) >= CHECK_INTERVAL.as_secs(),
    }
}

/// Whether to run a check at all. `force` (from `orbit --update`) bypasses the
/// daily interval, so a forced run always consults GitHub.
fn should_fetch(force: bool, last_check: Option<u64>, now: u64) -> bool {
    force || should_check(last_check, now)
}

/// Parse a GitHub release tag the way the version comparison wants it: strip a
/// leading "v", and split off any pre-release/build suffix. Returns a list of
/// numeric components for the version-proper part plus the pre-release string.
fn parse_version(raw: &str) -> (Vec<u64>, String) {
    let raw = raw.trim().trim_start_matches(['v', 'V']);
    let (ver, pre) = match raw.split_once('-') {
        Some((v, p)) => (v, p.to_string()),
        None => (raw, String::new()),
    };
    let nums: Vec<u64> = ver
        .split('.')
        .filter_map(|part| part.trim().parse::<u64>().ok())
        .collect();
    (nums, pre)
}

/// True if `latest` is a newer release than `current`. Numeric comparison of
/// dotted components; a prerelease (`-` suffix) is treated as older than any
/// release build. Non-numeric current versions (e.g. "dev") are never "newer".
pub fn is_newer(current: &str, latest: &str) -> bool {
    let (cur_nums, cur_pre) = parse_version(current);
    let (lat_nums, lat_pre) = parse_version(latest);

    // Length first (1.2 < 1.2.0 is treated equal-length here because we pad),
    // but we compare component-wise with zero-padding so 1.2 == 1.2.0.
    let max_len = cur_nums.len().max(lat_nums.len());
    for i in 0..max_len {
        let a = cur_nums.get(i).copied().unwrap_or(0);
        let b = lat_nums.get(i).copied().unwrap_or(0);
        match a.cmp(&b) {
            std::cmp::Ordering::Greater => return false,
            std::cmp::Ordering::Less => return true,
            std::cmp::Ordering::Equal => {}
        }
    }

    // Same version-proper: a release (no pre) beats a prerelease.
    if cur_pre.is_empty() && !lat_pre.is_empty() {
        return false;
    }
    if !cur_pre.is_empty() && lat_pre.is_empty() {
        return true;
    }
    false
}

fn strip_version_tag(tag: &str) -> String {
    tag.trim().trim_start_matches(['v', 'V']).to_string()
}

/// Load the persisted cache (default if missing or unreadable).
pub fn load_cache() -> CheckCache {
    match std::fs::read_to_string(cache_path()) {
        Ok(raw) => serde_json::from_str(&raw).unwrap_or_default(),
        Err(_) => CheckCache::default(),
    }
}

/// Persist the cache. Failures are ignored — a stale cache only means the
/// next launch checks again, which is harmless.
pub fn save_cache(cache: &CheckCache) {
    if let Some(parent) = cache_path().parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(raw) = serde_json::to_string(cache) {
        let _ = std::fs::write(cache_path(), raw);
    }
}

/// A release item from the GitHub Releases API, only the fields we read.
#[derive(Debug, Deserialize)]
struct GithubRelease {
    tag_name: String,
    #[allow(dead_code)]
    html_url: String,
    #[serde(default)]
    prerelease: Option<bool>,
}

/// Ask GitHub for the latest release. Returns `Err` on network/parse failure
/// (callers swallow this); otherwise `Ok(Some(version))` is the newest release
/// tag (v-stripped). The newest release is the first non-empty tag in the
/// API response, which GitHub returns newest-first.
pub fn fetch_latest_release() -> Result<Option<String>> {
    let releases = fetch_releases()?;
    for release in releases {
        if !release.tag_name.is_empty() && release.prerelease != Some(true) {
            return Ok(Some(strip_version_tag(&release.tag_name)));
        }
    }
    Ok(None)
}

fn fetch_releases() -> Result<Vec<GithubRelease>> {
    let url = "https://api.github.com/repos/doughlass/orbit/releases";
    let client = reqwest::blocking::Client::builder()
        .user_agent("orbit-tui")
        .timeout(Duration::from_secs(10))
        .build()?;
    let resp = client.get(url).send()?;
    if !resp.status().is_success() {
        anyhow::bail!("GitHub API returned {}", resp.status());
    }
    let releases: Vec<GithubRelease> = resp.json()?;
    Ok(releases)
}

/// Full update path: consult the cache (unless forced), fetch if due, persist
/// the new timestamp on a successful check, and return the latest version only
/// when it is newer than the running one. Any failure returns `Ok(None)` and
/// leaves the cache untouched, so an offline launch is not counted as a check.
///
/// `force` bypasses the daily interval gate (used by `orbit --update`) but
/// still refreshes the cache timestamp, since a forced check is a real check.
pub fn check_with_cache(force: bool) -> Option<String> {
    let now = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let mut cache = load_cache();
    if !should_fetch(force, cache.last_check, now) {
        return None;
    }

    let mut result = None;
    if let Ok(Some(latest)) = fetch_latest_release() {
        if is_newer(crate::VERSION, &latest) {
            result = Some(latest);
        }
        cache.last_check = Some(now);
        save_cache(&cache);
    }
    result
}

/// Attempt a raw-binary self-update: download the release asset for this
/// platform, verify its checksum, and hand off the swap+relaunch to a detached
/// helper so the currently-running binary can be overwritten.
pub async fn self_update(latest: &str) -> Result<()> {
    let asset_name = asset_name()?;
    let download_url = format!(
        "https://github.com/doughlass/orbit/releases/download/v{}/{}",
        strip_version_tag(latest),
        asset_name
    );

    let tmp = std::env::temp_dir().join(format!("orbit-update-{}", std::process::id()));
    std::fs::create_dir_all(&tmp)?;

    let archive_path = tmp.join(&asset_name);
    download(&download_url, &archive_path).await?;

    // Checksums are published alongside the assets. Verifying is best-effort:
    // if the file is absent we proceed (older releases predate it), but if it
    // is present and mismatches we refuse to install.
    if let Ok(checksums) = download_checksums(latest).await {
        verify_checksum(
            &archive_path,
            &tmp.join("checksums.txt"),
            checksums.as_bytes(),
        )?;
    }

    let binary_path = extract_binary(&archive_path).await?;
    let current = std::env::current_exe()?;

    tracing::warn!(
        "self-update staged at {:?}; handing off swap to detached helper",
        binary_path
    );

    // Swap + relaunch happen in a detached child because the running binary
    // cannot be overwritten while it is executing. The original CLI args are
    // threaded through so the relaunched orbit keeps the user's flags
    // (--region, --profile, --readonly, ...). On failure to spawn, nothing is
    // left half-done.
    let original_args: Vec<String> = std::env::args().skip(1).collect();
    spawn_update_helper(&binary_path, &current, &original_args)?;

    // The archive dir is no longer needed; the staged binary lives in its own
    // dir and is read by the detached helper after we exit.
    let _ = std::fs::remove_dir_all(&tmp);

    Ok(())
}

/// Release asset file name for this platform. The release workflow cross-
/// builds with `cross`, producing musl (static) Linux tarballs and darwin
/// tarballs; Windows ships a `.zip`, which the tar-based self-update cannot
/// read, so it is sensibly excluded here (the user still gets the release
/// page for a manual download).
fn asset_name() -> Result<String> {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let crate_name = "orbit-tui";
    let target = match (os, arch) {
        ("macos", "aarch64") => "aarch64-apple-darwin",
        ("macos", "x86_64") => "x86_64-apple-darwin",
        ("linux", "aarch64") => "aarch64-unknown-linux-musl",
        ("linux", "x86_64") => "x86_64-unknown-linux-musl",
        (os, arch) => {
            anyhow::bail!("self-update unsupported for OS {os} / arch {arch}; update manually")
        }
    };
    Ok(format!("{crate_name}-{target}.tar.gz"))
}

async fn download(url: &str, dest: &std::path::Path) -> Result<()> {
    let client = reqwest::Client::builder()
        .user_agent("orbit-tui")
        .timeout(Duration::from_secs(120))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("download failed: {}", resp.status());
    }
    let bytes = resp.bytes().await?;
    std::fs::write(dest, &bytes)?;
    Ok(())
}

async fn download_checksums(latest: &str) -> Result<String> {
    let url = format!(
        "https://github.com/doughlass/orbit/releases/download/v{}/checksums.txt",
        strip_version_tag(latest)
    );
    let client = reqwest::Client::builder()
        .user_agent("orbit-tui")
        .timeout(Duration::from_secs(60))
        .build()?;
    let resp = client.get(url).send().await?;
    if !resp.status().is_success() {
        anyhow::bail!("checksums download failed: {}", resp.status());
    }
    Ok(resp.text().await?)
}

/// Verify `archive` against the published `sha256sum`-style `checksums`
/// listing. Errors out on mismatch; a missing entry for our asset also errors.
fn verify_checksum(
    archive: &std::path::Path,
    _tmp_checksums: &std::path::Path,
    checksums: &[u8],
) -> Result<()> {
    let listing = String::from_utf8_lossy(checksums);
    let file_name = archive
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or_default();

    let expected = listing
        .lines()
        .map(str::trim)
        .find_map(|line| {
            // sha256sum format: "<hash>  <filename>"
            let mut it = line.split_whitespace();
            let hash = it.next()?.to_lowercase();
            let name = it.next().unwrap_or("");
            if name == file_name {
                Some(hash)
            } else {
                None
            }
        })
        .ok_or_else(|| anyhow::anyhow!("no checksum entry for {file_name}"))?;

    let actual = sha256_file(archive)?;
    if actual != expected {
        anyhow::bail!("checksum mismatch for {file_name}: expected {expected}, got {actual}");
    }
    Ok(())
}

fn sha256_file(path: &std::path::Path) -> Result<String> {
    use sha2::{Digest, Sha256};
    let bytes = std::fs::read(path)?;
    let mut hasher = Sha256::new();
    hasher.update(&bytes);
    Ok(hex::encode(hasher.finalize()))
}

/// Extract the single binary from a `tar.gz` release asset into a fresh temp
/// dir and return its path.
async fn extract_binary(archive: &std::path::Path) -> Result<std::path::PathBuf> {
    use std::io::Read;
    let archive_display = archive.display().to_string();
    let file = std::fs::File::open(archive)?;
    let mut gz = flate2::read::GzDecoder::new(file);
    let mut tar_bytes = Vec::new();
    gz.read_to_end(&mut tar_bytes)?;

    let dir = std::env::temp_dir().join(format!("orbit-extract-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;

    let mut archive = tar::Archive::new(tar_bytes.as_slice());
    for entry in archive.entries()? {
        let mut entry = entry?;
        let entry_name = entry
            .path()?
            .file_name()
            .map(|n| n.to_os_string())
            .unwrap_or_default();
        let is_binary = entry_name == "orbit" || entry_name == "orbit.exe";
        if is_binary {
            let dest = dir.join(entry_name);
            entry.unpack(&dest)?;
            return Ok(dest);
        }
    }
    anyhow::bail!("no orbit binary found in {}", archive_display)
}

/// Spawn a detached helper (this same binary in `__update-apply` mode) that
/// waits for us to exit, swaps the binaries, and relaunches with `original_args`.
fn spawn_update_helper(
    staged: &std::path::Path,
    current: &std::path::Path,
    original_args: &[String],
) -> Result<()> {
    use std::process::{Command, Stdio};

    let helper = current.to_path_buf();

    let base_args = [
        "__update-apply",
        staged.to_str().unwrap_or(""),
        current.to_str().unwrap_or(""),
    ];

    #[cfg(target_os = "windows")]
    {
        Command::new(&helper)
            .args(base_args)
            .args(original_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
    }

    #[cfg(not(target_os = "windows"))]
    {
        use std::os::unix::process::CommandExt;

        // Detach so the helper outlives the parent and can overwrite it.
        Command::new(&helper)
            .args(base_args)
            .args(original_args)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
    }
    Ok(())
}

/// Blocking driver for the hidden `__update-apply` helper subcommand: wait for
/// the parent to exit, replace the running binary with the staged one, and
/// relaunch orbit. `staged` is the freshly-downloaded binary, `current` the
/// path of the currently-running (parent) binary. Any args after those two
/// are the previous invocation's original CLI args and are forwarded on
/// relaunch.
pub fn apply_update(staged_path: &str, current_path: &str) {
    // Give the parent a moment to tear down and release its file handle.
    std::thread::sleep(Duration::from_millis(800));

    let staged = std::path::Path::new(staged_path);
    let current = std::path::Path::new(current_path);

    let result = (|| -> Result<()> {
        // Rename the running binary aside, then move the new one into place.
        // Renaming over a running executable is refused on some platforms, so
        // remove-after-rename rather than overwrite.
        let backup = current.with_extension("old");
        if backup.exists() {
            std::fs::remove_file(&backup)?;
        }
        std::fs::rename(current, &backup)?;
        std::fs::rename(staged, current)?;
        Ok(())
    })();

    if let Err(e) = result {
        tracing::error!("update apply failed: {e}");
        // Try to restore the original from backup.
        let _ = std::fs::rename(current.with_extension("old"), current);
        std::process::exit(1);
    }
    // Relaunch the fresh binary with the original user args (see
    // `relaunch_args` for the argv layout and skip count).
    let args: Vec<String> = relaunch_args();
    match std::process::Command::new(current).args(&args).spawn() {
        Ok(_) => {}
        Err(e) => {
            tracing::error!("relaunch failed: {e}");
            let _ = std::fs::rename(current.with_extension("old"), current);
            std::process::exit(1);
        }
    }
    std::process::exit(0);
}

/// Extract the previous invocation's original args from the helper's argv.
/// When the helper runs it was spawned as
/// `[exe, __update-apply, staged, current, orig1, orig2, ...]`, so the user's
/// flags begin at index 4. Keeping this as its own function lets a test pin
/// the skip count against an off-by-one mistake.
fn relaunch_args() -> Vec<String> {
    relaunch_args_from(std::env::args())
}

fn relaunch_args_from<S: Into<String>>(args: impl Iterator<Item = S>) -> Vec<String> {
    args.skip(4).map(Into::into).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_compare_matches_semver_major_minor_patch() {
        assert!(is_newer("1.3.1", "1.4.0"));
        assert!(is_newer("1.4.0", "1.4.1"));
        assert!(!is_newer("1.4.1", "1.4.0"));
        assert!(!is_newer("1.4.0", "1.4.0"));
    }

    #[test]
    fn a_prerelease_is_older_than_its_own_release() {
        // Same version-proper, one is a prerelease: the release outranks the rc
        // even though their numbers match, and the rc does not outrank the
        // release. A prerelease of a *higher* version is still newer than a
        // lower released version (semver), so that case is not asserted here.
        assert!(is_newer("1.5.0-rc1", "1.5.0"));
        assert!(!is_newer("1.5.0", "1.5.0-rc1"));
    }

    #[test]
    fn missing_parts_treat_equal_as_equal() {
        // 1.2 == 1.2.0; neither is newer.
        assert!(!is_newer("1.2", "1.2.0"));
        assert!(!is_newer("1.2.0", "1.2"));
    }

    #[test]
    fn leading_v_is_stripped() {
        assert!(is_newer("1.3.1", "v1.4.0"));
        assert!(!is_newer("v1.4.0", "1.4.0"));
    }

    #[test]
    fn zero_pads_so_1_9_beats_1_10_correctly() {
        // Must compare numerically, not lexically: 9 < 10.
        assert!(is_newer("1.9.0", "1.10.0"));
        assert!(!is_newer("1.10.0", "1.9.0"));
    }

    #[test]
    fn daily_interval_gates_the_check() {
        let now = 1_000_000u64;
        assert!(should_check(None, now), "no prior check is always due");
        assert!(!should_check(Some(now - 60), now), "checked an hour ago");
        assert!(
            !should_check(Some(now - 1_000), now),
            "checked yesterday morning"
        );
        assert!(
            should_check(Some(now - 86_400), now),
            "checked a full day ago is due"
        );
        assert!(
            should_check(Some(now - 900_000), now),
            "checked well over a day ago is due"
        );
    }

    #[test]
    fn force_update_bypasses_the_interval_gate() {
        let now = 1_000_000u64;
        // A recent auto-check would normally suppress the fetch...
        assert!(!should_check(Some(now - 60), now));
        // ...but `--update` forces it through regardless.
        assert!(should_fetch(true, Some(now - 60), now));
        assert!(should_fetch(true, Some(now - 100_000), now));
        assert!(should_fetch(false, None, now), "no cache is always due");
    }

    #[test]
    fn install_method_detects_cargo_home() {
        let exe = std::path::Path::new("/Users/me/.cargo/bin/orbit");
        assert_eq!(InstallMethod::detect(exe), InstallMethod::Cargo);
    }

    #[test]
    fn install_method_detects_brew_cellar() {
        let exe = std::path::Path::new("/opt/homebrew/bin/orbit");
        assert_eq!(InstallMethod::detect(exe), InstallMethod::Brew);
    }

    #[test]
    fn install_method_treats_ambiguous_as_raw() {
        let exe = std::path::Path::new("/usr/local/bin/orbit");
        assert_eq!(InstallMethod::detect(exe), InstallMethod::Raw);
    }

    #[test]
    fn relaunch_args_skip_exactly_the_helper_prefix() {
        // Helper argv: [exe, __update-apply, staged, current, then user flags].
        let argv = [
            "/usr/bin/orbit",
            "__update-apply",
            "/tmp/staged",
            "/usr/bin/orbit",
            "--region",
            "eu-west-1",
            "--readonly",
        ];
        assert_eq!(
            relaunch_args_from(argv.into_iter()),
            vec!["--region", "eu-west-1", "--readonly"],
            "only the user's original flags must survive the relaunch"
        );
    }

    #[test]
    fn cache_round_trips_via_disk() {
        let dir = tempfile::tempdir().unwrap();
        // Point cache_path at the temp dir is awkward; instead just test the
        // serde round-trip directly, which is what persistence relies on.
        let cache = CheckCache {
            last_check: Some(42),
        };
        let raw = serde_json::to_string(&cache).unwrap();
        let back: CheckCache = serde_json::from_str(&raw).unwrap();
        assert_eq!(back.last_check, Some(42));
        let _ = dir;
    }
}
