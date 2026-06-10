# CLAUDE.md — Standing rules for WizSearch

Read this every session. These rules are enforced in code where possible; keep it that way.

## What WizSearch is
A local-first, open-source desktop app (Tauri 2 + React + Rust) for DISCOVERING meme
assets for video editing: sound effects, GIFs/stickers, short video memes, green
screens. The model is "going to myinstants for a sound": the main screen is search,
the app queries several sources at once, and renders one unified previewable grid.
Collecting to the local library is the end of the funnel, not the focus.

## The four principles (enforced in code, not just docs)
1. **Customization** — the user can change almost anything. Every user-facing behavior
   goes in the typed settings registry (`settings::build_registry`) with a default,
   description, and schema; the Settings UI is generated from it. No hardcoded
   tunables.
2. **Open source** — MIT, public GitHub, no shipped secrets of any kind.
3. **Modularity** — sources, search host, pipeline stages, preview/collection backends
   sit behind stable Rust traits (`SearchSource`, `SourceContext`, `SettingsBackend`).
   Core never depends on a concrete source.
4. **Everything has a setting with a sane default.**

## Hard invariants (never violate)
- **Keychain only for secrets.** Per-source API keys and login cookies go through
  `security::` into the OS keychain (Secret Service on Linux, Credential Manager on
  Windows) via the `keyring` crate. NEVER Stronghold (deprecated for Tauri v3),
  never the DB, never settings files. Secrets are write-only from the UI
  (`secret_set`/`secret_exists`/`secret_clear`; there is no read command).
- **No embedded developer keys.** Every `SourceDescriptor.embedded_credential` must be
  empty; `security::assert_no_embedded_credentials` panics at startup otherwise.
- **Rate limits respected.** The only http client sources receive acquires the
  per-source limiter (`search::rate_limit`) before every request.
- **Local-first only.** No hosted backend, no SaaS, no required account.
- **Collected media is git-ignored and never committed.** So are keys and `*.db`.
- **All media playback goes through the `wzstream://` Rust protocol** (Range-capable
  streaming, allowlist-checked remote proxy). Never `new Audio(path)`, never raw
  paths or direct remote URLs in `<audio>`/`<video>`.
- **Sources are pure.** A source implements `SearchSource` and must not import
  shell/fs/db/http-client/keyring/tauri modules. It gets a read-only `SourceContext`
  (injected rate-limited http, read-only credential) and returns `ResultItem`s with
  declarative `FetchPlan`s. Enforced by `src-tauri/tests/source_purity.rs` and
  `scripts/check_source_purity.sh` (both run in CI).
- **Fetches happen only on explicit user selection**, and the host validates every
  URL (preview and fetch plan) against the source's `allowed_hosts` first
  (`security::validate_fetch_plan` / `validate_source_url`, https only).
- **Sidecars are pinned + SHA-256-verified** (`src-tauri/sidecars/manifest.json`);
  the installer refuses unpinned entries. yt-dlp may self-update independently
  (`yt-dlp -U`, gated by a setting).
- **Prefer per-user keys/logins (keychain) and scraping over any shared secret.**
- Keep `SearchSource` stable: AI tagging, external plugins, and the browse-and-capture
  fallback WebView are future layers that must not break existing sources.

## Stack facts
- Tauri 2 shell; React + Vite + TS frontend; Rust core.
- SQLite + FTS5 (rusqlite, bundled) for the LOCAL collection only — remote search is
  live and never stored.
- `bundle.linux.appimage.bundleMediaFramework = true` stays on (Linux media playback).
- Prefer animated WebP/MP4 over raw GIF for previews.
- yt-dlp discovery (future source) is metadata-only:
  `ytsearchN: --skip-download --flat-playlist --dump-single-json`.

## Code conventions
- Casual, terse `//` comments; no docstring walls.
- No em dashes in user-facing strings.
- Typed errors; no `unwrap()` on fallible IO outside tests.
- New user-facing behavior => new entry in `settings::build_registry`, with default
  and description, before the feature merges.
- After changes: `cargo test && cargo clippy --all-targets` in `src-tauri/`,
  `npm run build`, and `scripts/check_source_purity.sh` must all pass.

## Milestones
M1 (done): trait + host orchestration, myinstants/KLIPY/Pexels, unified virtualized
grid, collect-to-SQLite, settings registry + generated UI, keychain secrets, sidecar
download+verify, drag-out/copy-path/reveal.
Explicitly NOT in M1: AI tagging, external/WASM plugins, browse-and-capture WebView,
trend scoring. See docs/ROADMAP.md.
