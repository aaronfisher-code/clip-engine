use anyhow::{bail, Context};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;
use tokio::io::AsyncWriteExt;

const DEFAULT_REPO: &str = "aaronfisher-code/clip-engine";
const CHECK_COOLDOWN: Duration = Duration::from_secs(2 * 60 * 60);
const SETTING_CHECKED_AT: &str = "update_checked_at";
const SETTING_CACHED: &str = "update_cached_release";
const SETTING_SNOOZED: &str = "update_snoozed_version";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdatePackage {
    Nsis,
    AppImage,
    Deb,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AvailableUpdate {
    pub version: String,
    pub notes: String,
    pub html_url: String,
    pub asset_name: String,
    pub download_url: String,
    pub size: u64,
    pub package: UpdatePackage,
}

#[derive(Deserialize)]
struct GithubRelease {
    tag_name: String,
    html_url: String,
    body: Option<String>,
    #[serde(default)]
    draft: bool,
    #[serde(default)]
    prerelease: bool,
    #[serde(default)]
    assets: Vec<GithubAsset>,
}

#[derive(Deserialize)]
struct GithubAsset {
    name: String,
    browser_download_url: String,
    size: u64,
}

#[derive(Clone)]
pub struct UpdateClient {
    client: reqwest::Client,
    repo: String,
}

impl UpdateClient {
    pub fn new() -> anyhow::Result<Self> {
        let repo = std::env::var("CLIP_ENGINE_GITHUB_REPO").unwrap_or_else(|_| DEFAULT_REPO.into());
        Ok(Self {
            client: reqwest::Client::builder()
                .user_agent(format!("clip-engine/{}", env!("CARGO_PKG_VERSION")))
                .build()?,
            repo,
        })
    }

    pub async fn latest(&self, current_version: &str) -> anyhow::Result<Option<AvailableUpdate>> {
        let url = format!(
            "https://api.github.com/repos/{}/releases/latest",
            self.repo.trim_matches('/')
        );
        let response = self
            .client
            .get(url)
            .header("Accept", "application/vnd.github+json")
            .header("X-GitHub-Api-Version", "2022-11-28")
            .send()
            .await
            .context("Could not reach GitHub Releases")?;
        if response.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !response.status().is_success() {
            bail!(
                "GitHub Releases returned HTTP {}.",
                response.status().as_u16()
            );
        }
        let release = response.json::<GithubRelease>().await?;
        available_from_release(&release, current_version, current_os(), current_arch())
    }

    pub async fn download(
        &self,
        update: &AvailableUpdate,
        destination: &Path,
        mut progress: impl FnMut(u64, u64),
    ) -> anyhow::Result<PathBuf> {
        if let Some(parent) = destination.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let response = self
            .client
            .get(&update.download_url)
            .send()
            .await
            .context("Could not download the update")?;
        if !response.status().is_success() {
            bail!(
                "Downloading the update failed with HTTP {}.",
                response.status().as_u16()
            );
        }
        let total = response.content_length().unwrap_or(update.size).max(1);
        let mut file = tokio::fs::File::create(destination).await?;
        let mut received = 0_u64;
        let mut stream = response;
        while let Some(chunk) = stream.chunk().await? {
            file.write_all(&chunk).await?;
            received += chunk.len() as u64;
            progress(received, total);
        }
        file.flush().await?;
        Ok(destination.to_path_buf())
    }
}

impl crate::Engine {
    pub fn current_version() -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    pub fn snoozed_update_version(&self) -> Option<String> {
        self.database.setting(SETTING_SNOOZED).ok().flatten()
    }

    pub fn snooze_update(&self, version: &str) -> anyhow::Result<()> {
        self.database.put_setting(SETTING_SNOOZED, version)
    }

    pub async fn check_desktop_update(
        &self,
        force: bool,
    ) -> anyhow::Result<Option<AvailableUpdate>> {
        let current = Self::current_version();
        if !force {
            if let Some(cached) = cached_if_fresh(&self.database, current)? {
                return Ok(cached);
            }
        }
        let found = UpdateClient::new()?.latest(current).await?;
        self.database
            .put_setting(SETTING_CHECKED_AT, &chrono::Utc::now().to_rfc3339())?;
        self.database.put_setting(
            SETTING_CACHED,
            &serde_json::to_string(&found).unwrap_or_else(|_| "null".into()),
        )?;
        Ok(found)
    }

    pub async fn download_desktop_update(
        &self,
        update: &AvailableUpdate,
        progress: impl FnMut(u64, u64),
    ) -> anyhow::Result<PathBuf> {
        let path = update_download_path(&self.paths.data, update)?;
        UpdateClient::new()?.download(update, &path, progress).await
    }
}

pub fn install_desktop_update(path: &Path, package: UpdatePackage) -> anyhow::Result<()> {
    match package {
        UpdatePackage::Nsis => install_nsis(path),
        UpdatePackage::AppImage => install_appimage(path),
        UpdatePackage::Deb => open_installer(path),
    }
}

fn cached_if_fresh(
    database: &crate::database::Database,
    current: &str,
) -> anyhow::Result<Option<Option<AvailableUpdate>>> {
    let Some(checked) = database.setting(SETTING_CHECKED_AT)? else {
        return Ok(None);
    };
    let checked = chrono::DateTime::parse_from_rfc3339(&checked)
        .ok()
        .map(|value| value.with_timezone(&chrono::Utc));
    let Some(checked) = checked else {
        return Ok(None);
    };
    if chrono::Utc::now() - checked
        < chrono::Duration::from_std(CHECK_COOLDOWN).unwrap_or(chrono::Duration::hours(2))
    {
        if let Some(payload) = database.setting(SETTING_CACHED)? {
            if let Ok(cached) = serde_json::from_str::<Option<AvailableUpdate>>(&payload) {
                if let Some(update) = &cached {
                    if !is_newer(&update.version, current) {
                        return Ok(Some(None));
                    }
                }
                return Ok(Some(cached));
            }
        }
        return Ok(None);
    }
    Ok(None)
}

fn available_from_release(
    release: &GithubRelease,
    current_version: &str,
    os: &str,
    arch: &str,
) -> anyhow::Result<Option<AvailableUpdate>> {
    if release.draft || release.prerelease {
        return Ok(None);
    }
    let Some(version) = parse_version_tag(&release.tag_name) else {
        bail!("The GitHub release tag is not a version.");
    };
    if !is_newer(&version, current_version) {
        return Ok(None);
    }
    let asset = select_asset(&release.assets, os, arch)
        .ok_or_else(|| anyhow::anyhow!("No installer is attached for this computer."))?;
    let package = package_for_asset(&asset.name)
        .ok_or_else(|| anyhow::anyhow!("The GitHub asset is not a known installer format."))?;
    Ok(Some(AvailableUpdate {
        version,
        notes: release.body.clone().unwrap_or_default(),
        html_url: release.html_url.clone(),
        asset_name: asset.name.clone(),
        download_url: asset.browser_download_url.clone(),
        size: asset.size,
        package,
    }))
}

fn parse_version_tag(tag: &str) -> Option<String> {
    let trimmed = tag.trim().trim_start_matches('v');
    semver::Version::parse(trimmed).ok()?;
    Some(trimmed.to_string())
}

fn is_newer(remote: &str, current: &str) -> bool {
    let Ok(remote) = semver::Version::parse(remote.trim().trim_start_matches('v')) else {
        return false;
    };
    let Ok(current) = semver::Version::parse(current.trim().trim_start_matches('v')) else {
        return false;
    };
    remote > current
}

fn current_os() -> &'static str {
    std::env::consts::OS
}

fn current_arch() -> &'static str {
    std::env::consts::ARCH
}

fn package_for_asset(name: &str) -> Option<UpdatePackage> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".exe") && !lower.ends_with(".sig") {
        Some(UpdatePackage::Nsis)
    } else if lower.ends_with(".appimage") {
        Some(UpdatePackage::AppImage)
    } else if lower.ends_with(".deb") {
        Some(UpdatePackage::Deb)
    } else {
        None
    }
}

fn select_asset<'a>(assets: &'a [GithubAsset], os: &str, arch: &str) -> Option<&'a GithubAsset> {
    let mut ranked = assets
        .iter()
        .filter(|asset| asset_matches(asset, os, arch))
        .collect::<Vec<_>>();
    ranked.sort_by_key(|asset| asset_rank(&asset.name, os));
    ranked.into_iter().next()
}

fn asset_matches(asset: &GithubAsset, os: &str, arch: &str) -> bool {
    let name = asset.name.to_ascii_lowercase();
    if name.ends_with(".sig") || name.ends_with(".json") || name.contains("checksum") {
        return false;
    }
    let os_ok = match os {
        "windows" => name.ends_with(".exe"),
        "linux" => name.ends_with(".appimage") || name.ends_with(".deb"),
        _ => false,
    };
    os_ok && arch_matches(&name, arch)
}

fn arch_matches(name: &str, arch: &str) -> bool {
    match arch {
        "x86_64" => {
            name.contains("x64")
                || name.contains("x86_64")
                || name.contains("amd64")
                || name.contains("win64")
                || name.contains("linux64")
                || !(name.contains("arm") || name.contains("aarch64") || name.contains("i686"))
        }
        "aarch64" => name.contains("arm64") || name.contains("aarch64"),
        _ => true,
    }
}

fn asset_rank(name: &str, os: &str) -> u8 {
    let lower = name.to_ascii_lowercase();
    if (os == "linux" && lower.ends_with(".appimage"))
        || (os == "windows" && lower.contains("setup"))
    {
        0
    } else {
        1
    }
}

fn update_download_path(data: &Path, update: &AvailableUpdate) -> anyhow::Result<PathBuf> {
    let directory = data.join("updates");
    std::fs::create_dir_all(&directory)?;
    Ok(directory.join(&update.asset_name))
}

fn install_nsis(path: &Path) -> anyhow::Result<()> {
    let mut command = std::process::Command::new(path);
    command.args(["/S", "/NS", "/UPDATE"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x00000008 | 0x00000200);
    }
    command
        .spawn()
        .context("Could not start the Windows installer")?;
    Ok(())
}

fn install_appimage(path: &Path) -> anyhow::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = std::fs::metadata(path)?.permissions();
        permissions.set_mode(0o755);
        std::fs::set_permissions(path, permissions)?;
    }
    if let Ok(current) = std::env::var("APPIMAGE") {
        if !current.is_empty() {
            let script = format!(
                "sleep 1; mv -f {} {} && chmod +x {} && exec {}",
                sh_quote(&path.display().to_string()),
                sh_quote(&current),
                sh_quote(&current),
                sh_quote(&current)
            );
            std::process::Command::new("sh")
                .args(["-c", &script])
                .spawn()
                .context("Could not replace the running AppImage")?;
            return Ok(());
        }
    }
    open_installer(path)
}

fn open_installer(path: &Path) -> anyhow::Result<()> {
    let opener = if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    std::process::Command::new(opener)
        .arg(path)
        .spawn()
        .context("Could not open the downloaded installer")?;
    Ok(())
}

fn sh_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\"'\"'"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn asset(name: &str) -> GithubAsset {
        GithubAsset {
            name: name.into(),
            browser_download_url: format!("https://example.test/{name}"),
            size: 12,
        }
    }

    #[test]
    fn newer_semver_tags_qualify() {
        assert!(is_newer("1.0.1", "1.0.0"));
        assert!(is_newer("v1.2.0", "1.1.9"));
        assert!(!is_newer("1.0.0", "1.0.0"));
        assert!(!is_newer("1.0.0", "1.0.1"));
    }

    #[test]
    fn parse_strips_v_prefix() {
        assert_eq!(parse_version_tag("v1.0.1").as_deref(), Some("1.0.1"));
        assert_eq!(parse_version_tag("1.0.1").as_deref(), Some("1.0.1"));
        assert!(parse_version_tag("nightly").is_none());
    }

    #[test]
    fn windows_picks_nsis_setup() {
        let assets = vec![
            asset("Clip Engine_1.0.1_amd64.AppImage"),
            asset("Clip Engine_1.0.1_x64-setup.exe"),
            asset("Clip Engine_1.0.1_amd64.deb"),
            asset("latest.json"),
        ];
        let selected = select_asset(&assets, "windows", "x86_64").unwrap();
        assert!(selected.name.ends_with("-setup.exe"));
        assert_eq!(package_for_asset(&selected.name), Some(UpdatePackage::Nsis));
    }

    #[test]
    fn linux_prefers_appimage_over_deb() {
        let assets = vec![
            asset("Clip Engine_1.0.1_amd64.deb"),
            asset("Clip Engine_1.0.1_amd64.AppImage"),
            asset("Clip Engine_1.0.1_x64-setup.exe"),
        ];
        let selected = select_asset(&assets, "linux", "x86_64").unwrap();
        assert!(selected.name.ends_with(".AppImage"));
        assert_eq!(
            package_for_asset(&selected.name),
            Some(UpdatePackage::AppImage)
        );
    }

    #[test]
    fn release_becomes_update_when_newer() {
        let release = GithubRelease {
            tag_name: "v1.2.0".into(),
            html_url: "https://github.com/aaronfisher-code/clip-engine/releases/tag/v1.2.0".into(),
            body: Some("Fixes playback.".into()),
            draft: false,
            prerelease: false,
            assets: vec![asset("Clip Engine_1.2.0_x64-setup.exe")],
        };
        let update = available_from_release(&release, "1.0.0", "windows", "x86_64")
            .unwrap()
            .unwrap();
        assert_eq!(update.version, "1.2.0");
        assert_eq!(update.notes, "Fixes playback.");
        assert_eq!(update.package, UpdatePackage::Nsis);
    }

    #[test]
    fn same_version_is_not_an_update() {
        let release = GithubRelease {
            tag_name: "v1.0.0".into(),
            html_url: "https://example.test".into(),
            body: None,
            draft: false,
            prerelease: false,
            assets: vec![asset("Clip Engine_1.0.0_x64-setup.exe")],
        };
        assert!(
            available_from_release(&release, "1.0.0", "windows", "x86_64")
                .unwrap()
                .is_none()
        );
    }
}
