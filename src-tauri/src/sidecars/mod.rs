//! First-run sidecar management: download pinned ffmpeg/ffprobe/yt-dlp builds,
//! verify SHA-256 before use, refuse unpinned entries. yt-dlp may additionally
//! self-update via `yt-dlp -U` (a setting), independent of the pinned install.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;

const MANIFEST: &str = include_str!("../../sidecars/manifest.json");

#[derive(Debug, thiserror::Error)]
pub enum SidecarError {
    #[error("manifest: {0}")]
    Manifest(String),
    #[error("unsupported platform {0}")]
    UnsupportedPlatform(String),
    #[error("download: {0}")]
    Download(String),
    #[error("checksum mismatch for {tool}: expected {expected}, got {actual}")]
    ChecksumMismatch {
        tool: String,
        expected: String,
        actual: String,
    },
    #[error("'{0}' has no pinned checksum in the manifest; refusing to install")]
    Unpinned(String),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Debug, Deserialize)]
struct Manifest {
    tools: Vec<ToolSpec>,
}

#[derive(Debug, Deserialize)]
struct ToolSpec {
    name: String,
    platforms: std::collections::HashMap<String, PlatformSpec>,
}

#[derive(Debug, Deserialize, Clone)]
struct PlatformSpec {
    url: String,
    sha256: String,
    /// "binary" or "tar.xz" or "zip"
    archive: String,
    /// for archives: archive-internal paths to extract, keyed by output binary name
    #[serde(default)]
    extract: std::collections::HashMap<String, String>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SidecarStatus {
    pub name: String,
    pub installed: bool,
    pub path: Option<String>,
    pub pinned: bool,
}

fn platform_key() -> String {
    let os = if cfg!(target_os = "windows") {
        "windows"
    } else {
        "linux"
    };
    format!("{os}-{}", std::env::consts::ARCH)
}

fn bin_name(name: &str) -> String {
    if cfg!(target_os = "windows") {
        format!("{name}.exe")
    } else {
        name.to_string()
    }
}

fn parse_manifest() -> Result<Manifest, SidecarError> {
    serde_json::from_str(MANIFEST).map_err(|e| SidecarError::Manifest(e.to_string()))
}

pub fn sidecar_dir(app_data: &Path) -> PathBuf {
    app_data.join("sidecars-bin")
}

pub fn tool_path(app_data: &Path, name: &str) -> PathBuf {
    sidecar_dir(app_data).join(bin_name(name))
}

/// which managed binaries each manifest tool provides
fn provided_bins(spec: &PlatformSpec, tool: &str) -> Vec<String> {
    if spec.archive == "binary" {
        vec![tool.to_string()]
    } else {
        spec.extract.keys().cloned().collect()
    }
}

pub fn status(app_data: &Path) -> Result<Vec<SidecarStatus>, SidecarError> {
    let manifest = parse_manifest()?;
    let key = platform_key();
    let mut out = Vec::new();
    for tool in &manifest.tools {
        let plat = tool.platforms.get(&key);
        let pinned = plat
            .map(|p| !p.sha256.is_empty() && p.sha256 != "UNPINNED")
            .unwrap_or(false);
        let bins = plat
            .map(|p| provided_bins(p, &tool.name))
            .unwrap_or_else(|| vec![tool.name.clone()]);
        for bin in bins {
            let p = tool_path(app_data, &bin);
            out.push(SidecarStatus {
                name: bin,
                installed: p.exists(),
                path: p.exists().then(|| p.to_string_lossy().into_owned()),
                pinned,
            });
        }
    }
    Ok(out)
}

/// download + verify + install every missing tool; returns final statuses
pub async fn ensure_all(
    client: &reqwest::Client,
    app_data: &Path,
) -> Result<Vec<SidecarStatus>, SidecarError> {
    let manifest = parse_manifest()?;
    let key = platform_key();
    let dir = sidecar_dir(app_data);
    tokio::fs::create_dir_all(&dir).await?;

    for tool in &manifest.tools {
        let Some(spec) = tool.platforms.get(&key) else {
            log::warn!("no {} build for {key}", tool.name);
            continue;
        };
        let missing = provided_bins(spec, &tool.name)
            .iter()
            .any(|b| !tool_path(app_data, b).exists());
        if !missing {
            continue;
        }
        install_tool(client, app_data, &tool.name, spec).await?;
    }
    status(app_data)
}

async fn install_tool(
    client: &reqwest::Client,
    app_data: &Path,
    name: &str,
    spec: &PlatformSpec,
) -> Result<(), SidecarError> {
    if spec.sha256.is_empty() || spec.sha256 == "UNPINNED" {
        return Err(SidecarError::Unpinned(name.to_string()));
    }
    if !spec.url.starts_with("https://") {
        return Err(SidecarError::Download("https only".into()));
    }

    log::info!("downloading sidecar {name} from {}", spec.url);
    let dir = sidecar_dir(app_data);
    let tmp = dir.join(format!("{name}.download.part"));

    // stream to disk while hashing
    let resp = client
        .get(&spec.url)
        .send()
        .await
        .map_err(|e| SidecarError::Download(e.to_string()))?;
    if !resp.status().is_success() {
        return Err(SidecarError::Download(format!("http {}", resp.status())));
    }
    let mut hasher = Sha256::new();
    {
        let mut file = tokio::fs::File::create(&tmp).await?;
        let mut resp = resp;
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| SidecarError::Download(e.to_string()))?
        {
            hasher.update(&chunk);
            file.write_all(&chunk).await?;
        }
        file.flush().await?;
    }
    let actual = hex::encode(hasher.finalize());
    if actual != spec.sha256.to_lowercase() {
        let _ = tokio::fs::remove_file(&tmp).await;
        return Err(SidecarError::ChecksumMismatch {
            tool: name.to_string(),
            expected: spec.sha256.clone(),
            actual,
        });
    }

    // verified; unpack or move into place
    match spec.archive.as_str() {
        "binary" => {
            let dest = tool_path(app_data, name);
            tokio::fs::rename(&tmp, &dest).await?;
            make_executable(&dest)?;
        }
        "tar.xz" => {
            let spec = spec.clone();
            let dir = dir.clone();
            let tmp2 = tmp.clone();
            tokio::task::spawn_blocking(move || extract_tar_xz(&tmp2, &dir, &spec))
                .await
                .map_err(|e| SidecarError::Download(e.to_string()))??;
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        "zip" => {
            let spec = spec.clone();
            let dir = dir.clone();
            let tmp2 = tmp.clone();
            tokio::task::spawn_blocking(move || extract_zip(&tmp2, &dir, &spec))
                .await
                .map_err(|e| SidecarError::Download(e.to_string()))??;
            let _ = tokio::fs::remove_file(&tmp).await;
        }
        other => {
            return Err(SidecarError::Manifest(format!(
                "unknown archive kind {other}"
            )))
        }
    }
    Ok(())
}

fn make_executable(path: &Path) -> Result<(), SidecarError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755))?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

/// pull only the entries listed in spec.extract out of the tarball; entry paths
/// match by suffix so the versioned top-level dir doesn't matter
fn extract_tar_xz(archive: &Path, dir: &Path, spec: &PlatformSpec) -> Result<(), SidecarError> {
    let file = std::fs::File::open(archive)?;
    let decompressed = xz2::read::XzDecoder::new(file);
    let mut tar = tar::Archive::new(decompressed);
    let mut remaining: std::collections::HashMap<String, String> = spec.extract.clone();

    for entry in tar.entries()? {
        let mut entry = entry?;
        let path = entry.path()?.to_string_lossy().into_owned();
        let hit = remaining
            .iter()
            .find(|(_, suffix)| path.ends_with(suffix.as_str()))
            .map(|(k, _)| k.clone());
        if let Some(out_name) = hit {
            let dest = dir.join(bin_name(&out_name));
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
            make_executable(&dest)?;
            remaining.remove(&out_name);
            if remaining.is_empty() {
                break;
            }
        }
    }
    if !remaining.is_empty() {
        return Err(SidecarError::Manifest(format!(
            "archive missing entries: {:?}",
            remaining.values().collect::<Vec<_>>()
        )));
    }
    Ok(())
}

fn extract_zip(archive: &Path, dir: &Path, spec: &PlatformSpec) -> Result<(), SidecarError> {
    let file = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| SidecarError::Manifest(e.to_string()))?;
    let mut remaining = spec.extract.clone();

    for i in 0..zip.len() {
        let mut entry = zip
            .by_index(i)
            .map_err(|e| SidecarError::Manifest(e.to_string()))?;
        let path = entry.name().to_string();
        let hit = remaining
            .iter()
            .find(|(_, suffix)| path.ends_with(suffix.as_str()))
            .map(|(k, _)| k.clone());
        if let Some(out_name) = hit {
            let dest = dir.join(bin_name(&out_name));
            let mut out = std::fs::File::create(&dest)?;
            std::io::copy(&mut entry, &mut out)?;
            make_executable(&dest)?;
            remaining.remove(&out_name);
            if remaining.is_empty() {
                break;
            }
        }
    }
    if !remaining.is_empty() {
        return Err(SidecarError::Manifest(format!(
            "archive missing entries: {:?}",
            remaining.values().collect::<Vec<_>>()
        )));
    }
    Ok(())
}

/// `yt-dlp -U`: self-update independent of the pinned install (sites change,
/// yt-dlp rots fast). Gated by the sidecars.ytdlp_self_update setting.
pub async fn ytdlp_self_update(app_data: &Path) -> Result<String, SidecarError> {
    let bin = tool_path(app_data, "yt-dlp");
    if !bin.exists() {
        return Err(SidecarError::Download("yt-dlp is not installed yet".into()));
    }
    let output = tokio::process::Command::new(&bin)
        .arg("-U")
        .output()
        .await
        .map_err(|e| SidecarError::Download(e.to_string()))?;
    let text = String::from_utf8_lossy(&output.stdout).into_owned()
        + &String::from_utf8_lossy(&output.stderr);
    Ok(text.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifest_parses_and_is_pinned_for_linux() {
        let m = parse_manifest().expect("manifest must parse");
        assert!(!m.tools.is_empty());
        for tool in &m.tools {
            let plat = tool
                .platforms
                .get("linux-x86_64")
                .unwrap_or_else(|| panic!("{} missing linux-x86_64", tool.name));
            assert!(
                plat.url.starts_with("https://"),
                "{} url must be https",
                tool.name
            );
            assert_eq!(
                plat.sha256.len(),
                64,
                "{} linux sha256 must be a real pinned hash",
                tool.name
            );
        }
    }
}
