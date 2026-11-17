# Favorites (star & resume) — Design

Date: 2026-11-17
Status: draft, pending user approval

## Intent

Let the user star ("favorite") a Claude Code session so it can be recalled
later — even after its tmux pane and claude process are gone. Recalling a
dead favorite resumes the same Claude conversation (`claude --resume <uuid>`)
in a new tmux window opened at the session's original working directory.

Why: important work sessions get closed (laptop restart, tmux server gone,
pane killed) and finding them again today means hunting through
`~/.claude/projects/` by hand. The panel already knows everything needed to
bring one back.

## Goals

- Star/unstar any listed session with one key (`f`) or a click.
- A **Favorites tab** in the panel, switched like CleanClip's chip bar
  (`[All] [Favorites N]` at the bottom), toggled with the `Tab` key or by
  clicking a chip.
- Favorites persist across app restarts and tmux server restarts.
- A dead favorite shows live-derived info (title, last activity summary,
  age) read from its transcript jsonl, not a frozen snapshot.
- Enter on a live favorite = existing jump. Enter on a dead favorite =
  resume: `tmux new-window -c <saved-cwd> "claude --resume <uuid>"`, then
  the normal jump flow (iTerm2 to front, panel hides).

## Non-goals

- No manual reordering of favorites, no notes/tags, no sync between machines.
- No resurrection of sessions whose transcript jsonl was deleted (they stay
  listed with a "transcript missing" note; resume disabled).
- No change to notification policy; dead favorites never notify.
- Not a general session browser for *all* past sessions — only starred ones.

## Anti-goals

- Writing to tmux beyond the one new sanctioned mutation (`new-window` for
  resume). Observation stays read-only.
- Blocking the poll tick on favorites I/O.

## Constraints

- GUI .app: no login shell PATH. tmux binary is already resolved absolutely
  and every tmux spawn forces `LC_ALL=en_US.UTF-8` (`run_tmux`) — resume
  MUST go through the same `run_tmux` path.
- The `claude` binary inside the new window: the command string passed to
  `tmux new-window` is executed by the **tmux server**, which inherited the
  user's login environment (it was started from their shell), so a bare
  `claude` resolves there. Decision: rely on that; if the window flashes
  closed because claude isn't on the server's PATH, the escape hatch is the
  `TCM_CLAUDE_BIN` env var / `claude_bin` setting (below).
- The escaped project-dir name (`escape_project_path`) is lossy — the real
  cwd cannot be recovered from `~/.claude/projects/<escaped>/`. The cwd must
  be captured verbatim at star time.
- Transcript jsonl files are large; dead-favorite info uses the existing
  bounded-tail reader (`read_tail_lines`, 256 KB), same as live cards.
- Frontend stays vanilla TS; no new dependencies, Rust or JS.

## Approaches considered

1. **Inline stars in the single list** — starred-but-dead sessions appended,
   grayed, at the bottom of the one list. Simplest, no modes. Rejected by
   the user: they want a separate view.
2. **Separate Favorites tab (chosen)** — CleanClip-style chip bar, `Tab`
   switches. One list area, the active tab decides its contents. Slightly
   more UI state (active tab interacts with selection + search), but keeps
   the All view exactly as it is today.
3. A separate window/panel for favorites — rejected: two windows to manage,
   fights the summon-one-panel model.

## Design

### Identity & persistence

A favorite is keyed by the Claude **session uuid** (the transcript file
stem).

**Assumption (author's call):** `claude --resume <uuid>` continues the SAME
session id and appends to the same jsonl — forking a new id is opt-in via
Claude Code's separate `--fork-session` flag. The plan's first task verifies
this empirically (resume a throwaway session, confirm the same file's mtime
advances). If a future Claude Code version changes this, the symptom is a
favorite whose card freezes after resume — revisit then; no migration logic
is built now. Stored in `favorites.json` next to `settings.json`
(`~/Library/Application Support/jp.isa.tmux-claude-monitor/favorites.json`):

```json
{
  "favorites": [
    {
      "session_uuid": "01234567-89ab-cdef-0123-456789abcdef",
      "cwd": "/Users/x/work/rainbow",
      "title": "rainbow: fix dedup",
      "starred_at": "2026-11-17T10:00:00Z"
    }
  ]
}
```

`title` is a fallback for display when the transcript is missing; while the
jsonl exists, title/summary/age are derived from it each tick (same parsing
as live cards; pane-ruler titles don't exist for dead sessions, so title
precedence is transcript title record → first user text → saved `title`).
Load/save mirrors `Settings` (read each tick is unnecessary — hold in memory,
write-through on change; re-read only at startup. Manual edits apply after
restart, and the file is documented as app-managed).

### Backend (Rust)

- `SessionCard` gains `session_uuid: Option<String>` (file stem of the
  resolved transcript; `None` until resolved), `cwd: String`, and
  `starred: bool`.
- New module `favorites.rs`: the store (load/save/toggle) + building
  **dead favorite cards**: favorites whose uuid matches no live card this
  tick. Each dead card reuses `SessionCard` with the synthetic
  `pane_id: "dead:<session_uuid>"` — the frontend keys selection, refresh
  reconciliation, and click targets on `paneId`, so dead cards must carry a
  unique one; `%`-prefixed means live, `dead:`-prefixed means resume. When
  a favorite comes back to life its key flips to the real pane id and the
  selection falls back as it does today when a pane vanishes. State is a
  new variant `Dead` (serialized for the frontend),
  `idle_secs` = now − jsonl mtime, summary from the transcript tail, and
  `transcript_missing: bool` when the jsonl is gone.
- The poll tick emits one payload: `{ cards: [...], favorites: [...] }`
  where `favorites` = live starred cards (same objects, by reference
  semantics in JSON: duplicated) + dead favorite cards, sorted by
  `idle_secs` ascending, transcript-missing entries last.
- Tauri commands:
  - `toggle_favorite(session_uuid, cwd, title)` — upsert/remove; returns
    new starred state. No-op (error string) when `session_uuid` is null.
  - `resume_session(session_uuid)` — if a live pane currently maps to that
    uuid, delegate to the existing jump; else
    `run_tmux(["new-window", "-P", "-F",
    "#{session_name}\t#{window_index}\t#{pane_index}", "-c", cwd,
    &format!("{claude_bin} --resume {uuid}")])`. The `-P` output names the
    created window; feed it to the existing `switch_to` so the user's
    attached client lands on the new window no matter which tmux session
    the server created it in, then the existing "bring iTerm2 forward +
    hide panel" flow. Errors (no tmux server / cwd missing / transcript
    missing) surface as the existing error-toast event.
- `claude_bin` resolution for the command string: `TCM_CLAUDE_BIN` env var,
  else new optional setting `claude_bin` in settings.json, else bare
  `"claude"` (tmux server PATH). Shell-quote cwd/uuid defensively (uuid is
  validated `[0-9a-f-]`, cwd goes through `-c` as an argv element, not
  inside the command string).

### Frontend (TS)

- App state gains `activeTab: "all" | "favorites"` (default `all`) and the
  `favorites` array from the tick payload.
- Chip bar rendered under the list, above/merged with the hintbar:
  `[All] [Favorites N]` — active chip highlighted; chips clickable;
  `data-tauri-drag-region` on the bar's non-interactive surface.
- Keys: `Tab` toggles the active tab (normal *and* search mode); `f`
  (normal mode only) toggles star on the selected card. Selection index and
  fuzzy query are **per-tab-switch reset to 0 / kept**: switching tabs
  resets selection to the first card; the search query is cleared (a query
  typed against one list is noise against the other).
- Rendering: every card gets a right-aligned star glyph that is its own
  click target — dim hollow `☆` when unstarred, solid `★` when starred;
  clicking it toggles the favorite (stops propagation), clicking anywhere
  else on the card jumps/resumes as before. Dead
  cards: dimmed, `◌` instead of a status dot, second line
  `<cwd, ~-abbreviated> · resume ↻`, or `transcript missing` note (and
  Enter does nothing for those).
- Enter on favorites tab: live card → `jump` (existing), dead card →
  `resume_session`.

### Edge cases (decided)

- Starring a card whose transcript is unresolved (`session_uuid == null`):
  `f` shows the error toast "no transcript yet for this session".
- The same conversation resumed manually elsewhere: the favorite's uuid
  then matches a live card again — it renders live in the Favorites tab;
  no duplicate dead entry.
- `new-window -c` with a deleted cwd: tmux errors; surfaced as toast.
- Resume while no tmux client is attached: the window is still created
  (new-window works server-side); the jump step then fails with the
  existing toast — acceptable.
- Un-starring a dead favorite removes it from the list immediately (that is
  the only way it disappears besides deletion of favorites.json).

## Left to the builder

- Exact chip-bar markup/CSS (match existing hintbar styling).
- Whether dead-card info is re-read every tick or cached by mtime
  (either is fine within the tick budget; prefer mtime-cached).
- Test fixture shapes, following the existing synthetic-fixture style
  (no real work content — public repo mirror).
