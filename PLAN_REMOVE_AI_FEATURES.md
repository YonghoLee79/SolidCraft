# Plan: Remove AI features from SolidCraft

**Branch:** `remove-ai-features` (created from `main`)
**Fork remote:** `fork` → `https://github.com/dinujohnk/3D_model_app.git`
**Status:** planning (nothing removed yet)

## Scope

Remove everything that lets an AI agent control or chat with SolidCraft:

1. **The Model Context Protocol (MCP) server** — `crates/mcp/` (`solidcraft-mcp`), the `solidcraft-cli mcp` subcommand, and the `mcp` workspace dependency.
2. **The agent chat panel** — `crates/ui-egui/src/agent_chat/` and every UI/app/CLI integration point that references it.
3. **Agent skills** — `skills/`.
4. **ACP test agent example** — `crates/ui-egui/examples/acp_test_agent.rs`.
5. **Documentation** — `docs/mcp.md`, `docs/agent-chat.md`, and agent/MCP references in other docs.

Out of scope (stay): the core modelling engine, sketch solver, kernel, file formats, viewport, UI commands, CLI `run`/`eval`/`snapshot`/`render`/`commands`, STEP/IGES/3MF import-export, crash recovery, etc.

## Inventory of AI-related code

### Crate: `solidcraft-mcp` (`crates/mcp/`) — L5, only used by `solidcraft-cli`

| File | Role |
|---|---|
| `crates/mcp/src/lib.rs` | MCP server entry, temp-dir handling, path scoping |
| `crates/mcp/src/server.rs` | stdio JSON-RPC server, `Server`, `Backend` trait |
| `crates/mcp/src/backend.rs` | `Headless`, `Remote` backends, command dispatch |
| `crates/mcp/src/tools.rs` | MCP tool definitions (`execute`, `batch`, `screenshot`, …) |
| `crates/mcp/src/tests.rs` | MCP server tests |

Referenced by: `apps/solidcraft-cli/Cargo.toml`, root `Cargo.toml` workspace dep.

### Agent chat UI (`crates/ui-egui/src/agent_chat/`) — L5

| File | Role |
|---|---|
| `mod.rs` | `AgentChat` state, `show`, `take_drops`, `toggle`, `shown` |
| `panel.rs` | chat panel rendering |
| `acp.rs` / `acp_client.rs` | Agent Client Protocol client |
| `agents.rs` | `AgentConfig`, `Mode`, agent discovery |
| `api_client.rs` | OpenAI-compatible HTTP API client |
| `attach.rs` | attachment handling |
| `backend.rs` | `HttpFn`, `Secrets` traits |
| `discover.rs` | agent discovery |
| `gate.rs` | turn-limit gate |
| `markdown.rs` / `markdown_view.rs` | markdown rendering |
| `paste.rs` | paste handling |
| `process.rs` | agent process management |
| `settings.rs` | agent chat settings |
| `tests.rs` | agent chat tests |

### UI integration (`crates/ui-egui/src/`)

| File | References |
|---|---|
| `lib.rs` | `pub mod agent_chat`; `App.agent_chat`, `agent_http`, `secrets`, `take_drops`, `show` |
| `control.rs` | `ui.agent_chat` command |
| `toolbar.rs` | AI button (`AI_BUTTON_ROOM`) |
| `viewport.rs` | agent chat panel toggle + scenario publish |
| `prefs.rs` | `agents`, `ai_permissions`, `agent_permissions`, `agent_chat_side/width/minimized/popped/window/turn_limit` |
| `icon_svg.rs` | (check for AI/agent icons) |

### App (`apps/solidcraft/src/main.rs`)

| Line(s) | Role |
|---|---|
| 93 | `agent_chat::shutdown` on app drop |
| 172 | agent chat attachments file filter |
| 213 | `agent_http` hooked into `App` |
| 274–296 | `agent_http` function (HTTP calls to agent API endpoints) |
| 308–317 | `Keychain` implements `agent_chat::backend::Secrets` |
| 631–634 | `control_starter` for the agent MCP bridge |

### CLI (`apps/solidcraft-cli/src/main.rs`)

| Line(s) | Role |
|---|---|
| 15, 74 | `mcp` subcommand help text |
| 115 | `Some("mcp") => cmd_mcp(...)` |
| 536–590 | `cmd_mcp`, `agent_scope`, `--connect`/`--in`/`--allow-path` |

### CLI tests

| File | Role |
|---|---|
| `apps/solidcraft-cli/tests/mcp_connect.rs` | `mcp --connect` behavior |
| `apps/solidcraft-cli/tests/snapshot_camera.rs` | uses `solidcraft_mcp::{Headless, Server}` |

### Example

| File | Role |
|---|---|
| `crates/ui-egui/examples/acp_test_agent.rs` | ACP test agent example |

### Skills

| Path | Role |
|---|---|
| `skills/README.md` | Agent skills install instructions |
| `skills/solidcraft-checking/` | agent skill |
| `skills/solidcraft-editing/` | agent skill |
| `skills/solidcraft-mechanisms/` | agent skill |
| `skills/solidcraft-modeling/` | agent skill |

### Documentation

| File | Role |
|---|---|
| `docs/mcp.md` | MCP server user guide |
| `docs/agent-chat.md` | agent chat user guide (referenced) |
| `docs/capabilities.md` | mentions MCP server |
| `docs/perf.md` | `bench-agent` references |
| `docs/building.md` | agent browser control mention |
| `docs/browser-api.md` | agent JS access mention |
| `docs/crash-reports.md` | "For agents" section |
| `ROADMAP.md` | agent chat feature description |
| `AGENTS.md` | MCP server name mention |

## Dependency analysis (safe removal order)

The layering rules (`cargo xtask layers`, `crates/xtask/src/layers.rs`) place `mcp` at **L5** (app layer). Nothing below L5 depends on it:

- `solidcraft-mcp` is depended on **only** by `solidcraft-cli` (and declared in root `Cargo.toml`).
- `crates/engine`, `crates/doc`, `crates/kernel`, `crates/sketch`, `crates/geom`, `crates/render`, `crates/gpu`, `crates/io`, `crates/derive`, `crates/space-mouse`, `crates/threedx-client` — none depend on `solidcraft-mcp`.
- `crates/ui-egui` does **not** depend on `solidcraft-mcp`; it only implements the `agent_chat` UI module.
- `apps/solidcraft` depends on `solidcraft-ui-egui` (which has agent_chat) — not on mcp directly.

Removal order (dependency-safe):

1. **Phase 1 — Docs & skills** (no code impact): remove `skills/`, `docs/mcp.md`, `docs/agent-chat.md`, agent mentions in other docs.
2. **Phase 2 — MCP crate & CLI subcommand**: delete `crates/mcp/`, remove `solidcraft-mcp` from CLI `Cargo.toml` + root `Cargo.toml`, remove `cmd_mcp`/`mcp` subcommand from CLI `main.rs`, delete MCP CLI tests (`mcp_connect.rs`, `snapshot_camera.rs`).
3. **Phase 3 — Agent chat UI module**: delete `crates/ui-egui/src/agent_chat/`, remove `pub mod agent_chat` from `lib.rs`.
4. **Phase 4 — UI integration**: remove agent_chat field from `App` in `lib.rs`, remove `ui.agent_chat` from `control.rs`, remove AI button from `toolbar.rs`, remove panel toggle from `viewport.rs`, remove agent chat prefs from `prefs.rs`, remove AI icons from `icon_svg.rs`.
5. **Phase 5 — App integration**: remove `agent_chat::shutdown`, `agent_http`, `Keychain::Secrets`, `control_starter`, attachments filter from `apps/solidcraft/src/main.rs`.
6. **Phase 6 — Example**: delete `crates/ui-egui/examples/acp_test_agent.rs`.
7. **Phase 7 — Build & verify**: `cargo build --release -p solidcraft -p solidcraft-cli`, `cargo xtask ci` subset (fmt, clippy on touched crates, ui-egui + cli tests).

## What breaks / needs care

- `crates/ui-egui/src/lib.rs` `App` struct has an `agent_chat: AgentChat` field — removing it touches construction (`Default::default()`), `take_drops`, `show`, and `agent_http`/`secrets` fields.
- `apps/solidcraft/src/main.rs` constructs `App` and wires `agent_http`, `secrets`, `control_starter`.
- `crates/ui-egui/src/control.rs` registers `ui.agent_chat` — remove that arm.
- `crates/ui-egui/src/prefs.rs` has agent_chat preference fields with serialization — removing them changes the preferences schema (old saved prefs will have unknown fields, which egui persistence tolerates).
- `solidcraft-cli` loses its `mcp` subcommand; `solidcraft mcp` help text must go.
- `cargo xtask layers` `INTRA_LAYER_ORDER` references `mcp` — remove `("mcp", Class::Layer(5))` and the `&["gpu", "mcp"]` entry.

## Verification

After each phase:
- `cargo build --release -p solidcraft-cli` (after phase 2)
- `cargo build --release -p solidcraft-ui-egui -p solidcraft` (after phases 4–5)
- `cargo xtask ci` on the whole workspace (final)
- `cargo run --release -p solidcraft -- --sample` should launch (Vulkan init) without agent chat.

## Commit strategy

Per AGENTS.md: one topic per PR, reference the issue in the title. This is one topic ("remove AI features"), so one PR from `remove-ai-features` against `origin/main`, opened as a draft against the fork `dinujohnk/3D_model_app`.

## Open question before starting

The MCP server (`solidcraft-mcp`) is also how `solidcraft-cli mcp` serves a headless design session. Removing it removes that headless agent-driven path entirely. Confirm you want the full MCP crate deleted (not just the agent-chat UI) before I begin Phase 2.
