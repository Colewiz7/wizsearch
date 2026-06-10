# SECURITY

## Secrets
- Per-source API keys and login cookies live in the OS keychain only: Secret Service
  (libsecret) on Linux, Credential Manager on Windows, via the `keyring` crate.
  NEVER Stronghold (deprecated for Tauri v3), never SQLite, never JSON/env files.
- The UI is write-only for secrets: `secret_set` / `secret_exists` / `secret_clear`.
  There is intentionally no IPC command that returns a secret value.
- `secret_set` only accepts keys registered as Secret settings in the registry.
- No developer key ever ships: `SourceDescriptor.embedded_credential` must be empty
  and `security::assert_no_embedded_credentials` panics at startup otherwise.

## Network containment
- Sources can only reach hosts on their descriptor's `allowed_hosts` (suffix match,
  https only, no userinfo, no custom ports). This applies to all three network paths:
  source API calls (`HostHttp::get`), preview proxying (`/remote`), and fetch-plan
  execution (`collect_item`).
- Fetch plans may not smuggle `Authorization`/`Cookie` headers.
- Downloads only happen on explicit user selection (Collect). Search never fetches
  full assets.

## Filesystem containment
- `wzstream://localhost/local` canonicalizes and refuses anything outside the
  configured collection dir.
- Stored filenames are `uid-prefix + sanitized stem`; untrusted metadata never forms
  a path.
- Deletion only removes files under the collection dir.

## Supply chain
- Sidecars (ffmpeg/ffprobe/yt-dlp) are pinned by URL + SHA-256 in
  `src-tauri/sidecars/manifest.json`; hashes come from each project's official
  release checksum files. The installer refuses unpinned entries and https-only.
- yt-dlp self-update (`-U`) is allowed (gated by a setting) because yt-dlp rots fast;
  that is an explicit, documented exception to pinning.
- Process args are built programmatically (no shell interpolation).

## Rate limiting
- Per-source sliding-window limiter; the only http client a source receives acquires
  it before every request, so a source cannot bypass it.
