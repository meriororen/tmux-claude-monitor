# tmux Claude Monitor v2 — Summon, Keyboard, Tray — Design

**Date:** 2026-11-05
**Status:** Draft for approval
**Builds on:** v1 (`2026-11-05-tmux-claude-monitor-design.md`), implemented on `feature/initial-implementation`.

## Intent

v1 is a passive always-on-top panel. In practice the user wants it out of
sight until needed: press a global shortcut from anywhere, the panel appears
with keyboard focus, pick a session entirely from the keyboard (vim-style,
with fuzzy search), Enter lands in that tmux pane, and the panel is gone
again. A menu-bar (tray) icon shows the app is alive even while the panel is
hidden.

## Goals

- **Background app.** Closing or hiding the panel never quits the app; the
  poller keeps running (notifications and tray state keep working). The app
  has no Dock icon — it is a menu-bar utility (activation policy Accessory).
- **Global shortcut toggle.** Default **⌘⇧K** (configurable as
  `global_shortcut` in settings.json). Press anywhere → panel shows at its
  saved position and takes keyboard focus. Press again → panel hides and
  focus returns to the previous app. Esc (in normal mode) also hides.
- **Vim-style keyboard navigation (modal).** The panel opens in **normal
  mode** with the first card selected (visible highlight):
  - `j` / `k` — move selection down / up (also arrow keys)
  - `g` / `G` — first / last card
  - `Enter` — jump to the selected session (v1 jump) and hide the panel
  - `/` — enter search mode
  - `Esc` — hide the panel
- **Fuzzy search (search mode).** `/` shows a search bar; every keystroke
  filters the card list live. Matching is fzf-style subsequence matching
  over title + tmux session name + summary, best score first, matched
  characters highlighted. `Enter` jumps to the selected match and hides;
  `Esc` clears the query and returns to normal mode; `Ctrl+j`/`Ctrl+k` (and
  arrows) move the selection while typing. `Backspace` on an empty query
  also returns to normal mode.
- **Tray icon.** Always present while the app runs (that is the "is it
  running" signal). Menu: Show/Hide panel, Quit. The tray title shows a
  waiting count when any session is in needs-input (e.g. `● 2`), cleared
  when none are.
- A hint bar at the bottom of the panel shows the active mode's keys (per
  the approved v2 mockup `mockups/2026-11-05-v2-modes.html`).
- Mouse behavior from v1 (click card to jump, ⚙, drag) is unchanged.

## Non-goals (v2)

- Launch at login (user can add the app as a login item in System Settings;
  README notes this).
- Vim counts/operators (`3j`, `dd`), marks, or search history. `/` + type +
  Enter/Esc is the whole grammar.
- Search operators (regex, `!negation`, field prefixes).
- Changing the global shortcut without an app restart (documented; a live
  re-register is a welcome bonus if trivial, builder's option).
- Answering/typing into the Claude session from the panel (v1 non-goal
  stands: the panel is read-only; interaction happens in the terminal).

## Anti-goals

- **Focus theft.** The panel takes focus only on explicit summon (shortcut,
  tray menu, click). A 2s refresh, a state change, or a notification must
  never grab focus or re-show a hidden panel.
- **A Dock-resident app.** If it shows in the Dock or the ⌘Tab switcher, the
  "background utility" goal has failed.
- **Footprint regressions.** Hidden or shown, the poll loop stays as cheap
  as v1 (the poller must keep running while hidden for notifications and the
  tray count — do not add per-tick work beyond the tray title update).
- **Shortcut squatting surprises.** If the configured shortcut fails to
  register (conflict), the app must still run and say so (tray menu item
  showing the error state), not crash or silently have no summon.

## Constraints

- Same stack as v1: Tauri v2, Rust backend, vanilla TS frontend, vitest +
  cargo test. New Tauri pieces: `tauri-plugin-global-shortcut`, the
  `tray-icon` feature, `set_activation_policy(Accessory)`.
- Keyboard handling and fuzzy matching live in the frontend as pure,
  vitest-tested functions (same pattern as v1 `render.ts`): a mode/selection
  state machine reducer and a fuzzy scorer, with DOM/event glue kept thin.
- Selection is keyed by `paneId`, not list index, so the 2s refresh does not
  move the user's highlight; if the selected pane disappears, selection
  clamps to the nearest index.
- The approved mockup for the two modes (selection highlight, search bar,
  hint bar, match highlighting) is
  `docs/superpowers/specs/mockups/2026-11-05-v2-modes.html`.
- v1's spec and anti-goals (read-only observation, notification policy,
  mockup-A card anatomy) remain binding.

## Decided design

- **Summon/hide mechanics (decided with the user):** ⌘⇧K is a pure
  visibility toggle — show+focus if hidden, hide if visible, regardless of
  focus. **Every jump hides the panel: keyboard Enter AND mouse click-jump
  alike** (summon → pick → gone; this supersedes v1's click-keeps-panel
  behavior and makes a visible-but-unfocused panel rare). Window
  `CloseRequested` is intercepted → hide instead of close. Quit happens only
  via the tray menu (or ⌘Q while the panel is focused — builder may keep or
  suppress; suppressing is fine).
- **Launch state (designer's call):** the app launches hidden — the tray
  icon is the "it started" signal, matching the login-item background-utility
  pattern (Raycast-style). README says so.
- **Session-name matches visible (designer's call):** in search mode each
  card shows its tmux session name as a small gray tag, so a card matched
  via its session name shows its highlight there instead of appearing to
  match nothing. Normal mode stays tag-free (v1 card anatomy).
- **Modal state machine (frontend):**
  `{ mode: "normal" | "search", query: string, selectedPaneId: string | null }`
  with a pure `handleKey(state, key, cards) → { state, action? }` where
  `action` ∈ {jump(card), hide, none}. The DOM listener applies actions via
  the existing `invoke` calls plus a new `hide_panel` command.
- **Fuzzy scorer (frontend):** `fuzzyMatch(query, text) → { score, positions } | null`
  — case-insensitive subsequence match; bonuses for consecutive matches and
  word/segment starts; the card list in search mode is sorted by best score
  across (title, session, summary), with the matched field's characters
  wrapped for highlighting.
- **Tray (backend):** built at setup with menu items Show/Hide and Quit; the
  poller, after each tick, sets the tray title to `● <n>` where n = cards in
  `NeedsInputStructured` or `NeedsInputProse` (empty title when 0). Tray
  clicks use the same show/hide path as the shortcut.
- **Settings additions:** `global_shortcut: String` (default
  `"Cmd+Shift+K"`), read at launch.
- **Default shortcut ⌘⇧K** was confirmed by the user.

## Left to the builder

- Exact Tauri APIs/wiring for activation policy, tray construction, and
  close-interception (follow current Tauri v2 docs).
- Fuzzy scoring weights; keep the function pure and tested.
- How match highlighting composes with HTML escaping in `renderCards`
  (escape first, then wrap positions — or equivalent safe approach).
- Whether ⌘Q is suppressed or quits (either acceptable; tray Quit must work).
- Hint-bar copy details, selected-card style (follow the mockup).
- Live shortcut re-registration on settings change (optional bonus).

## Acceptance sketch

With the app running and the panel hidden, from inside a fullscreen iTerm2:

- ⌘⇧K shows the panel with focus; `j j Enter` lands in the third session and
  the panel is hidden again; focus is back in iTerm2.
- ⌘⇧K, `/`, typing `rai` filters to rainbow sessions with matched letters
  highlighted; `Enter` jumps to the top match.
- A mouse click on a card also jumps AND hides the panel (same as Enter).
- Esc in normal mode hides; ⌘⇧K twice shows then hides; ⌘W (the window is
  frameless, so there is no close button) hides but the tray icon remains;
  Quit from the tray actually exits. App launch shows no panel — only the
  tray icon.
- No Dock icon at any point; a session entering needs-input while the panel
  is hidden updates the tray title to `● 1` and notifies (structured only),
  without showing the panel.
- All v1 tests still pass; new pure functions (state machine, fuzzy) have
  vitest coverage.
