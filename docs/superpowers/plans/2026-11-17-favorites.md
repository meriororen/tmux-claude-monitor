# Favorites (star & resume) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Star sessions, browse them in a Favorites tab that survives pane death, and resume dead ones via `claude --resume` in a new tmux window at the saved cwd.

**Architecture:** A `favorites.json` store (new `favorites.rs`) shared between the poll thread and Tauri commands via `Mutex` app-state. The tick payload becomes `{cards, favorites}`: `favorites` = live starred cards + synthesized dead cards (`pane_id: "dead:<uuid>"`, read from the transcript jsonl). Frontend adds a tab (`all`/`favorites`) to the existing pure key/render modules; resume is a new sanctioned tmux mutation (`new-window -P` + existing switch-to).

**Tech Stack:** Rust (Tauri v2), vanilla TS + Vite, cargo test + vitest.

**Spec:** docs/superpowers/specs/2026-11-17-favorites-design.md

## Global Constraints

- All tmux spawns go through `tmux::run_tmux` (absolute bin, `LC_ALL=en_US.UTF-8`).
- `cwd` is stored verbatim at star time (escaped project dir names are lossy).
- Transcript reads are bounded tails (`read_tail_lines`, 262_144 bytes), never whole files.
- Favorites I/O never blocks the poll tick on failure: load errors → empty store, save errors → toast via command result only.
- No new dependencies (Rust or JS). Frontend stays vanilla TS; `render.ts`/`keys.ts`/`fuzzy.ts` stay pure (DOM-free).
- Session uuids are validated `[0-9a-f-]` before being interpolated into a tmux command string.
- Test fixtures stay synthetic (public mirror) — no real paths/project names beyond the existing `rainbow`-style placeholders.
- Commit messages in Japanese. NO Co-Authored-By / attribution lines (user's global CLAUDE.md overrides the harness default).

## Review Focus

Failure modes no task's tests fully exercise, most likely first — each has a pinned test added to its owning task:

1. favorites.json corrupted/hand-edited mid-write → app must start with an empty store, never crash or rewrite the file silently (Task 2, `load_malformed_returns_empty`).
2. A favorite whose cwd contains spaces/quotes → resume command string must not split it (Task 5: cwd travels as a separate `-c` argv element, `new_window_args_quote_safe`).
3. Dead favorite whose jsonl mtime is in the future (clock skew) → idle must clamp to 0, not underflow (Task 4, `dead_card_future_mtime_clamps`).
4. Two favorites, one transcript-missing → sort must put missing last without panicking on u64::MAX arithmetic (Task 4, `missing_transcript_sorts_last`).
5. `f` pressed while the selected card has `sessionUuid: null` → toast, no favorites.json write (Task 6 reducer test + Task 8 wiring).

---

### Task 1: Verify the `--resume` same-uuid assumption

No files; a ledger finding. The spec builds on: `claude --resume <uuid>` appends to the SAME jsonl (no fork).

- [ ] **Step 1: Create a throwaway session and capture its uuid**

Run (in a scratch dir, e.g. `$(mktemp -d)`):
```bash
cd "$(mktemp -d)" && claude -p "reply with exactly: ok" >/dev/null 2>&1; ls ~/.claude/projects/$(pwd | sed 's/[^A-Za-z0-9-]/-/g')/
```
Expected: exactly one `<uuid>.jsonl`.

- [ ] **Step 2: Resume it and compare**

Run (same dir): `claude -p --resume <uuid> "reply with exactly: ok again"`, then list the project dir again and check the original file's size/mtime.
Expected: the SAME `<uuid>.jsonl` grew; no second jsonl appeared. If a second file appeared instead, STOP — the spec's identity assumption is broken; surface to the user before building anything.

- [ ] **Step 3: Ledger the finding** (`Task 1: complete — resume appends to same jsonl: <observed>`), delete the scratch project dir under `~/.claude/projects/`.

### Task 2: Favorites store (`favorites.rs`)

**Files:**
- Create: `src-tauri/src/favorites.rs`
- Modify: `src-tauri/src/lib.rs` (add `pub mod favorites;`)
- Test: inline `#[cfg(test)]` (existing convention), uses `tempfile`

**Interfaces:**
- Produces:
  - `pub struct Favorite { pub session_uuid: String, pub cwd: String, pub title: String, pub starred_at: String }` (serde, camelCase like SessionCard is NOT needed here — file format uses snake_case per spec JSON)
  - `pub struct Favorites { pub favorites: Vec<Favorite> }`
  - `Favorites::file_in(dir: &Path) -> PathBuf` → `dir.join("favorites.json")`
  - `Favorites::load(dir: &Path) -> Favorites` (missing/malformed → empty)
  - `Favorites::save(&self, dir: &Path) -> std::io::Result<()>` (pretty JSON, create_dir_all)
  - `Favorites::toggle(&mut self, uuid: &str, cwd: &str, title: &str, now_iso: &str) -> bool` (returns new starred state; add if absent, remove if present)
  - `Favorites::contains(&self, uuid: &str) -> bool`

- [ ] **Step 1: Write the failing tests** — `toggle_adds_then_removes` (toggle twice: contains → true then false; added entry carries cwd/title/starred_at verbatim), `roundtrip_save_load` (save→load equality in a tempdir), `load_malformed_returns_empty` (write `{broken`, load → empty vec; file left untouched — mirror `settings.rs` test style).
- [ ] **Step 2: Run to verify FAIL** — `cd src-tauri && cargo test favorites` → compile error: module not found.
- [ ] **Step 3: Implement** `favorites.rs` per the Produces signatures; model load/save on `Settings::load/save`.
- [ ] **Step 4: Run to verify PASS** — `cargo test favorites`.
- [ ] **Step 5: Commit** — `feat: お気に入りストア(favorites.json)を追加`

### Task 3: Card identity — uuid/cwd/starred on SessionCard

**Files:**
- Modify: `src-tauri/src/poller.rs` (SessionCard, CardState, build_cards)
- Test: inline in poller.rs

**Interfaces:**
- Consumes: `Favorites::contains` (Task 2).
- Produces: `SessionCard` gains `pub session_uuid: Option<String>`, `pub cwd: String`, `pub starred: bool`, `pub transcript_missing: bool` (serde camelCase already set on the struct → `sessionUuid`, `cwd`, `starred`, `transcriptMissing`); `CardState` gains variant `Dead`. `build_cards` takes a new param `favs: &Favorites`.

- [ ] **Step 1: Write the failing test** — `live_card_carries_uuid_cwd_starred`: construct the uuid extraction helper directly: `pub fn uuid_of_transcript(path: &Path) -> Option<String>` returns the file stem (`/x/projects/-u-w-rainbow/0123...def.jsonl` → `Some("0123...def")`). Plus: update the existing `card()` test helper with the new fields (uuid `None`, cwd `"/tmp/t"`, false/false) so the suite compiles.
- [ ] **Step 2: Run to verify FAIL** — `cargo test uuid_of_transcript` → not defined.
- [ ] **Step 3: Implement**: `uuid_of_transcript`; in `build_cards`, set `session_uuid: entry.transcript.as_deref().and_then(uuid_of_transcript)`, `cwd: entry.cwd.to_string_lossy().into_owned()`, `starred: uuid matches favs.contains`, `transcript_missing: false`; add `CardState::Dead`; thread `&Favorites` into `build_cards`/`build_cards_for_example` (examples/scan.rs: pass `&Favorites::default()` — derive Default).
- [ ] **Step 4: Run to verify PASS** — `cargo test` (whole suite — the struct change touches many tests).
- [ ] **Step 5: Commit** — `feat: SessionCardにsessionUuid/cwd/starredを追加`

### Task 4: Dead favorite cards + merged favorites list

**Files:**
- Modify: `src-tauri/src/favorites.rs` (card synthesis lives with the store)
- Test: inline

**Interfaces:**
- Consumes: `SessionCard`/`CardState::Dead` (Task 3), `transcript::{escape_project_path, read_tail_lines, parse_transcript_tail}`, `poller::derive_title` semantics (dead title precedence: transcript title record → first user text → saved `fav.title`).
- Produces:
  - `pub fn dead_card(fav: &Favorite, projects_root: &Path, now: SystemTime) -> SessionCard` — `pane_id: format!("dead:{}", fav.session_uuid)`, `tmux_session: ""`, indices 0, `state: CardState::Dead`, `starred: true`; transcript at `projects_root/escape_project_path(cwd)/<uuid>.jsonl`: exists → summary/title from tail, `idle_secs = now − mtime` clamped ≥ 0, `transcript_missing: false`; missing → `transcript_missing: true`, `idle_secs: u64::MAX`, title = `fav.title`, summary = `""`.
  - `pub fn favorites_view(live: &[SessionCard], favs: &Favorites, projects_root: &Path, now: SystemTime) -> Vec<SessionCard>` — live starred cards (cloned) + dead cards for favorites with no live uuid match; sorted by `idle_secs` ascending (missing therefore last).

- [ ] **Step 1: Write the failing tests** — `dead_card_reads_transcript` (tempdir projects root, write a 2-line synthetic jsonl, assert pane_id prefix, state Dead, summary from the jsonl, idle > 0), `dead_card_future_mtime_clamps` (set mtime now+1h via `filetime` is NOT available — instead pass `now = UNIX_EPOCH` older than the file's mtime; assert `idle_secs == 0`), `missing_transcript_sorts_last` + `live_favorite_not_duplicated` (favorites_view with one live starred card whose uuid also sits in favs, one dead, one missing → len 3? no: live(1)+dead(1)+missing(1)=3, order live-or-dead by idle then missing last, and the live uuid appears exactly once).
- [ ] **Step 2: Run to verify FAIL** — `cargo test favorites_view dead_card` → not defined.
- [ ] **Step 3: Implement** per Produces.
- [ ] **Step 4: Run to verify PASS** — `cargo test`.
- [ ] **Step 5: Commit** — `feat: 死んだお気に入りをtranscriptから復元して一覧化`

### Task 5: Backend commands — toggle, resume, new payload

**Files:**
- Modify: `src-tauri/src/tmux.rs` (new_window), `src-tauri/src/favorites.rs` (commands), `src-tauri/src/settings.rs` (claude_bin), `src-tauri/src/poller.rs` (shared state + payload), `src-tauri/src/jump.rs` (extract reusable switch), `src-tauri/src/lib.rs` (manage state, register handlers)
- Test: inline per module

**Interfaces:**
- Consumes: Tasks 2–4.
- Produces:
  - `Settings` gains `pub claude_bin: Option<String>` (default None; update roundtrip test).
  - `pub fn resolve_claude_bin(setting: Option<&str>) -> String` in favorites.rs: `TCM_CLAUDE_BIN` env → setting → `"claude"`.
  - tmux.rs: `pub fn new_window(bin: &Path, cwd: &str, shell_cmd: &str) -> Result<(String, u32, u32), TmuxError>` — runs `["new-window", "-P", "-F", "#{session_name}\t#{window_index}\t#{pane_index}", "-c", cwd, shell_cmd]` via `run_tmux`; parse via new pure `pub fn parse_new_window_output(out: &str) -> Option<(String, u32, u32)>`.
  - jump.rs: extract `pub fn switch_and_show(app, session: &str, w: u32, p: u32) -> Result<(), String>` = activate_iterm + switch_to + hide_panel (jump_to_session becomes a thin wrapper).
  - App-managed state in lib.rs: `pub struct FavState(pub std::sync::Mutex<crate::favorites::Favorites>)`, `pub struct LiveIndex(pub std::sync::Mutex<std::collections::HashMap<String, (String, u32, u32)>>)` (uuid → tmux address, rebuilt each tick).
  - Commands in favorites.rs: `#[tauri::command] toggle_favorite(app, session_uuid: Option<String>, cwd: String, title: String) -> Result<bool, String>` (None → Err("no transcript yet for this session"); uuid chars must be `[0-9a-f-]` → else Err; toggle + save, write-through); `#[tauri::command] resume_session(app, session_uuid: String) -> Result<(), String>` — live in LiveIndex → `switch_and_show`; else find favorite (unknown uuid → Err), transcript missing → Err("transcript missing"), else `new_window` with `format!("{} --resume {}", resolve_claude_bin(...), uuid)` then `switch_and_show` on the parsed address.
  - poller tick: payload becomes `#[derive(Serialize)] struct TickPayload { cards: Vec<SessionCard>, favorites: Vec<SessionCard> }` emitted on the SAME `"sessions-updated"` event, where `favorites = favorites_view(&cards, &favs, &projects_root(), SystemTime::now())`; tick reads `FavState` (lock, clone, drop), rebuilds `LiveIndex` from cards with a `session_uuid`.

- [ ] **Step 1: Write the failing tests** — tmux.rs `parse_new_window_output_roundtrip` (`"work\t3\t0\n"` → `("work",3,0)`; garbage → None), favorites.rs `resolve_claude_bin_precedence` (env unset: setting Some("/x/claude") → it; None → "claude" — don't test the env branch, env is process-global), `toggle_rejects_bad_uuid` (pure helper `pub fn valid_uuid(s: &str) -> bool` — `[0-9a-f-]`, non-empty: `"0123…def"` true, `"x; rm -rf"` false), `new_window_args_quote_safe` → assert via `parse_new_window_output` + a unit test that the arg vector for a cwd with spaces keeps it one element (test the pure arg-builder `pub fn new_window_args<'a>(cwd: &'a str, cmd: &'a str) -> Vec<&'a str>`).
- [ ] **Step 2: Run to verify FAIL** — `cargo test parse_new_window resolve_claude valid_uuid new_window_args`.
- [ ] **Step 3: Implement** all of Produces; register `favorites::toggle_favorite, favorites::resume_session` in `generate_handler!`; `app.manage(FavState(...))` + `app.manage(LiveIndex(...))` in setup, loading favorites once from `poller::config_dir`.
- [ ] **Step 4: Run to verify PASS** — `cargo test` and `cargo build` (PATH gotcha: prefix `PATH="$HOME/.cargo/bin:$PATH"` if cargo missing).
- [ ] **Step 5: Commit** — `feat: toggle_favorite/resume_sessionコマンドとtmux new-window復帰`

### Task 6: Frontend state machine — tab, `f`, resume routing

**Files:**
- Modify: `src/keys.ts`, `src/render.ts` (types only: `CardState` + `SessionCard` fields)
- Test: `src/keys.test.ts`

**Interfaces:**
- Consumes: payload fields from Task 5 (`sessionUuid`, `cwd`, `starred`, `transcriptMissing`, state `"Dead"`).
- Produces:
  - render.ts types: `CardState` adds `"Dead"`; `SessionCard` adds `sessionUuid: string | null; cwd: string; starred: boolean; transcriptMissing: boolean`.
  - keys.ts: `export type Tab = "all" | "favorites"`; `UiState` gains `tab: Tab` (initial `"all"`); `Action` adds `{ type: "toggleStar"; card: SessionCard }`; new event handling —
    - `Tab` key (both modes): toggle tab, clear query, selection → index 0 of the new visible list, stay in current mode BUT search mode exits to normal (query is cleared anyway — decide: exit to normal; simpler and matches "query is noise against the other list").
    - `f` (normal mode only): `{ type: "toggleStar", card: selected }` (no selection → NONE).
    - `shown`: resets to `initialState` (tab therefore `"all"` — plan's call).
  - Reducer signature unchanged; `main.ts` passes the active tab's filtered list as `visible` (Task 8).

- [ ] **Step 1: Write the failing tests** — `tab_key_switches_tab_and_resets` (normal mode, tab→favorites, query cleared, selection = first of provided visible), `tab_in_search_exits_to_normal`, `f_emits_toggle_star_for_selected`, `f_without_selection_is_none`, `shown_resets_tab_to_all`.
- [ ] **Step 2: Run to verify FAIL** — `pnpm test -- keys` (type errors count as failure).
- [ ] **Step 3: Implement** keys.ts + type additions in render.ts. Update existing tests' card fixtures with the new fields.
- [ ] **Step 4: Run to verify PASS** — `pnpm test`.
- [ ] **Step 5: Commit** — `feat: タブ切替(Tab)とスター(f)のキーバインド`

### Task 7: Frontend rendering — chips, stars, dead cards

**Files:**
- Modify: `src/render.ts`, `src/styles.css`
- Test: `src/render.test.ts`

**Interfaces:**
- Consumes: Task 6 types (`UiState.tab`, Dead state, starred).
- Produces: `renderCards(visible, ui, total, favCount: number)` — 4th param (All-tab total stays `total`; chip shows `favCount`). Markup decisions:
  - Every `.sess` gains `data-uuid="${sessionUuid ?? ""}"`, `data-cwd`, `data-title`, `data-missing="true|false"` and a star: `<button class="star${c.starred ? " on" : ""}" title="favorite">${c.starred ? "★" : "☆"}</button>` placed in `.srow` before `.smeta`.
  - Dead card: dot class `"dead"` (CSS: hollow ring, card opacity ~0.6), idle from `formatIdle` as usual (missing → `"—"` when `idleSecs` is ≥ 2^52 — frontend check `transcriptMissing`), second line replaced by `` `${abbrevHome(cwd)} · resume ↻` `` or `"transcript missing"`.
  - `abbrevHome(p: string)`: leading `/Users/<name>/` → `~/` (pure, exported).
  - Chip bar above the hintbar: `<div class="tabs" data-tauri-drag-region><button class="chip${active all}" data-tab="all">All</button><button class="chip${active fav}" data-tab="favorites">Favorites ${favCount}</button></div>`.
  - Hints: normal adds `· <kbd>f</kbd> ★ · <kbd>⇥</kbd> tab`.
  - Empty favorites tab text: `"No favorites yet — press f on a session"`.

- [ ] **Step 1: Write the failing tests** — `star_glyph_reflects_starred`, `dead_card_shows_cwd_and_resume_hint`, `dead_card_missing_transcript_note`, `chip_bar_marks_active_tab_and_count`, `abbrev_home` (plus update existing render tests for the new 4th arg / fields).
- [ ] **Step 2: Run to verify FAIL** — `pnpm test -- render`.
- [ ] **Step 3: Implement** render.ts + styles (`.tabs`, `.chip`, `.chip.active`, `.star`, `.star.on`, `.dot.dead`, `.sess.dead`).
- [ ] **Step 4: Run to verify PASS** — `pnpm test`.
- [ ] **Step 5: Commit** — `feat: チップバー・スター・死んだお気に入りカードの描画`

### Task 8: Wiring — main.ts payload, clicks, resume

**Files:**
- Modify: `src/main.ts`
- Test: covered by Tasks 6–7 units + end-to-end manual step (this file is the thin DOM layer, per existing convention it has no unit tests)

**Interfaces:**
- Consumes: everything above. `invoke("toggle_favorite", { sessionUuid, cwd, title })`, `invoke("resume_session", { sessionUuid })`.

- [ ] **Step 1: Implement**:
  - `listen<TickPayload>("sessions-updated", …)` with `type TickPayload = { cards: SessionCard[]; favorites: SessionCard[] }`; keep `cards` + new `favs` arrays; `activeList()` = `ui.tab === "all" ? cards : favs`; all `filterCards` calls take `activeList()`.
  - `apply()`: `toggleStar` → invoke toggle_favorite (null uuid handled by backend error → toast); on success, optimistically flip `starred` on the matching card in both arrays and, when it became false, drop it from `favs` — the spec requires unstar to remove the card immediately, not at the next 2s tick; `jump` action routes: `card.paneId.startsWith("dead:")` → `card.transcriptMissing ? showToast("transcript missing — cannot resume") : invoke("resume_session", …)`; else existing jump.
  - Click handling: `.star` click → toggle (and `return` before the jump branch); `.chip` click → `dispatch({type:"key", key:"Tab", …})`-equivalent (add a dedicated `{type:"setTab", tab}` UiEvent in keys.ts if cleaner — implementer's choice, test already pins Tab-key behavior); `.sess` click → same routing as Enter (read `data-*`).
  - `render()` passes `favCount = favs.length` and the active list.
- [ ] **Step 2: Verify** — `pnpm test && pnpm type-check 2>/dev/null || npx tsc --noEmit`; then `PATH="$HOME/.cargo/bin:$PATH" cargo test --manifest-path src-tauri/Cargo.toml`.
Expected: all green.
- [ ] **Step 3: Manual smoke** — `pnpm tauri dev`: star a live session (★ appears), Tab → Favorites tab shows it; quit claude in that pane → card turns dead (dimmed, cwd line); Enter → new tmux window opens in the cwd running `claude --resume`, client lands on it, panel hides.
Expected: each observation as stated; fix before committing otherwise.
- [ ] **Step 4: Commit** — `feat: お気に入りタブ配線とresume連携`

### Task 9: Docs

**Files:**
- Modify: `README.md`, `README.ja.md`

- [ ] **Step 1: Update both READMEs** — Keys table: `f` (toggle favorite), `Tab` (switch All/Favorites); new "Favorites" paragraph (star, tab, dead-session resume via new tmux window at the saved cwd, `favorites.json` location, `claude_bin`/`TCM_CLAUDE_BIN` escape hatch, transcript-missing behavior); Settings table: `claude_bin` row (default `null` → tmux server PATH).
- [ ] **Step 2: Commit** — `docs: お気に入り機能をREADMEに追記`
