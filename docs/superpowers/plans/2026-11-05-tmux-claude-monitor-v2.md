# tmux Claude Monitor v2 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Turn the v1 panel into a background menu-bar utility summoned by ⌘⇧K, driven by vim-style modal keyboard navigation with fuzzy search, with a tray icon showing liveness and a waiting count.

**Architecture:** Backend (Rust) gains window show/hide/toggle mechanics, the global-shortcut plugin, and a tray icon whose title the existing poller updates each tick. Frontend (vanilla TS) gains two new pure, vitest-tested modules — `fuzzy.ts` (fzf-style scorer) and `keys.ts` (modal state-machine reducer) — and `render.ts`/`main.ts` grow a `UiState` parameter and thin event glue. Jump (Enter or click) always hides the panel, implemented once in the backend command.

**Tech Stack:** Existing v1 stack (Tauri v2, vanilla TS, vitest, cargo test) + `tauri-plugin-global-shortcut`, Tauri `tray-icon` feature, `ActivationPolicy::Accessory`.

**Spec:** `docs/superpowers/specs/2026-11-05-tmux-claude-monitor-v2-design.md` (v1 spec and its anti-goals remain binding; approved mockup `docs/superpowers/specs/mockups/2026-11-05-v2-modes.html`).

## Global Constraints

- All v1 global constraints hold (read-only observation, footprint, absolute binary paths, Japanese commits without attribution lines, no LLM calls).
- Default shortcut **⌘⇧K** = settings key `global_shortcut`, default string `"cmd+shift+k"`, read at launch (restart to change; live re-register optional).
- **Every jump hides the panel** — keyboard Enter AND mouse click alike.
- ⌘⇧K is a pure visibility toggle: show+focus if hidden, hide if visible (focus irrelevant).
- App launches **hidden**; no Dock icon ever (`ActivationPolicy::Accessory`); window close requests hide; quit only via tray menu.
- Panel opens in **normal mode**, first card selected, empty query — on every summon.
- Tray title: empty when no session waits, `● N` when N sessions are in NeedsInputStructured **or** NeedsInputProse.
- Focus theft is an anti-goal: nothing but explicit summon (shortcut, tray, launch of a jump… no — jump hides) may show or focus the panel; the 2s refresh must not steal focus from the search input either.
- Keyboard/fuzzy logic are pure functions in `src/keys.ts` / `src/fuzzy.ts` with vitest coverage; DOM glue stays in `main.ts`.
- New-mode UI (selection highlight, search bar, hint bar, session tag in search mode, match highlighting) follows the approved v2 mockup.

## Review Focus

1. **Shortcut registration fails** (another app owns ⌘⇧K) → app must still run, with the failure visible in the tray menu, not crash or silently lose summon. (Task 3, `shortcut_menu_label` test.)
2. **Japanese IME composition in search** → Enter pressed to confirm a conversion must not jump; composing keystrokes must not trigger normal-mode bindings. (Task 5, `enter_ignored_while_composing` test.)
3. **Query filters everything out** → Enter does nothing (no crash, no wrong jump); header shows `0/N`. (Task 5, `enter_with_no_matches_is_noop` test; Task 6 header test.)
4. **Selected pane vanishes between ticks** (session exits mid-navigation) → selection clamps to nearest, never dangles or crashes. (Task 5, `reconcile_clamps_when_selected_disappears` test.)
5. **Unicode text in fuzzy matching** (Japanese titles, emoji) → positions must index code points, not UTF-16 units, or highlights land mid-character. (Task 4, `fuzzy_positions_are_codepoint_indices` test with Japanese text.)

---

### Task 1: Background window mechanics (hide/show/toggle, jump hides)

**Files:**
- Modify: `src-tauri/src/window.rs` (add show/hide/toggle), `src-tauri/src/jump.rs:30` (AppHandle + hide on success), `src-tauri/src/lib.rs` (close-intercept, Accessory, command), `src-tauri/tauri.conf.json` (window `"visible": false`)

**Interfaces:**
- Consumes: v1 `window::configure_panel`, `jump::jump_to_session`, `tmux::switch_to`.
- Produces (all in `window.rs`):
  - `pub fn show_panel(app: &tauri::AppHandle)` — show + `set_focus` + `emit("panel-shown", ())`.
  - `pub fn hide_panel_now(app: &tauri::AppHandle)` — hide the main window.
  - `pub fn toggle_panel(app: &tauri::AppHandle)` — `is_visible()` ? hide : show.
  - `#[tauri::command] pub fn hide_panel(app: tauri::AppHandle)` — frontend Esc path; registered in `generate_handler!`.
  - `jump_to_session(app: tauri::AppHandle, session: String, window_index: u32, pane_index: u32) -> Result<(), String>` — same behavior plus `hide_panel_now(&app)` after a successful switch (before returning Ok). Signature change is transparent to the frontend (`app` is injected by Tauri).
- `lib.rs` setup additions: `app.set_activation_policy(tauri::ActivationPolicy::Accessory)`; `.on_window_event` intercepting `WindowEvent::CloseRequested` → `api.prevent_close()` + hide.

- [ ] **Step 1: No meaningful pure surface — this task is glue.** Change `tauri.conf.json` main window to `"visible": false`; implement the four functions and the jump change; wire Accessory + close-intercept.
- [ ] **Step 2: Verify** — `cargo build` green; `cargo test` still 33/33 (jump's `switch_target_formatting` untouched).
- [ ] **Step 3: Manual check** — `pnpm tauri dev`: app starts with NO visible panel and NO Dock icon (`pgrep` shows it running). Show/toggle cannot be exercised until the shortcut exists — the full summon/hide/jump-hides check is Task 2 Step 5's manual check; note that in the ledger.
- [ ] **Step 4: Commit** — `git commit -m "v2: バックグラウンド化（起動時非表示・Dockなし・closeで隠す・ジャンプ成功時に自動非表示）"`

### Task 2: Settings + global shortcut

**Files:**
- Modify: `src-tauri/src/settings.rs` (field), `src-tauri/src/lib.rs` (plugin + registration), `src-tauri/Cargo.toml` (`cargo add tauri-plugin-global-shortcut`)

**Interfaces:**
- Consumes: `window::toggle_panel` (Task 1).
- Produces:
  - `Settings.global_shortcut: String`, default `"cmd+shift+k"` (update `Default` impl; `#[serde(default)]` already on the struct fills old files).
  - In `lib.rs` setup: register `settings.global_shortcut` via `tauri_plugin_global_shortcut`, handler (on `ShortcutState::Pressed` only) calls `toggle_panel`. Registration result stored as `pub struct ShortcutStatus(pub Option<String>)` (the error string) in `app.manage(...)` — Task 3's tray menu reads it.

- [ ] **Step 1: Write failing test** in `settings.rs`: extend `settings_roundtrip_and_defaults` with `assert_eq!(s.global_shortcut, "cmd+shift+k");` (defaults case) and include the field in the roundtrip struct literal.
- [ ] **Step 2: Run** `cargo test settings` — FAIL (no field).
- [ ] **Step 3: Implement** field + default + plugin wiring per Interfaces.
- [ ] **Step 4: Run** `cargo test` — all green.
- [ ] **Step 5: Manual check** — `pnpm tauri dev`; press ⌘⇧K: panel appears focused; again: hides. (This also completes Task 1's hidden-launch check.)
- [ ] **Step 6: Commit** — `git commit -m "v2: グローバルショートカット⌘⇧K（settings.global_shortcutで変更可・トグル動作）"`

### Task 3: Tray icon + waiting count

**Files:**
- Create: `src-tauri/src/tray.rs`
- Modify: `src-tauri/src/lib.rs` (feature `tray-icon` in Cargo.toml tauri dep, setup), `src-tauri/src/poller.rs` (title update per tick)

**Interfaces:**
- Consumes: `SessionCard`/`CardState` (v1 poller), `ShortcutStatus` (Task 2), `window::toggle_panel`.
- Produces (in `tray.rs`):
  - `pub fn waiting_count(cards: &[crate::poller::SessionCard]) -> usize` — `NeedsInputStructured` + `NeedsInputProse`.
  - `pub fn tray_title(waiting: usize) -> String` — `""` when 0, else `"● {n}"`.
  - `pub fn shortcut_menu_label(err: Option<&str>) -> Option<String>` — `None` when no error, else `Some("⚠ Shortcut unavailable: {err}")` (rendered as a disabled menu item).
  - `pub fn setup_tray(app: &tauri::App, shortcut_err: Option<String>) -> tauri::Result<()>` — builds the tray (menu: Show/Hide → `toggle_panel`, optional disabled error item, separator, Quit → `app.exit(0)`), stores the `TrayIcon` in `app.manage(TrayHandle(...))`.
  - `pub struct TrayHandle(pub std::sync::Mutex<tauri::tray::TrayIcon>)`.
- Poller: after each `sessions-updated` emit, `let _ = handle.0.lock().unwrap().set_title(Some(tray_title(waiting_count(&cards))))` via `app.try_state::<TrayHandle>()` (skip silently when absent).

- [ ] **Step 1: Write failing tests** in `tray.rs`:

```rust
#[test] fn tray_title_formats() {
    assert_eq!(tray_title(0), "");
    assert_eq!(tray_title(2), "● 2");
}
#[test] fn waiting_counts_structured_and_prose() { /* 4 cards: Working, Structured, Prose, Stale → 2 */ }
#[test] fn shortcut_label_only_on_error() {
    assert!(shortcut_menu_label(None).is_none());
    assert!(shortcut_menu_label(Some("taken")).unwrap().contains("taken"));
}
```

- [ ] **Step 2: Run** `cargo test tray` — FAIL. **Step 3: Implement** (enable `tray-icon` feature). **Step 4: Run** `cargo test` — green.
- [ ] **Step 5: Manual check** — dev run: menu-bar icon present; menu has Show/Hide + Quit; with one session on a permission prompt the title shows `● 1`; Quit exits.
- [ ] **Step 6: Commit** — `git commit -m "v2: トレイ常駐（Show/Hide・Quit・待ちセッション数●N・ショートカット失敗の表示）"`

### Task 4: Fuzzy matcher (`src/fuzzy.ts`)

**Files:**
- Create: `src/fuzzy.ts` — Test: `src/fuzzy.test.ts`

**Interfaces:**
- Consumes: `SessionCard` type from `./render`.
- Produces:
  - `export interface FuzzyResult { score: number; positions: number[] }` — positions are **code-point** indices into the text.
  - `export function fuzzyMatch(query: string, text: string): FuzzyResult | null` — case-insensitive subsequence match (`Array.from` both strings); null when not a subsequence; score favors consecutive runs and segment starts (after `/ - _ : space`), builder picks weights.
  - `export interface ScoredCard { card: SessionCard; field: "title" | "session" | "summary" | null; positions: number[]; score: number }`
  - `export function filterCards(query: string, cards: SessionCard[]): ScoredCard[]` — empty query → all cards in given order with `field: null, positions: [], score: 0`; else per card take the best-scoring field among title / tmuxSession / summary, drop non-matching cards, sort by score desc (stable).

- [ ] **Step 1: Write failing tests** (`fuzzy.test.ts`): subsequence hit (`fuzzyMatch("rai","rainbow: fix")!.positions` = `[0,1,2]`); case-insensitive; non-subsequence → null; consecutive run outscores scattered (`fuzzyMatch("dedup", "dedup fix")!.score > fuzzyMatch("dedup","d-e-d-u-p")!.score`); `fuzzy_positions_are_codepoint_indices`: `fuzzyMatch("複数","rainbow: web 複数予約")!.positions` = `[12,13]` (code points, not UTF-16); `filterCards("")` returns all with field null; `filterCards("rai", …)` drops non-matches, best field wins, sorted by score.
- [ ] **Step 2: Run** `pnpm test` — FAIL (module missing). **Step 3: Implement.** **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** — `git commit -m "v2: fzf風ファジーマッチャ追加（コードポイント位置・フィールド別ベストマッチ）"`

### Task 5: Modal key reducer (`src/keys.ts`)

**Files:**
- Create: `src/keys.ts` — Test: `src/keys.test.ts`

**Interfaces:**
- Consumes: `ScoredCard` (Task 4), `SessionCard` (render).
- Produces:
  - `export type Mode = "normal" | "search"`
  - `export interface UiState { mode: Mode; query: string; selectedPaneId: string | null }`
  - `export const initialState: UiState` — `{ mode: "normal", query: "", selectedPaneId: null }`
  - `export type UiEvent = { type: "key"; key: string; ctrl: boolean; composing: boolean } | { type: "query"; value: string } | { type: "shown" }`
  - `export type Action = { type: "none" } | { type: "jump"; card: SessionCard } | { type: "hide" }`
  - `export function reduce(state: UiState, ev: UiEvent, visible: ScoredCard[]): { state: UiState; action: Action }`
    - `shown` → `initialState` with first visible card selected.
    - `query` → set query, select first match.
    - keys with `composing: true` → no-op (state unchanged).
    - normal mode: `j`/`ArrowDown`, `k`/`ArrowUp` move (clamped); `g` first / `G` last; `/` → search mode (query `""`); `Enter` → jump selected (none when list empty); `Escape` → hide.
    - search mode: `Escape` → normal + query cleared; `Enter` → jump selected (none when empty); `Backspace` with empty query → normal; `ctrl+j`/`ctrl+k`/arrows move within matches. Printable keys are NOT handled here (the real `<input>` owns text; `query` events carry the result).
  - `export function reconcile(state: UiState, visible: ScoredCard[]): UiState` — selected pane still visible → unchanged; gone → select the card now at the old index (clamped to last), null when list empty.

- [ ] **Step 1: Write failing tests** covering: j/k movement + clamping at both ends; g/G; `/` enters search; Esc-in-search clears query and returns to normal; Esc-in-normal → `hide`; Enter → `jump` with the selected card; `enter_with_no_matches_is_noop` (empty visible → action none, state unchanged); `enter_ignored_while_composing` (`composing: true` → none); Backspace on empty query → normal mode; `query` event selects top match; ctrl+j moves in search mode; `shown` resets a dirty state to normal/empty/first; `reconcile_clamps_when_selected_disappears` (selected id removed → same index clamped; empty list → null).
- [ ] **Step 2: Run** `pnpm test` — FAIL. **Step 3: Implement.** **Step 4: Run** — PASS.
- [ ] **Step 5: Commit** — `git commit -m "v2: vimモーダルキーリデューサ追加（normal/search・IME合成中は無視・選択の再整合）"`

### Task 6: Panel UI v2 (render + wiring)

**Files:**
- Modify: `src/render.ts`, `src/main.ts`, `src/styles.css`, `src/render.test.ts`

**Interfaces:**
- Consumes: Tasks 4–5 exports; backend `hide_panel` command and `panel-shown` event (Task 1).
- Produces:
  - `renderCards(visible: ScoredCard[], ui: UiState, total: number): string` (replaces the 1-arg form):
    - header count: `CLAUDE SESSIONS — N` in normal mode, `K/N` in search mode (K = visible length).
    - search mode adds `<div class="searchbar">` containing `/` glyph and `<input id="q">` (value = ui.query), and a per-card `<span class="stag">` session-name tag.
    - selected card (`ui.selectedPaneId`) gets class `selected` (style per mockup: `border-color:#7aa2f7; background:#20283a`).
    - matched characters (positions on the matched field) wrapped in `<span class="hl">` — escape-then-wrap per code point so HTML-escaping cannot split a highlight.
    - hint bar per mockup: normal `j/k move · g/G top/bottom · / search · ↵ jump · esc hide`; search `type filter · ↵ jump to selected · esc clear → normal · ^j/^k move`.
  - `main.ts`: holds `UiState` + latest cards; `document.addEventListener("keydown", …)` builds `UiEvent` (`composing: e.isComposing`), passes `filterCards(state.query, cards)` as `visible`; actions: `jump` → `invoke("jump_to_session", …)` (backend hides); `hide` → `invoke("hide_panel")`; `listen("panel-shown")` → `shown` event + render; `sessions-updated` → `reconcile` + render. After every render in search mode: restore `#q` value, focus, caret to end (the 2s refresh must not steal the input's focus — set only if `document.activeElement` isn't already the input, else leave the live input alone and skip rewriting its value… builder: simplest correct approach is to skip re-rendering the searchbar input element itself; decide and keep the no-focus-theft constraint).
  - keydown must not double-handle keys that the input itself processes: only `Enter`, `Escape`, `Backspace`(empty), ctrl-combos and arrows go to `reduce` while the input is focused; plain printable keys reach the input, whose `input` event emits `{type:"query"}`.

- [ ] **Step 1: Write failing vitest** (extend `render.test.ts`, new 2-arg calls): selected card carries `selected`; search mode renders searchbar + `K/N` header + session tag; normal mode has neither; hint bar strings per mode; highlight: card titled `x<b>複数` with positions on `複数` renders `&lt;b&gt;` escaped AND `<span class="hl">複</span>` present (escape-then-wrap).
- [ ] **Step 2: Run** `pnpm test` — FAIL. **Step 3: Implement** render + main + CSS (searchbar/selected/stag/hl/hintbar styles copied from the v2 mockup). **Step 4: Run** `pnpm test` + `pnpm build` — green.
- [ ] **Step 5: Manual check** — dev run: ⌘⇧K → panel, `j`/`k` move highlight, `/` + `rai` filters with highlights + tags, Enter jumps to the match AND hides, ⌘⇧K → Esc hides; mouse click on a card jumps and hides.
- [ ] **Step 6: Commit** — `git commit -m "v2: パネルUI（選択ハイライト・検索バー・ヒントバー・マッチ強調・クリックでも非表示）"`

### Task 7: Docs, release, acceptance

**Files:**
- Modify: `README.md`

- [ ] **Step 1: README** — add: summon/keys table (⌘⇧K, j/k/g/G, /, Enter, Esc), background behavior (hidden launch, tray = running signal, quit from tray), `global_shortcut` settings row (restart to apply), login-item note (System Settings → open at login).
- [ ] **Step 2: Full suites** — `pnpm test` and `cargo test` green; `pnpm tauri build`; reinstall to `/Applications` (quit the running copy first).
- [ ] **Step 3: Acceptance (spec sketch)** — machine-verifiable: launch → no panel, no Dock icon, tray present; user items: ⌘⇧K flow (`jj↵` lands in 3rd session, panel gone, focus back in iTerm2), `/rai↵` jump, Esc/⌘⇧K hide, tray `● N` while hidden, Quit works. Record results.
- [ ] **Step 4: Commit** — `git commit -m "v2リリース: READMEにキー操作・常駐挙動・ログイン項目手順を追記"`
