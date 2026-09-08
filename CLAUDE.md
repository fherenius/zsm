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

### Tests

The pure logic lives in a **lib target** (`src/lib.rs`) precisely so it can be tested: the binary links `zellij-tile`, whose host imports are undefined off `wasm32-wasip1`, so a test harness containing the binary will not link. Run the suite against the host:

```bash
cargo test --lib --target aarch64-apple-darwin   # or your host triple
```

Anything Zellij-free belongs in the lib (`naming`, `session_name`, `text`, `list`, `config`) and should come with tests. The binary modules (`state`, `ui`, `session`, `zoxide`, `new_session_info`) are not covered — keep logic out of them where you reasonably can.

Note: nixpkgs' `rustc` ships no `wasm32-wasip1` std, so a plain `nix shell nixpkgs#cargo` can only `cargo check --target <host>`. For a real WASM build without rustup, use docker: `docker run --rm -v "$PWD":/w -w /w rust:1-slim sh -c 'rustup target add wasm32-wasip1 && cargo build --release'`.

## Architecture

### Plugin lifecycle (`src/main.rs`)

`register_plugin!(PluginState)` wires `PluginState` into Zellij via the `ZellijPlugin` trait. The flow is strictly event-driven:

1. **`load()`** — initializes config from the KDL `BTreeMap`, requests permissions (`RunCommands`, `ReadApplicationState`, `ChangeApplicationState`, `MessageAndLaunchOtherPlugins`), and subscribes to events. **It does not fetch zoxide directories yet.**
2. **`update(event)`** — handles `Key`, `SessionUpdate`, `ModeUpdate`, `RunCommandResult`, `PermissionRequestResult`, `Visible`. Returns `bool` = "should re-render". Zoxide is fetched only **after** `PermissionStatus::Granted` arrives — this permission-gated sequencing is load-bearing; fetching earlier silently fails. `Visible(true)` re-queries zoxide and re-pulls the session list, so **naming and the merge re-run every time the plugin is shown** — keep both cheap.
3. **`pipe()`** — receives the filepicker plugin's result (matched by `request_id`).
4. **`render(rows, cols)`** — delegates to `PluginRenderer`.

`update()` also owns zoxide integration: it runs `zoxide query -l -s` (output is `"<score> <path>"` per line) and hands the paths to `zsm::naming::session_names` for the **smart session-naming algorithm**.

### State & screens (`src/state.rs`)

`PluginState` is the single source of truth. It routes keys by `ActiveScreen` (`Main` or `NewSession`), holds the `SessionManager`, `SearchEngine`, `NewSessionInfo`, zoxide directories, config, the cached item list, and pending-deletion / filepicker `request_id` state.

`build_combined_items()` is the core data merge: it matches existing/resurrectable Zellij sessions against zoxide-derived session names (including incremented variants like `project.2`) and produces a `Vec<SessionItem>` of `ExistingSession` / `ResurrectableSession` / `Directory`. The result is **cached** in the `combined_items` field and rebuilt only by `rebuild_combined_items()`, which every `update_*` method calls — it is read several times per render and once per keystroke. `rebuild_combined_items()` also re-runs the active search and clamps `selected_index`, so any new way of changing the data must go through it. When searching, displayed rows come from `SearchEngine` instead.

### Module map

- `session/` — `SessionManager` (switch/kill/delete-dead sessions, `generate_incremented_name`) and `types.rs` (`SessionItem`, `SessionAction`).
- `zoxide/` — `ZoxideDirectory` (path + ranking + generated `session_name`; its `Ord` is what `process_zoxide_output` sorts by) and `SearchEngine` (fuzzy matching via `fuzzy-matcher`/skim; sorts sessions before directories). Search matches `SessionItem::display_text`, which is **also** what the renderer draws — that shared method is what keeps fuzzy match indices pointing at the right characters, so do not format rows anywhere else.
- `new_session_info.rs` — the new-session screen state machine: `EnteringName` → `EnteringLayoutSearch`, with layout fuzzy search and the actual `switch_session_with_layout`/`switch_session_with_cwd` calls.
- `ui/` — `PluginRenderer` (`renderer.rs`) draws both screens via `print_text_with_coordinates` / `print_table_with_coordinates`. `theme.rs` uses **indexed colors (0–3) that map to the user's Zellij theme**; there is deliberately no palette anywhere, so do not hardcode RGB. Add a named role to `Theme` rather than calling `color_range` at the call site.
- **Lib target** (`lib.rs`, all Zellij-free and unit tested):
  - `naming.rs` — the smart session-naming algorithm (`session_names`).
  - `session_name.rs` — the hard limits (`validate`, `validate_against_current`) and the `base`/`base.2`/`base.3` series (`first_free_increment`). Every path that creates a session goes through these.
  - `text.rs` — character-safe shortening for the UI. **Never slice a `str` by byte offset or call `String::truncate` on display text**: a cut inside a codepoint panics, and a panic traps the WASM instance and kills the plugin.
  - `list.rs` — scrolling window and selection movement, shared by the main list and the layout list.
  - `config.rs` — `Config::from_zellij_config` parses the KDL options. `base_paths` is a **pipe-separated** (`|`) list.

### Key constraints & gotchas

- **Session-name length limit.** Zellij session names live in a Unix-domain-socket path capped at 108 bytes, and the socket path isn't knowable from WASM. Generated names aim for `naming::MAX_GENERATED_NAME_LEN` (29 chars); `session_name::MAX_SESSION_NAME_BYTES` (108) is the hard limit actually enforced, along with rejecting any name containing `/`. Preserve these checks when touching naming.
- **Smart naming** resolves basename conflicts by adding the minimal number of leading path segments, adds extra context for directories nested inside other zoxide dirs, and abbreviates/truncates (`abbreviate_segment`) only when over the length cap. `normalize_path` strips the longest matching `base_path` first (exact matches keep the full path). Nesting is decided by looking up each path's ancestors in a set — keep it out of the O(n²) shape, since zoxide databases run to thousands of entries and naming re-runs every time the plugin is shown.
- **Filepicker** is launched as a separate plugin via `pipe_message_to_plugin`; correlation is by a UUID `request_id` tracked in `PluginState.request_ids` and validated in `pipe()`. The returned path is used as-is: **a plugin cannot stat host paths** (it only sees the `/host`, `/data`, `/tmp` preopens), so `Path::exists`/`is_file` always report "missing" here and must not be used to make decisions.

## Config (Zellij KDL layout, not env/files)

All config comes from the plugin block in the user's Zellij layout, read as `BTreeMap<String, String>`: `default_layout`, `session_separator` (default `.`), `show_resurrectable_sessions` (default `false`), `base_paths` (pipe-separated). See `plugin.kdl` for the annotated template.

## Releases & Commits

Releases are automated by **release-please** (`release-type: rust`) on push to `main`. When it cuts a release, CI (`.github/workflows/release-please.yml`) builds the WASM and uploads `zsm.wasm` + `checksums.txt` to the GitHub release. `Cargo.toml` `version` is bumped by the release PR — don't edit it by hand.

Commit messages **must** follow Conventional Commits (they drive the changelog and version bump):

- `fix:` → patch, `feat:` → minor (pre-1.0: `bump-minor-pre-major` is on, so `feat` bumps minor and `fix` does not bump). `!` suffix → major.
- Other types (`docs`, `refactor`, `ci`, `chore`, `perf`, `style`, `build`, `revert`) are categorized in the changelog (`.release-please-config.json`).
- An optional scope describes the affected area, e.g. `feat(config): ...`, `fix(naming): ...`, `fix(ui): ...`. Do not add `Co-Authored-By` lines.

**Workflow:** Always `git commit` after each step of work. **NEVER `git push`** — pushing is done manually by the maintainer.
