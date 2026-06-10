//! Poster thumbnail generation for collected videos via the ffmpeg sidecar.
//! Best-effort: collection works fine without it.

use std::path::{Path, PathBuf};

/// grab a frame ~0.5s in, scale to 480w, write webp next to the asset
pub async fn make_thumbnail(
    ffmpeg: &Path,
    input: &Path,
    collection_dir: &Path,
    uid: &str,
) -> Result<String, String> {
    let rel = format!(".thumbs/{uid}.webp");
    let out: PathBuf = collection_dir.join(&rel);
    if let Some(parent) = out.parent() {
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| e.to_string())?;
    }

    // args built programmatically, no shell
    let status = tokio::process::Command::new(ffmpeg)
        .arg("-y")
        .arg("-ss")
        .arg("0.5")
        .arg("-i")
        .arg(input)
        .arg("-frames:v")
        .arg("1")
        .arg("-vf")
        .arg("scale=480:-2")
        .arg(&out)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| e.to_string())?;

    if status.success() && out.exists() {
        Ok(rel)
    } else {
        Err(format!("ffmpeg exited with {status}"))
    }
}
