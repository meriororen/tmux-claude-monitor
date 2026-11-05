# tmux Claude Monitor — Design

**Date:** 2026-11-05
**Status:** Draft for approval

## Intent

The user runs several persisted tmux sessions, most containing a Claude Code
session (often in one pane, next to nvim/terminal panes). Finding "which
session is working on what, which one is waiting for my feedback, which one is
stale" currently means cycling `ctrl+b n/p` and reading each screen. tmux
session names exist but are inconsistently maintained, and a name alone cannot
say what a Claude is *currently* doing or whether it is blocked.

This project is a small always-on-top desktop panel (macOS) that watches all
tmux sessions, finds every running Claude Code session, and shows a glanceable
card per Claude session: derived title, one-line summary of current activity,
and state (working / needs input / stale). Clicking a card jumps straight to
that pane in iTerm2. When a session flips to "needs input," the app fires a
macOS notification.

## Goals

- Discover every Claude Code session running inside any pane of any tmux
  session on the local machine, automatically — no manual registration, no
  reliance on tmux session names.
- Per session, show at a glance:
  - **status dot**: green = actively working, amber = waiting for user input
    (permission prompt or question), gray = idle/stale
  - **title**: short, meaningful, derived from the session itself
  - **one-line summary** of current/last activity
  - **idle time** (e.g. `now`, `2m`, `3h`)
- Highlight needs-input cards (amber card background per the approved mockup)
  and send a macOS notification on the transition into that state.
- Click a card → focus iTerm2 and switch the attached tmux client to that
  session/window/pane.
- Compact layout (approved mockup option A): one line per session (dot +
  title + idle time) plus one summary line; ~8 sessions visible in a ~340px
  wide panel without scrolling.
- Always-on-top, corner-pinned panel that stays out of the way — and stays
  visible on **all macOS Spaces and over fullscreen apps** (decided with the
  user). If Tauri v2's window flags (`visible_on_all_workspaces`,
  always-on-top level) don't cover the fullscreen-auxiliary case, a small
  native NSWindow shim (`collectionBehavior` with `canJoinAllSpaces` +
  `fullScreenAuxiliary`, elevated window level) is in scope for v1.

## Non-goals (v1)

- Linux/Windows support, or terminals other than iTerm2 (the jump-to action
  is iTerm2-specific; detection itself is terminal-agnostic).
- Sending input to Claude sessions from the panel (answering prompts,
  typing). The panel is read-only; interaction happens in the terminal.
- Monitoring Claude Code sessions running *outside* tmux.
- LLM-based summarization. Titles/summaries are heuristic in v1 (free,
  instant). An optional cheap-model summarizer is a possible v2.
- History, analytics, or persistence of past sessions. Only the live picture.
- Modifying the user's Claude Code configuration (no required hooks or
  statusline changes). A hook-based signal could be an optional later
  improvement, not a v1 requirement.

## Anti-goals

Things that would make this a failure even if it "works":

- **Heavy footprint.** This runs all day. Polling must be cheap (shelling out
  to `tmux` and tailing files — no per-tick subprocess storms, no pegged CPU,
  no hundreds of MB of RAM).
- **Interfering with sessions.** Observation must be strictly read-only:
  `tmux capture-pane`/`list-panes` and reading transcript files. Never
  `send-keys`, never writing to Claude's files, never attaching/detaching
  clients except on an explicit card click.
- **Requiring discipline from the user.** If it only works when tmux sessions
  are consistently named or panes arranged a certain way, it has failed — the
  whole point is that naming consistently is hard.
- **Noisy notifications.** One notification per transition into needs-input,
  not repeated nagging while a prompt sits unanswered.

## Constraints

- macOS (Apple Silicon dev machine, Darwin 25.x), tmux, iTerm2, Claude Code
  CLI installed at the usual locations.
- Tauri (v2) app: Rust backend for the monitoring/jump logic, web frontend
  for the panel UI. Decided with the user.
- Claude Code's transcript files live under
  `~/.claude/projects/<escaped-cwd>/<session-uuid>.jsonl` (the escaped cwd
  replaces `/` and some characters with `-`). Format is JSONL of
  user/assistant/tool events; treat it as an undocumented format that may
  change — parse defensively.

## Decided design (as decided with the user)

### Form factor

- Small always-on-top panel pinned in a screen corner. Frameless or minimal
  chrome; dark theme per the approved mockup.
- The approved mockup is checked in at
  `docs/superpowers/specs/mockups/2026-11-05-panel-layout.html` (option A,
  "Compact", is the approved one; option B was rejected). It is the
  authoritative reference for colors, spacing, card anatomy, and the header
  treatment.
- Compact card layout (mockup option A):
  - Header row: `CLAUDE SESSIONS — <n>` and a settings affordance.
  - Card row 1: status dot, bold title (truncated with ellipsis), idle-time
    on the right.
  - Card row 2: one-line summary, truncated.
  - Needs-input card: amber border/background + `⚠ Waiting: <what it wants>`
    as the summary line.

### Detection pipeline (polling)

Poll on a short interval (default ~2s; configurable):

1. **Enumerate panes:** `tmux list-panes -a` with a `-F` format giving
   session name, window/pane index, pane id, pane PID, current command,
   current path, tty.
2. **Identify Claude panes:** a pane whose foreground process tree contains a
   Claude Code process. `pane_current_command` alone may show `node` or a
   wrapper, so check the pane PID's descendants (`ps`) for the claude CLI.
3. **Map pane → transcript:** resolve the claude process's cwd to the escaped
   project directory under `~/.claude/projects/`, then pick the session
   `.jsonl` the process actually has open (suggestion: `lsof -p <pid>` on the
   claude PID is the precise mapping; fall back to the most recently modified
   `.jsonl` in that project dir). Exact mechanism left to the builder, but it
   must handle two Claude sessions in the same project directory.
4. **State detection** from `tmux capture-pane -p` on the Claude pane
   (read-only), classified by heuristics:
   - **working**: the running indicator is visible (e.g. the `esc to
     interrupt` spinner line).
   - **needs input**: a permission dialog, AskUserQuestion option box, plan
     approval, or trailing open question is on screen awaiting the user.
     Two sub-kinds, decided with the user: **structured** prompts
     (permission dialogs, option pickers, plan approvals) go amber *and*
     notify; a turn that merely ends with a **prose** question ("Want me to
     also…?") goes amber but never fires a notification — otherwise nearly
     every completed turn would ping.
   - **idle**: an empty prompt with no pending work.
   - **stale**: idle with no transcript/pane activity for longer than a
     threshold (default 30 min, configurable).
   Heuristic patterns live in one well-commented module so they are easy to
   update when Claude Code's UI strings change.
5. **Title and summary** derived heuristically from the transcript (no LLM
   calls):
   - Title: prefer Claude Code's own summary records in the JSONL when
     present; otherwise the first user message, truncated; prefix or fall
     back to the project directory basename. (Exact precedence is the
     builder's call; the requirement is "meaningful at a glance without the
     user naming anything.")
   - Summary: the most recent meaningful activity — current tool action,
     last assistant text snippet, or for needs-input, what it is asking for.
   - Idle time: since the last transcript event (or pane content change).

### Jump-to action

On card click:

1. Activate iTerm2 (AppleScript via `osascript`, or Tauri shell).
2. `tmux switch-client -t <session>` on the attached client, then
   `select-window` / `select-pane` to land on the exact Claude pane.
3. Multiple attached clients: switch the most recently active client
   (builder's call on mechanism).
4. If no client is attached to the tmux server, open a new iTerm2 window that
   attaches (`tmux attach -t <session>`) — nice-to-have; at minimum do
   nothing gracefully rather than erroring.

### Notifications

- macOS user notification (Tauri notification plugin) when a session
  transitions into **structured** needs-input (see state detection); prose
  trailing questions never notify. Include title + what it's waiting for.
- Clicking the notification should jump to the session if the plugin
  supports click-through on macOS; otherwise notification is inform-only
  (builder verifies plugin capability).
- No repeat notifications for the same unanswered prompt. New notification
  only after the state has left and re-entered needs-input.
- App launch with sessions already waiting (designer's call): notify once on
  first observation, and if more than one session is already waiting,
  coalesce into a single summary notification ("2 sessions waiting for
  input") instead of a burst.

### Settings (minimal)

- Poll interval, stale threshold, panel corner/position (or just remember
  last dragged position), notifications on/off.
- Stored locally (Tauri store or a plain JSON config file).

## Left to the builder

- Frontend stack inside Tauri (vanilla TS vs a small framework) — keep it
  light.
- Exact `ps`/`lsof` strategy and caching so per-tick cost stays low (e.g.
  only re-run `lsof` when a new claude PID appears, not every tick).
- Exact capture-pane heuristic patterns and their test fixtures (capture real
  pane snapshots of each state as fixtures).
- tmux output parsing details and format strings.
- Multi-monitor / corner-pinning mechanics in Tauri.
- Whether title/summary parsing tails the JSONL incrementally or re-reads a
  bounded tail of the file each tick (either is fine if cheap).
- App name/branding. Repo lives at `~/work/tmux-claude-monitor` (own git
  repo, personal — not a work project).

## Acceptance sketch

With 3+ tmux sessions running Claude Code in various states (one working, one
sitting on a permission prompt, one idle for hours) plus panes running nvim
and plain shells:

- The panel lists exactly the Claude sessions (not the nvim/shell panes),
  each with a sensible title and summary, correct status colors, and idle
  times.
- The permission-prompt session is amber-highlighted and produced exactly one
  macOS notification when it started waiting.
- Clicking any card lands the user on that pane in iTerm2 within ~a second.
- The app idles at negligible CPU and modest memory while polling.
