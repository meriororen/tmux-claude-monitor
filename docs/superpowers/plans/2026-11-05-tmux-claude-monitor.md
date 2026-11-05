# tmux Claude Monitor Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** An always-on-top macOS Tauri panel that discovers every Claude Code session running in any tmux pane, shows state/title/summary per session, notifies on structured needs-input, and jumps to the pane on click.

**Architecture:** A Rust backend polls `tmux list-panes`/`capture-pane`, `ps`, and Claude Code transcript files every ~2s, builds a `Vec<SessionCard>`, and emits it to a vanilla-TS webview that renders the approved compact mockup. All observation is read-only; the only mutations are `tmux switch-client` + iTerm2 activation on card click. Pure parsing/classification functions are unit-tested against fixtures captured from real panes.

**Tech Stack:** Tauri v2 (Rust stable via rustup), vanilla TypeScript + Vite frontend, vitest for frontend units, `tauri-plugin-notification`, `objc2` for the fullscreen-auxiliary window shim.

**Spec:** `docs/superpowers/specs/2026-11-05-tmux-claude-monitor-design.md` (+ approved mockup `docs/superpowers/specs/mockups/2026-11-05-panel-layout.html`, option A).

## Global Constraints

- macOS only; terminal jump targets iTerm2 only.
- Tauri v2; frontend is vanilla TS (no framework); keep footprint small (anti-goal: heavy footprint).
- Observation strictly read-only: never `tmux send-keys`, never write to `~/.claude/**`.
- Never rely on the login shell `PATH`: a GUI `.app` gets `/usr/bin:/bin:/usr/sbin:/sbin` — every external binary (`tmux`) is resolved to an absolute path; `ps`, `lsof`, `osascript` live in default paths.
- Defaults (all configurable): poll interval **2000ms**, stale threshold **30 min**, notifications **on**.
- Panel: **340px** wide, dark theme; colors/anatomy exactly per mockup option A (`#16181d` panel bg, green `#4ade80` working, amber `#fbbf24` needs-input, gray `#6b7280` stale, amber card `#2a2210`/border `#fbbf24`).
- Notifications: only **structured** needs-input notifies; prose questions go amber silently; one per transition; startup coalesces multiple already-waiting sessions into one notification.
- No LLM calls anywhere.
- Commit messages in Japanese, no attribution/`Co-Authored-By` lines (user preference).
- Facts verified on the target machine (2026-11-05, Claude Code ~2.1.31x, tmux 3.8): a Claude pane's `pane_current_command` is a bare version string (e.g. `2.1.314`); the claude process is a *child* of the pane shell with argv `claude [--resume <uuid>] …` (may also appear as `node …/cli.js`); claude does **not** hold its `.jsonl` open (lsof mapping unusable); the pane renders a session title in the ruler line above the input box (`──── <title> ─`); the idle prompt line starts with `❯`.

## Review Focus

Failure modes the spec implies but didn't enumerate; each line's pinning test is added to the owning task below.

1. **tmux not on the GUI PATH** → panel silently empty on every launch from Finder. Expect: tmux resolved from known Homebrew/system locations. (Task 2, `resolve_tmux_bin` test.)
2. **tmux session names containing the field delimiter** (`|`, spaces) → garbled parsing, wrong jump target. Expect: parsing uses a TAB-separated format and survives `|` in names. (Task 2, `parse_panes_with_pipe_in_name` test.)
3. **Huge transcript files** (long-running sessions reach 100s of MB) → reading whole file every 2s pegs CPU/RAM. Expect: bounded tail read only. (Task 4, `read_tail_lines_bounded` test.)
4. **claude launched via a wrapper** (`node …/cli.js`) or without `--resume` → session invisible or mapped to the wrong transcript. Expect: detection matches both argv shapes; missing resume-id falls back to newest-mtime `.jsonl`. (Task 3, `find_claude_node_wrapper` test; Task 4, `locate_transcript_mtime_fallback` test.)
5. **No tmux server running** (`no server running on …` on stderr, exit 1) → app must show an empty panel, not crash or error-loop. Expect: `TmuxError::NoServer` → zero cards. (Task 2, `list_panes_no_server` test; Task 6 builds zero cards from it.)

---

### Task 1: Toolchain + Tauri scaffold

**Files:**
- Create: Tauri scaffold at repo root (`src-tauri/`, `ui/` as Vite root, `package.json`, `.gitignore`)

**Interfaces:**
- Produces: a building Tauri v2 app; `cargo test` runs in `src-tauri/`; `pnpm vitest run` runs at root. All later Rust modules live in `src-tauri/src/`, registered in `lib.rs`.

- [ ] **Step 1: Install Rust** — `curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y`, then `source "$HOME/.cargo/env"`. Verify: `rustc --version` prints stable ≥1.80.
- [ ] **Step 2: Scaffold** — `pnpm create tauri-app@latest` into the existing repo (app name `tmux-claude-monitor`, identifier `jp.isa.tmux-claude-monitor`, template **vanilla-ts**, manager pnpm). Move/keep frontend under `ui/` with `tauri.conf.json` `frontendDist`/`devUrl` pointing at it (or accept the template's `src/` layout — implementer's call; use the template layout consistently everywhere the plan says `ui/`).
- [ ] **Step 3: Panel window config** — in `tauri.conf.json` set the main window: `width: 340`, `height: 480`, `alwaysOnTop: true`, `decorations: false`, `resizable: false`, `title: "Claude Sessions"`.
- [ ] **Step 4: Add vitest** — `pnpm add -D vitest`; add `"test": "vitest run"` script.
- [ ] **Step 5: Verify** — `pnpm tauri build --debug` succeeds (a `.app` is produced); `cargo test` (in `src-tauri/`) and `pnpm test` both run green (zero tests is fine).
- [ ] **Step 6: Commit** — `git add -A && git commit -m "Tauri v2 スキャフォールド追加（vanilla-ts・パネルウィンドウ設定）"`

### Task 2: tmux module

**Files:**
- Create: `src-tauri/src/tmux.rs` (+ `mod tmux;` in `lib.rs`)

**Interfaces:**
- Produces:
  - `pub struct TmuxPane { pub session: String, pub window_index: u32, pub pane_index: u32, pub pane_id: String, pub pane_pid: i32, pub current_command: String, pub current_path: String }`
  - `pub enum TmuxError { NoServer, NotFound, Io(String) }`
  - `pub fn resolve_tmux_bin() -> Result<PathBuf, TmuxError>` — first existing of `$TCM_TMUX_BIN`, `/opt/homebrew/bin/tmux`, `/usr/local/bin/tmux`, `/usr/bin/tmux`.
  - `pub fn parse_panes(output: &str) -> Vec<TmuxPane>` — pure; TAB-separated.
  - `pub fn list_panes(bin: &Path) -> Result<Vec<TmuxPane>, TmuxError>` — runs `tmux list-panes -a -F '#{session_name}<TAB>#{window_index}<TAB>#{pane_index}<TAB>#{pane_id}<TAB>#{pane_pid}<TAB>#{pane_current_command}<TAB>#{pane_current_path}'` (literal tab via `-F` string).
  - `pub fn capture_pane(bin: &Path, pane_id: &str) -> Result<String, TmuxError>` — `capture-pane -p -t <pane_id>`.
  - `pub fn switch_to(bin: &Path, session: &str, window_index: u32, pane_index: u32) -> Result<(), TmuxError>` — `switch-client -t <session>` then `select-window -t <session>:<window_index>` then `select-pane -t <session>:<window_index>.<pane_index>`.

- [ ] **Step 1: Write failing tests** in `tmux.rs` `#[cfg(test)]`:

```rust
#[test] fn parse_panes_basic() {
    let out = "neovim\t2\t0\t%6\t1993\t2.1.314\t/Users/x/work/rainbow\n";
    let p = &parse_panes(out)[0];
    assert_eq!((p.session.as_str(), p.window_index, p.pane_index, p.pane_id.as_str(), p.pane_pid),
               ("neovim", 2, 0, "%6", 1993));
}
#[test] fn parse_panes_with_pipe_in_name() {
    let out = "my|weird session\t0\t1\t%3\t42\tzsh\t/tmp\n";
    assert_eq!(parse_panes(out)[0].session, "my|weird session");
}
#[test] fn parse_panes_skips_malformed_lines() {
    assert!(parse_panes("garbage-no-tabs\n").is_empty());
}
#[test] fn list_panes_no_server() {
    // run list_panes against a guaranteed-dead socket: `tmux -L tcm-test-nonexistent`
    // implement by allowing extra args in a test-only helper, or classify stderr:
    // stderr containing "no server running" => TmuxError::NoServer
    assert!(matches!(classify_tmux_failure("no server running on /tmp/x"), TmuxError::NoServer));
}
#[test] fn resolve_tmux_env_override() {
    std::env::set_var("TCM_TMUX_BIN", "/usr/bin/true");
    assert_eq!(resolve_tmux_bin().unwrap(), PathBuf::from("/usr/bin/true"));
    std::env::remove_var("TCM_TMUX_BIN");
}
```

(`classify_tmux_failure(stderr: &str) -> TmuxError` is a pub(crate) helper used by `list_panes`.)

- [ ] **Step 2: Run** `cargo test tmux` — expect FAIL (module/functions missing).
- [ ] **Step 3: Implement** the module per the interface block. Spawn `std::process::Command` with absolute bin path; nonzero exit → `classify_tmux_failure(stderr)`.
- [ ] **Step 4: Run** `cargo test tmux` — expect PASS.
- [ ] **Step 5: Commit** — `git commit -m "tmuxモジュール追加: pane列挙・capture・switchラッパー（TAB区切りパース）"`

### Task 3: Claude process discovery (`procs.rs`)

**Files:**
- Create: `src-tauri/src/procs.rs`

**Interfaces:**
- Consumes: `TmuxPane.pane_pid` (Task 2).
- Produces:
  - `pub struct ClaudeProc { pub pid: i32, pub cmdline: String, pub resume_session_id: Option<String> }`
  - `pub fn snapshot_ps() -> String` — runs `/bin/ps -axo pid=,ppid=,command=` **once per poll tick** (caller holds the output).
  - `pub fn find_claude_in_pane(pane_pid: i32, ps_output: &str) -> Option<ClaudeProc>` — pure; walks the descendant tree of `pane_pid` in the ps snapshot; a process matches when its command's argv0 basename is `claude`, OR argv0 is `node`/ends with `/node` and any arg path component contains `claude` and ends with `cli.js`.
  - `pub fn extract_resume_id(cmdline: &str) -> Option<String>` — the UUID after `--resume`.
  - `pub fn cwd_of_pid(pid: i32) -> Option<PathBuf>` — `/usr/sbin/lsof -a -p <pid> -d cwd -Fn`, parse the `n`-prefixed line. Called only when a *new* claude PID appears (cache by PID in the poller).

- [ ] **Step 1: Write failing tests** (pure functions, fixture strings):

```rust
const PS: &str = "\
 1993     1075 -zsh\n\
77840     1993 claude --resume 01234567-89ab-cdef-0123-456789abcdef\n\
77900    77840 git status\n";

#[test] fn find_claude_direct() {
    let c = find_claude_in_pane(1993, PS).unwrap();
    assert_eq!(c.pid, 77840);
    assert_eq!(c.resume_session_id.as_deref(), Some("01234567-89ab-cdef-0123-456789abcdef"));
}
#[test] fn find_claude_node_wrapper() {
    let ps = " 10 1 -zsh\n 20 10 node /Users/x/.nvm/versions/node/v24/lib/node_modules/@anthropic-ai/claude-code/cli.js\n";
    assert_eq!(find_claude_in_pane(10, ps).unwrap().pid, 20);
}
#[test] fn find_claude_grandchild() {
    let ps = " 10 1 -zsh\n 15 10 -zsh\n 20 15 claude\n";
    assert_eq!(find_claude_in_pane(10, ps).unwrap().pid, 20);
}
#[test] fn no_claude_in_plain_shell_pane() {
    let ps = " 30 1 -zsh\n 31 30 nvim .\n";
    assert!(find_claude_in_pane(30, ps).is_none());
}
#[test] fn resume_id_absent() {
    assert!(extract_resume_id("claude --dangerously-skip-permissions").is_none());
}
```

- [ ] **Step 2: Run** `cargo test procs` — expect FAIL.
- [ ] **Step 3: Implement**; build a `ppid → children` map, BFS from `pane_pid`.
- [ ] **Step 4: Run** `cargo test procs` — expect PASS.
- [ ] **Step 5: Commit** — `git commit -m "procsモジュール追加: paneプロセスツリーからclaudeプロセス検出（--resume/ラッパー対応）"`

### Task 4: Transcript reader (`transcript.rs`)

**Files:**
- Create: `src-tauri/src/transcript.rs`
- Test fixtures: `src-tauri/tests/fixtures/transcript_sample.jsonl` (built in Step 1 from a real session file: copy ~40 lines spanning a `summary` record, a user message, an assistant message, a tool_use — sanitize any content you wouldn't commit)

**Interfaces:**
- Consumes: `ClaudeProc` (Task 3), pane `current_path` (Task 2).
- Produces:
  - `pub fn escape_project_path(cwd: &Path) -> String` — every char outside `[A-Za-z0-9-]` becomes `-` (verify against real dirs in `~/.claude/projects/` before finalizing; e.g. `/Users/x/work/rainbow` → `-Users-x-work-rainbow`).
  - `pub fn locate_transcript(projects_root: &Path, cwd: &Path, resume_id: Option<&str>) -> Option<PathBuf>` — `<root>/<escaped>/<resume_id>.jsonl` if it exists, else the newest-mtime `*.jsonl` in that dir.
  - `pub fn read_tail_lines(path: &Path, max_bytes: u64) -> std::io::Result<Vec<String>>` — seek to `len - max_bytes` (clamp 0), read, drop the first (possibly partial) line when seek > 0. Poller uses `max_bytes = 262_144`.
  - `pub struct TranscriptInfo { pub summary_title: Option<String>, pub first_user_text: Option<String>, pub last_assistant_text: Option<String>, pub current_tool: Option<String>, pub last_event_at: Option<std::time::SystemTime> }`
  - `pub fn parse_transcript_tail(lines: &[String]) -> TranscriptInfo` — serde_json per line, ignore unparseable lines; `summary_title` from the latest `{"type":"summary","summary":…}`; `first_user_text`/`last_assistant_text` from `message.role` + content text blocks (truncate at 120 chars); `current_tool` = name/target of the latest `tool_use` after the latest assistant text; `last_event_at` = file mtime (passed in or read by caller — implementer picks one, document it in the signature actually chosen).

- [ ] **Step 1: Build the fixture** from a real file, e.g. `~/.claude/projects/-Users-x-work-rainbow/<session-id>.jsonl`. Then write failing tests:

```rust
#[test] fn escape_matches_real_layout() {
    assert_eq!(escape_project_path(Path::new("/Users/x/work/rainbow")),
               "-Users-x-work-rainbow");
}
#[test] fn locate_prefers_resume_id() { /* tempdir with two jsonl files; resume id wins over newer mtime */ }
#[test] fn locate_transcript_mtime_fallback() { /* no resume id → newest mtime wins */ }
#[test] fn read_tail_lines_bounded() {
    // 10MB temp file of numbered lines; max_bytes 4096 → only trailing lines returned,
    // first returned line is complete (equals a known full line)
}
#[test] fn parse_tail_extracts_fields() {
    // fixture: summary_title Some, last_assistant_text Some and ≤120 chars
}
```

- [ ] **Step 2: Run** `cargo test transcript` — expect FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** `cargo test transcript` — expect PASS.
- [ ] **Step 5: Commit** — `git commit -m "transcriptモジュール追加: プロジェクトパスエスケープ・jsonl選択・有界tail読み・要約抽出"`

### Task 5: Pane state heuristics (`state_detect.rs`)

**Files:**
- Create: `src-tauri/src/state_detect.rs`
- Test fixtures: `src-tauri/tests/fixtures/pane_{working,permission,question_box,idle,idle_title}.txt` — captured from **live panes** with `tmux capture-pane -p -t <id>` (this machine has panes in most states daily; for `permission`, trigger any permission prompt in a scratch claude session). Keep fixtures verbatim.

**Interfaces:**
- Consumes: pane text from `capture_pane` (Task 2).
- Produces:
  - `pub enum PaneState { Working, WaitingStructured, Idle }`
  - `pub fn classify_pane(text: &str) -> PaneState` — ordered rules, all in this one module with a comment block naming the Claude Code version they were captured against:
    - `Working`: contains `esc to interrupt`.
    - `WaitingStructured`: contains a numbered-option cursor (`❯ 1.`) or any of the permission/question markers found in the fixtures (e.g. `Do you want`, `Esc to cancel` — finalize the list from the actual fixture text, not from this plan).
    - else `Idle`.
  - `pub fn extract_pane_title(text: &str) -> Option<String>` — the ruler line above the input box: a line of `─` runs ending `─ <title> ─`; return the title text.
  - `pub fn extract_pending_question(text: &str) -> Option<String>` — for `WaitingStructured`, the nearest non-empty line above the option list / inside the dialog; used as the card's `⚠ Waiting:` summary.

- [ ] **Step 1: Capture the five fixtures** (one per state; `idle_title` = idle pane whose ruler shows a session title, like the live `slide-5-logtonewrelic-link` pane). Write failing tests: one `classify_pane` assertion per fixture, `extract_pane_title` returns the known title for `idle_title` and `None` for a titleless fixture, `extract_pending_question` returns the question line for `permission`.
- [ ] **Step 2: Run** `cargo test state_detect` — expect FAIL.
- [ ] **Step 3: Implement** against the fixtures.
- [ ] **Step 4: Run** `cargo test state_detect` — expect PASS.
- [ ] **Step 5: Commit** — `git commit -m "state_detectモジュール追加: capture-paneヒューリスティクス（実画面フィクスチャ付き）"`

### Task 6: Poller + card model + notification policy (`poller.rs`)

**Files:**
- Create: `src-tauri/src/poller.rs`, `src-tauri/src/settings.rs`
- Modify: `src-tauri/src/lib.rs` (spawn poller on setup, register notification plugin)

**Interfaces:**
- Consumes: Tasks 2–5 interfaces.
- Produces:
  - `pub struct Settings { pub poll_interval_ms: u64, pub stale_after_min: u64, pub notifications: bool, pub window_pos: Option<(i32,i32)> }` with `Default` (2000 / 30 / true / None); `load()`/`save()` as JSON at `app_config_dir()/settings.json`.
  - `#[derive(Serialize, Clone, PartialEq)] pub struct SessionCard { pub pane_id: String, pub tmux_session: String, pub window_index: u32, pub pane_index: u32, pub title: String, pub summary: String, pub state: CardState, pub idle_secs: u64 }`
  - `#[derive(Serialize, Clone, Copy, PartialEq)] pub enum CardState { Working, NeedsInputStructured, NeedsInputProse, Idle, Stale }`
  - `pub fn derive_card_state(pane: PaneState, info: &TranscriptInfo, idle_secs: u64, stale_after_min: u64) -> CardState` — `Working`→Working; `WaitingStructured`→NeedsInputStructured; `Idle` + `last_assistant_text` ending in `?`/`？` → NeedsInputProse; `Idle` + idle ≥ threshold → Stale; else Idle.
  - `pub fn derive_title(cwd: &Path, pane_title: Option<&str>, info: &TranscriptInfo) -> String` — `<cwd basename>: <X>` where X = pane ruler title, else `summary_title`, else truncated `first_user_text`, else cwd basename alone.
  - `pub fn derive_summary(state: CardState, pending_question: Option<&str>, info: &TranscriptInfo) -> String` — structured: `⚠ Waiting: <question>`; working: `current_tool` else `last_assistant_text`; others: `last_assistant_text` else empty.
  - `pub fn notifications_due(prev: Option<&HashMap<String, CardState>>, curr: &[SessionCard]) -> Vec<String>` — `prev=None` (first tick): all `NeedsInputStructured` coalesced into **one** message (`"2 sessions waiting for input"`) when >1; afterwards: one message per card whose state became `NeedsInputStructured` from something else. Prose never notifies.
  - Poller loop (`tauri::async_runtime::spawn`): every tick — `snapshot_ps()` once; for each pane with a claude descendant build a `SessionCard`; emit `app.emit("sessions-updated", &cards)`; send due notifications via `tauri_plugin_notification`. Cache `pane_id → (claude_pid, cwd, transcript_path)` and re-resolve only when the claude PID changes.

- [ ] **Step 1: Write failing tests** for the four pure functions: state derivation (one assertion per `CardState`, including prose `？`), title precedence (pane title > summary > first message > basename), structured summary formatting, and notification policy:

```rust
#[test] fn startup_coalesces_waiting() {
    let cards = vec![card("%1", CardState::NeedsInputStructured), card("%2", CardState::NeedsInputStructured)];
    let msgs = notifications_due(None, &cards);
    assert_eq!(msgs.len(), 1);
}
#[test] fn transition_notifies_once() { /* Working→NeedsInputStructured notifies; unchanged does not */ }
#[test] fn prose_never_notifies() { /* Idle→NeedsInputProse → empty */ }
```

- [ ] **Step 2: Run** `cargo test poller settings` — expect FAIL.
- [ ] **Step 3: Implement** pure functions + settings + the loop; wire `tauri-plugin-notification` (`pnpm add @tauri-apps/plugin-notification`, cargo add, permission in capabilities).
- [ ] **Step 4: Run** `cargo test` — all green. Then `pnpm tauri dev` with tmux running: log line per tick shows the expected number of cards (manual check: count matches the real Claude panes).
- [ ] **Step 5: Commit** — `git commit -m "pollerと通知ポリシー追加: カード生成・状態遷移通知（起動時はまとめて1通）・設定ファイル"`

### Task 7: Jump-to-pane (`jump.rs`)

**Files:**
- Create: `src-tauri/src/jump.rs`
- Modify: `lib.rs` (register `#[tauri::command] jump_to_session`)

**Interfaces:**
- Consumes: `switch_to` (Task 2); `SessionCard` fields (Task 6).
- Produces:
  - `pub fn activate_iterm() -> Result<(), String>` — `/usr/bin/osascript -e 'tell application "iTerm2" to activate'`.
  - `#[tauri::command] pub fn jump_to_session(session: String, window_index: u32, pane_index: u32) -> Result<(), String>` — activate iTerm2, then `switch_to`. If `switch_to` fails with `NoServer`/no attached client, return `Err` with a short message (frontend shows it as a transient toast line in the header) — graceful, no crash. (Opening a new iTerm2 window that attaches is the spec's nice-to-have; skip in v1.)

- [ ] **Step 1: Write failing test** — unit test only the argument formatting: `switch_target(("neovim", 2, 0)) == ("neovim", "neovim:2", "neovim:2.0")` (pure helper in `tmux.rs` or `jump.rs`).
- [ ] **Step 2: Run** — expect FAIL. **Step 3: Implement.** **Step 4: Run** — PASS; then manual: `pnpm tauri dev`, invoke the command from the devtools console, confirm iTerm2 fronts and tmux lands on the right pane.
- [ ] **Step 5: Commit** — `git commit -m "jumpコマンド追加: iTerm2アクティベート+tmux switch-client/select-pane"`

### Task 8: Frontend panel UI

**Files:**
- Create/Modify: `ui/index.html`, `ui/src/main.ts`, `ui/src/render.ts`, `ui/src/style.css`
- Test: `ui/src/render.test.ts` (vitest)

**Interfaces:**
- Consumes: `sessions-updated` event payload = `SessionCard[]` (Task 6 serde names, camelCased via `#[serde(rename_all = "camelCase")]` — add that attribute to `SessionCard`/`CardState` in Task 6's structs when wiring); `jump_to_session` command (Task 7).
- Produces: `export function renderCards(cards: SessionCard[]): string` — pure HTML-string renderer (testable without DOM); `main.ts` wires `listen()`, innerHTML swap, click delegation → `invoke('jump_to_session', …)`, drag region (`data-tauri-drag-region` on the header), and saves window position on move (debounced `invoke('save_window_pos')` — add that tiny command to `settings.rs`). The header's ⚙ affordance calls `invoke('open_settings')` (also added to `settings.rs`: runs `/usr/bin/open` on `settings.json`); the poller re-reads settings each tick so edits apply without restart — that is the whole settings UI for v1.

- [ ] **Step 1: Write failing vitest** for `renderCards`: renders one `.sess` per card; `NeedsInputStructured`/`NeedsInputProse` cards carry class `attn` and dot class `wait`; `Working`→`work` dot; `Stale`→`stale` dot + idle time text (`3h`); header shows `CLAUDE SESSIONS — <n>`; card element carries `data-session`/`data-window`/`data-pane` attributes; HTML-escapes titles (test a title containing `<b>`).
- [ ] **Step 2: Run** `pnpm test` — expect FAIL.
- [ ] **Step 3: Implement** `render.ts` + `style.css` copying the approved mockup's CSS (panel/sess/dot/stitle/ssummary classes verbatim from `docs/superpowers/specs/mockups/2026-11-05-panel-layout.html` option A), `main.ts` wiring, idle formatting (`now` <60s, `Nm`, `Nh`).
- [ ] **Step 4: Run** `pnpm test` — PASS. Then `pnpm tauri dev`: panel matches mockup A against the live sessions; clicking a card jumps.
- [ ] **Step 5: Commit** — `git commit -m "パネルUI実装: 承認済みモックアップA準拠のコンパクトカード・クリックでジャンプ"`

### Task 9: Always-everywhere window behavior (`window.rs`)

**Files:**
- Create: `src-tauri/src/window.rs`
- Modify: `lib.rs` setup hook

**Interfaces:**
- Consumes: main `WebviewWindow`; `Settings.window_pos` (Task 6).
- Produces: `pub fn configure_panel(win: &tauri::WebviewWindow)` — `set_always_on_top(true)`, `set_visible_on_all_workspaces(true)`, restore saved position; then raise above fullscreen apps: via `objc2`/`objc2-app-kit`, set the NSWindow `collectionBehavior |= CanJoinAllSpaces | FullScreenAuxiliary` and `level = NSStatusWindowLevel`. (Spec: native shim is in scope if Tauri flags fall short — they fall short for fullscreen-auxiliary.)

- [ ] **Step 1: Implement** (no unit test — windowing is manual-verification territory).
- [ ] **Step 2: Verify manually, all three:** (a) switch Spaces — panel visible on each; (b) iTerm2 fullscreen — panel floats over it; (c) quit + relaunch — panel reappears at the dragged position.
- [ ] **Step 3: Commit** — `git commit -m "ウィンドウ挙動: 全Space表示+フルスクリーン上フロート（NSWindow collectionBehavior shim）・位置永続化"`

### Task 10: Package + acceptance

**Files:**
- Modify: `README.md` (install/run/permissions notes)

- [ ] **Step 1: Build** — `pnpm tauri build`; ad-hoc signing is fine for personal use. Install the produced `.app` to `/Applications` (notifications require running as a bundled app, not `tauri dev`).
- [ ] **Step 2: First-run permissions** — trigger a jump (grant Automation→iTerm2) and a notification (grant Notifications). Note both in README.
- [ ] **Step 3: Acceptance sketch from the spec**, with ≥3 real sessions (one working, one on a permission prompt, one idle hours) plus nvim/shell panes:
  - panel lists exactly the Claude panes with sensible titles/summaries, correct colors, idle times;
  - the permission-prompt session is amber and produced exactly one notification at transition;
  - clicking any card lands on that pane in iTerm2 within ~1s;
  - `top`/Activity Monitor: app idles at low single-digit CPU% and modest memory over 10 minutes.
  Record results (pass/fail per line) in the README or commit message.
- [ ] **Step 4: Commit** — `git commit -m "リリースビルドと受け入れ確認: 権限手順をREADMEに記載"`
