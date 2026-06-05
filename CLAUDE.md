# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`zsm` (Zoxide Session Manager) is a **Zellij plugin written in Rust and compiled to WebAssembly** (`wasm32-wasip1`). It runs inside Zellij's WASM runtime, lists your zoxide directories ranked by frequency, and lets you create/switch Zellij sessions from them with smart auto-generated names.

> Note: the global `~/.claude/CLAUDE.md` documents an unrelated Go project (gondexalizer). Its Go build/test commands do **not** apply here — use the Rust/WASM commands below.

## Build & Dev Commands

`.cargo/config.toml` sets the default target to `wasm32-wasip1`, so plain `cargo` commands build for WASM (no native host build happens).

```bash
# One-time: add the WASM target
rustup target add wasm32-wasip1

# Release build → target/wasm32-wasip1/release/zsm.wasm
cargo build --release

# Debug build → target/wasm32-wasip1/debug/zsm.wasm
cargo build

# Lint / format
cargo clippy
cargo fmt
```

### Iterating on the plugin

You can't run a Zellij plugin standalone — it must be loaded by Zellij. Two dev loops:

```bash
# Option 1: dev layout with hot reload (Alt+r reloads via develop-rust-plugin)
zellij -l zellij.kdl
# re-open after exiting:
zellij action launch-or-focus-plugin file:target/wasm32-wasip1/debug/zsm.wasm

# Option 2: rebuild + reload on save
watchexec --exts rs -- 'cargo build; zellij action start-or-reload-plugin file:target/wasm32-wasip1/debug/zsm.wasm'
```

Plugin logs (including `eprintln!`) go to Zellij's log file, e.g. `tail -f $(find /private/var/folders -name zellij.log)` on macOS (see `zellij.kdl` for the dev layout's log pane).

There is currently **no test suite**. If adding tests, note the default target is WASM — run them against the host with `cargo test --target <host-triple>`.

## Architecture

### Plugin lifecycle (`src/main.rs`)

`register_plugin!(PluginState)` wires `PluginState` into Zellij via the `ZellijPlugin` trait. The flow is strictly event-driven:

1. **`load()`** — initializes config from the KDL `BTreeMap`, requests permissions (`RunCommands`, `ReadApplicationState`, `ChangeApplicationState`, `MessageAndLaunchOtherPlugins`), and subscribes to events. **It does not fetch zoxide directories yet.**
2. **`update(event)`** — handles `Key`, `SessionUpdate`, `ModeUpdate`, `RunCommandResult`, `PermissionRequestResult`. Returns `bool` = "should re-render". Zoxide is fetched only **after** `PermissionStatus::Granted` arrives — this permission-gated sequencing is load-bearing; fetching earlier silently fails.
3. **`pipe()`** — receives the filepicker plugin's result (matched by `request_id`).
4. **`render(rows, cols)`** — delegates to `PluginRenderer`.

`update()` also owns zoxide integration: it runs `zoxide query -l -s` (output is `"<score> <path>"` per line) and the **smart session-naming algorithm** (`generate_smart_session_names` and helpers).

### State & screens (`src/state.rs`)

`PluginState` is the single source of truth. It routes keys by `ActiveScreen` (`Main` or `NewSession`), holds the `SessionManager`, `SearchEngine`, `NewSessionInfo`, zoxide directories, config, colors, and pending-deletion / filepicker `request_id` state.

`combined_items()` is the core data merge: it matches existing/resurrectable Zellij sessions against zoxide-derived session names (including incremented variants like `project.2`) and produces a `Vec<SessionItem>` of `ExistingSession` / `ResurrectableSession` / `Directory`. When searching, items come from `SearchEngine` instead; otherwise from this list.

### Module map

- `session/` — `SessionManager` (switch/kill/delete-dead sessions, `generate_incremented_name`) and `types.rs` (`SessionItem`, `SessionAction`).
- `zoxide/` — `ZoxideDirectory` (path + ranking + generated `session_name`) and `SearchEngine` (fuzzy matching via `fuzzy-matcher`/skim; sorts sessions before directories; matches against the *rendered* display text).
- `new_session_info.rs` — the new-session screen state machine: `EnteringName` → `EnteringLayoutSearch`, with layout fuzzy search and the actual `switch_session_with_layout`/`switch_session_with_cwd` calls.
- `ui/` — `PluginRenderer` (`renderer.rs`) draws both screens via `print_text_with_coordinates` / `print_table_with_coordinates`. `theme.rs` uses **indexed colors (0–3) that map to the user's Zellij theme** — `Theme::new` deliberately ignores the palette; do not hardcode RGB.
- `config.rs` — `Config::from_zellij_config` parses the KDL options. `base_paths` is a **pipe-separated** (`|`) list.

### Key constraints & gotchas

- **Session-name length limit (~29 chars).** Zellij session names live in a Unix-domain-socket path capped at 108 bytes, and the socket path isn't knowable from WASM. `generate_context_aware_name` / `apply_smart_truncation` target ~29 chars; key handlers hard-reject names ≥ 108 bytes and any name containing `/`. Preserve these checks when touching naming.
- **Smart naming** resolves basename conflicts by adding the minimal number of leading path segments, adds extra context for directories nested inside other zoxide dirs, and abbreviates/truncates (`abbreviate_segment`) only when over the length cap. `normalize_path` strips the longest matching `base_path` first (exact matches keep the full path).
- **Filepicker** is launched as a separate plugin via `pipe_message_to_plugin`; correlation is by a UUID `request_id` tracked in `PluginState.request_ids` and validated in `pipe()`.

## Config (Zellij KDL layout, not env/files)

All config comes from the plugin block in the user's Zellij layout, read as `BTreeMap<String, String>`: `default_layout`, `session_separator` (default `.`), `show_resurrectable_sessions` (default `false`), `base_paths` (pipe-separated). See `plugin.kdl` for the annotated template.

## Releases & Commits

Releases are automated by **release-please** (`release-type: rust`) on push to `main`. When it cuts a release, CI (`.github/workflows/release-please.yml`) builds the WASM and uploads `zsm.wasm` + `checksums.txt` to the GitHub release. `Cargo.toml` `version` is bumped by the release PR — don't edit it by hand.

Commit messages **must** follow Conventional Commits (they drive the changelog and version bump):

- `fix:` → patch, `feat:` → minor (pre-1.0: `bump-minor-pre-major` is on, so `feat` bumps minor and `fix` does not bump). `!` suffix → major.
- Other types (`docs`, `refactor`, `ci`, `chore`, `perf`, `style`, `build`, `revert`) are categorized in the changelog (`.release-please-config.json`).
- An optional scope describes the affected area, e.g. `feat(config): ...`, `fix(naming): ...`, `fix(ui): ...`. Do not add `Co-Authored-By` lines.

**Workflow:** Always `git commit` after each step of work. **NEVER `git push`** — pushing is done manually by the maintainer.
