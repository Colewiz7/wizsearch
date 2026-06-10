# TESTING

Run before every merge:
```bash
cd src-tauri && cargo test && cargo clippy --all-targets
npm run build              # tsc + vite
bash scripts/check_source_purity.sh
```

## What's covered (and the invariant each test guards)
- `tests/source_purity.rs` — sources import no shell/fs/db/http/keyring/tauri
  (modularity, enforced in code). Mirrored by the CI grep script.
- `security::tests` — allowlist suffix matching, https-only, userinfo/port rejection,
  auth-header smuggling in fetch plans, empty embedded credentials.
- `settings::tests` — defaults, schema validation rejects bad values, secrets cannot
  pass through the settings store.
- `collection::tests` — collect -> FTS hit, sha256 dedupe returns the existing asset,
  tag edits update FTS, delete removes rows and files.
- `search::tests` + `rate_limit::tests` — merge strategies; limiter blocks after the
  window fills (virtual time).
- `sources::*::tests` — parsers against canned myinstants HTML / KLIPY JSON /
  Pexels JSON, including chosen preview vs fetch URLs.
- `sidecars::tests` — manifest parses, https-only, linux hashes pinned (64 hex).
- `preview::tests` — Range header parsing, query decoding.

## Conventions
- Source parsers are pure functions; test them with canned payloads, no network.
- Network/scrape shape drift shows up as a source error chip at runtime, not a crash;
  when a site changes, update the canned payload with the new real shape and fix the
  parser against it.
- DB tests run on in-memory SQLite; file tests use a temp dir they clean up.
