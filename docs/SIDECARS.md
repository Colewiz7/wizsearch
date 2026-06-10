# SIDECARS

ffmpeg, ffprobe, yt-dlp are first-run-downloaded external binaries, not bundled and
not required from the system.

- Manifest: `src-tauri/sidecars/manifest.json` pins url + sha256 + archive layout per
  platform (linux-x86_64, windows-x86_64). Hashes come from official release
  checksum files (yt-dlp `SHA2-256SUMS`, BtbN FFmpeg `checksums.sha256`).
- Install: stream to `.part` while hashing, compare sha256 BEFORE any use, then
  extract just `bin/ffmpeg` + `bin/ffprobe` from the tarball/zip (suffix match, so
  the versioned top dir doesn't matter) into `<app-data>/sidecars-bin/`, chmod 755.
- The installer refuses entries with empty/`UNPINNED` hashes and non-https URLs.
  A unit test asserts the manifest stays pinned for linux-x86_64.
- Auto-download on first run is a setting (`sidecars.auto_download`, default true);
  there's also a manual "Download missing tools" button in Settings.
- yt-dlp self-update: `yt-dlp -U` (gated by `sidecars.ytdlp_self_update`). This is a
  deliberate exception to pinning because yt-dlp breaks when sites change; the
  pinned manifest still controls fresh installs.
- ffmpeg is currently used for collected-video poster thumbnails
  (`preview/ffmpeg.rs`, best-effort). yt-dlp is installed and updatable but unused
  until the short-video discovery source lands (M2).

## Updating pins
Bump url + sha256 together, from the project's official checksum file. Never hash a
binary you got from anywhere else.
