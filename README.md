# OpenCut

Agentic video editor in Rust. Browser UI, Rust engine, cloud storage on R2, Postgres on Supabase.

## Layout

```
crates/oc-time         integer tick clock (120_000 / sec)
crates/oc-timeline     tracks, clips, captions, undo
crates/oc-tools        UI tools, timeline ops, MCP schemas for providers
crates/oc-compositor   frame planner (wgpu later)
crates/oc-render       ffmpeg bake (xfade, titles, grade, mix, captions)
crates/oc-media        probe / object-key helpers
crates/oc-providers    AI providers (SpaceXAI / xAI)
crates/oc-voice        Groq Whisper, local fallback
crates/oc-core         re-exports the editor crates
crates/oc-db           Postgres + object storage (R2)
apps/oc-api            Axum
apps/oc-worker         transcribe / proxy / export jobs
apps/oc-web            Dioxus UI
```

## Run

```bash
cp .env.example .env
# fill DATABASE_URL, R2_*, and GROQ_API_KEY. Director chat uses a signed-in `claude`, `grok`, or `codex`.
# B-roll and motion design use the `grok` sign-in. No xAI API key.

cargo run -p oc-api
cargo run -p oc-worker
dx serve --package oc-web   # or: cargo check -p oc-web --target wasm32-unknown-unknown
```

Apply schema:

```bash
cargo run -p oc-api          # runs sqlx migrations on boot
# or: sqlx migrate run --source crates/oc-db/migrations
```

## v0

Import → timeline (split / trim / undo) → Groq transcript → agent ops → export.
