import { describe, expect, test } from "vitest";
import { renderCards, renderSearchBar, formatIdle, type SessionCard } from "./render";
import type { ScoredCard } from "./fuzzy";
import type { UiState } from "./keys";

function card(over: Partial<SessionCard> = {}): SessionCard {
  return {
    paneId: "%1",
    tmuxSession: "neovim",
    windowIndex: 2,
    paneIndex: 0,
    title: "rainbow: fix dedup",
    summary: "Editing retry.rs",
    state: "Working",
    idleSecs: 5,
    ...over,
  };
}

const plain = (c: SessionCard): ScoredCard => ({ card: c, field: null, positions: [], score: 0 });

const normal: UiState = { mode: "normal", query: "", selectedPaneId: null };
const searching: UiState = { mode: "search", query: "rai", selectedPaneId: "%1" };

describe("renderCards", () => {
  test("renders one .sess per card and a counting header", () => {
    const html = renderCards([plain(card()), plain(card({ paneId: "%2" }))], normal, 2);
    expect(html.match(/class="sess/g)?.length).toBe(2);
    expect(html).toContain("CLAUDE SESSIONS — 2");
  });

  test("search mode header shows K/N", () => {
    const html = renderCards([plain(card())], searching, 5);
    expect(html).toContain("CLAUDE SESSIONS — 1/5");
  });

  test("needs-input cards (structured and prose) carry attn and wait dot", () => {
    for (const state of ["NeedsInputStructured", "NeedsInputProse"] as const) {
      const html = renderCards([plain(card({ state }))], normal, 1);
      expect(html).toContain("sess attn");
      expect(html).toContain("dot wait");
    }
  });

  test("working gets work dot, stale gets stale dot and hours idle text", () => {
    expect(renderCards([plain(card())], normal, 1)).toContain("dot work");
    const html = renderCards(
      [plain(card({ state: "Stale", idleSecs: 3 * 3600 + 120 }))],
      normal,
      1,
    );
    expect(html).toContain("dot stale");
    expect(html).toContain("3h");
  });

  test("card carries jump target data attributes", () => {
    const html = renderCards([plain(card())], normal, 1);
    expect(html).toContain('data-session="neovim"');
    expect(html).toContain('data-window="2"');
    expect(html).toContain('data-pane="0"');
  });

  test("escapes HTML in titles and summaries", () => {
    const html = renderCards(
      [plain(card({ title: 'x<b>"evil"</b>', summary: "a<i>b</i>" }))],
      normal,
      1,
    );
    expect(html).not.toContain("<b>");
    expect(html).not.toContain("<i>");
    expect(html).toContain("&lt;b&gt;");
  });

  test("selected card carries selected class", () => {
    const html = renderCards(
      [plain(card()), plain(card({ paneId: "%2" }))],
      { ...normal, selectedPaneId: "%2" },
      2,
    );
    expect(html.match(/class="sess[^"]*selected/g)?.length).toBe(1);
  });

  test("search mode renders session tags but NOT the searchbar (it lives outside the re-rendered region)", () => {
    const s = renderCards([plain(card())], searching, 1);
    expect(s).toContain('class="stag"');
    expect(s).not.toContain("searchbar");
    const n = renderCards([plain(card())], normal, 1);
    expect(n).not.toContain("searchbar");
    expect(n).not.toContain("stag");
  });

  test("renderSearchBar renders the input with the query escaped", () => {
    const s = renderSearchBar('ra"i');
    expect(s).toContain('class="searchbar"');
    expect(s).toContain('id="q"');
    expect(s).toContain('value="ra&quot;i"');
  });

  test("non-interactive chrome (hint bar, empty notice) is a window drag region", () => {
    const html = renderCards([], normal, 0);
    expect(html).toContain('<div class="hintbar" data-tauri-drag-region>');
    expect(html).toContain('<div class="empty" data-tauri-drag-region>');
  });

  test("hint bar per mode", () => {
    expect(renderCards([], normal, 0)).toContain("/</kbd> search");
    expect(renderCards([], searching, 0)).toContain("esc</kbd> clear");
  });

  test("escape-then-wrap highlighting cannot split escaped entities", () => {
    // title "x<b>複数" with the match on 複数 (code points 4,5)
    const sc: ScoredCard = {
      card: card({ title: "x<b>複数" }),
      field: "title",
      positions: [4, 5],
      score: 9,
    };
    const html = renderCards([sc], searching, 1);
    expect(html).toContain("&lt;b&gt;");
    expect(html).toContain('<span class="hl">複</span>');
    expect(html).not.toContain("<b>");
  });

  test("session-name match highlights inside the tag", () => {
    const sc: ScoredCard = {
      card: card({ tmuxSession: "raiden" }),
      field: "session",
      positions: [0, 1, 2],
      score: 9,
    };
    const html = renderCards([sc], searching, 1);
    expect(html).toContain(`<span class="stag"><span class="hl">r</span>`);
  });
});

describe("formatIdle", () => {
  test("now / minutes / hours", () => {
    expect(formatIdle(30)).toBe("now");
    expect(formatIdle(180)).toBe("3m");
    expect(formatIdle(7200)).toBe("2h");
  });
});
