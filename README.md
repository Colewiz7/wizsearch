# WizSearch

A local-first, open-source desktop app for **discovering** meme assets for video
editing: sound effects, GIFs/stickers, short video memes, and green screens.

Think "going to myinstants for a sound", but one search box queries several sources
at once and renders a single previewable grid: audio plays inline, GIFs and videos
loop on hover, green screens show poster + loop. When something is worth keeping you
collect it into a local, FTS-searchable library and drag it straight into your
editor. Discovery is the app; collecting is the end of the funnel.

> Linux x86_64 first, Windows x86_64 second. Everything runs on your machine.
> No cloud, no account, no telemetry.

## Principles (enforced in code, not just docs)
- **Customization** — change almost anything; every behavior is a typed setting with
  a sane default and a UI control generated from its schema.
- **Open source** — MIT, no shipped secrets. The app asserts at startup that no
  source carries an embedded credential.
- **Modularity** — sources implement one pure async trait (`SearchSource`); the
  search host, settings persistence, and preview/collection backends sit behind
  stable traits too. A purity test fails the build if a source touches IO directly.
- **Everything has a setting** with a sane default.

## Sources (M1)
| | | |
|---|---|---|
| Myinstants | sounds | scraped, no key |
| KLIPY | GIFs, stickers, clips | your own free key |
| Pexels | green screens, stock video | your own free key |

WizSearch ships **zero** API keys. Key-based sources need your personal free key,
pasted once in Settings and stored in your OS keychain (Secret Service / Credential
Manager) — never on disk.

## Stack
Tauri 2 · React + Vite + TypeScript · Rust · SQLite + FTS5 (local collection only) ·
ffmpeg/ffprobe/yt-dlp downloaded on first run and SHA-256-verified against pinned
hashes.

## Building from source
Requires Rust, Node 20+, and the [Tauri 2 prerequisites](https://tauri.app/start/prerequisites/) for your OS.
```bash
npm install
npm run tauri dev      # dev
npm run tauri build    # release bundles (AppImage/deb/NSIS)
```

## Checks
```bash
cd src-tauri && cargo test && cargo clippy --all-targets
npm run build
bash scripts/check_source_purity.sh
```

## Boundaries
WizSearch searches public endpoints and downloads only what you explicitly select,
from each source's allowlisted hosts. It does not bypass private, login-only, DRM, or
creator-disabled content. Licenses and attribution are captured per item and shown in
the UI; you are responsible for honoring them in your projects.

## Contributing
Read `CLAUDE.md` and `/docs` first. New tunables go in the settings registry; new
sources implement `SearchSource` (see `docs/SOURCES.md` for the checklist).

## License
MIT (see `LICENSE`).
