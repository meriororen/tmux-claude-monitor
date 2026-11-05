import { describe, expect, test } from "vitest";
import { fuzzyMatch, filterCards } from "./fuzzy";
import type { SessionCard } from "./render";

function card(over: Partial<SessionCard> = {}): SessionCard {
  return {
    paneId: "%1",
    tmuxSession: "neovim",
    windowIndex: 0,
    paneIndex: 0,
    title: "rainbow: fix dedup",
    summary: "Editing retry.rs",
    state: "Idle",
    idleSecs: 5,
    ...over,
  };
}

describe("fuzzyMatch", () => {
  test("subsequence hit with positions", () => {
    expect(fuzzyMatch("rai", "rainbow: fix")!.positions).toEqual([0, 1, 2]);
  });

  test("case-insensitive", () => {
    expect(fuzzyMatch("RAI", "rainbow")).not.toBeNull();
    expect(fuzzyMatch("rai", "RAINBOW")).not.toBeNull();
  });

  test("non-subsequence returns null", () => {
    expect(fuzzyMatch("xyz", "rainbow")).toBeNull();
  });

  test("consecutive run outscores scattered", () => {
    const run = fuzzyMatch("dedup", "dedup fix")!.score;
    const scattered = fuzzyMatch("dedup", "d-e-d-u-p")!.score;
    expect(run).toBeGreaterThan(scattered);
  });

  test("fuzzy_positions_are_codepoint_indices", () => {
    // "rainbow: web 複数予約" — 複 is code-point index 13 (not a UTF-16 offset issue
    // here, but Array.from semantics must hold for any astral chars too)
    expect(fuzzyMatch("複数", "rainbow: web 複数予約")!.positions).toEqual([13, 14]);
  });

  test("empty query matches with no positions", () => {
    expect(fuzzyMatch("", "anything")).toEqual({ score: 0, positions: [] });
  });
});

describe("filterCards", () => {
  test("empty query returns all in order, field null", () => {
    const cards = [card(), card({ paneId: "%2" })];
    const out = filterCards("", cards);
    expect(out.map((s) => s.card.paneId)).toEqual(["%1", "%2"]);
    expect(out.every((s) => s.field === null && s.positions.length === 0)).toBe(true);
  });

  test("drops non-matches and picks the best field", () => {
    const cards = [
      card({ paneId: "%1", title: "rainbow: fix", tmuxSession: "s1", summary: "none" }),
      card({ paneId: "%2", title: "work: other", tmuxSession: "s2", summary: "nothing here" }),
      card({ paneId: "%3", title: "zzz", tmuxSession: "raiden", summary: "zzz" }),
    ];
    const out = filterCards("rai", cards);
    expect(out.map((s) => s.card.paneId).sort()).toEqual(["%1", "%3"]);
    expect(out.find((s) => s.card.paneId === "%1")!.field).toBe("title");
    expect(out.find((s) => s.card.paneId === "%3")!.field).toBe("session");
  });

  test("sorted by score descending", () => {
    const cards = [
      card({ paneId: "%scattered", title: "d-e-d-u-p", summary: "x", tmuxSession: "x" }),
      card({ paneId: "%exact", title: "dedup now", summary: "x", tmuxSession: "x" }),
    ];
    const out = filterCards("dedup", cards);
    expect(out[0].card.paneId).toBe("%exact");
  });
});
