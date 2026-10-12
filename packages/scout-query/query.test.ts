import { describe, expect, test } from "vitest";
import { buildSearchQuery, EMPTY_FILTERS, type Filters, parseSearch, scopeToFilters } from "./src/query";

function build(raw: string, filters: Partial<Filters> = {}) {
  return buildSearchQuery({
    raw,
    filters: { ...EMPTY_FILTERS, ...filters },
    source: null,
    color: null,
    sort: "matches",
    mode: "keyword",
    page: 0,
    pageSize: 80,
  });
}

describe("buildSearchQuery filter merge", () => {
  test("popover type checkboxes combine (OR) into types", () => {
    expect(build("", { types: ["tweets", "articles"] }).types.sort()).toEqual([
      "articles",
      "tweets",
    ]);
  });

  test("a ty: token merges with popover types (deduped)", () => {
    expect(build("ty:tweets", { types: ["articles"] }).types.sort()).toEqual([
      "articles",
      "tweets",
    ]);
    // single-type field is superseded by the list
    expect(build("ty:tweets", { types: ["articles"] }).type).toBeNull();
  });

  test("boolean filters OR with typed tokens", () => {
    const p = build("i:", { favorite: true });
    expect(p.has_image).toBe(true);
    expect(p.favorite).toBe(true);
  });

  test("time filter folds into after", () => {
    expect(build("", { time: "t:30d" }).after).toMatch(/^\d{4}-\d{2}-\d{2}$/);
  });
});

describe("has_image filter", () => {
  test("i: token sets has_image and is removed from free text", () => {
    const p = parseSearch("i: karpathy");
    expect(p.has_image).toBe(true);
    expect(p.positive_terms).toContain("karpathy");
  });

  test("plain query leaves has_image false", () => {
    expect(parseSearch("karpathy").has_image).toBe(false);
  });

  test("img scope maps to has_image", () => {
    expect(scopeToFilters("img")).toEqual({ has_image: true });
  });
});

describe("repeated fields", () => {
  test("theme, entity, event and month keep every value; the scalar keeps the last", () => {
    const p = parseSearch('theme:A proof theme:"B c" entity:X ENTITY:Y event:e1 event:e2 month:2026 month:2026-02 in:one');
    expect(p.themes).toEqual(["A", "B c"]);
    expect(p.entities).toEqual(["X", "Y"]);
    expect(p.events).toEqual(["e1", "e2"]);
    expect(p.months).toEqual(["2026", "2026-02"]);
    expect([p.theme, p.entity, p.event, p.month]).toEqual(["B c", "Y", "e2", "2026-02"]);
    expect(p.in).toBe("one");
    expect(p.positive_terms).toEqual(["proof"]);
  });

  test("every other field keeps the last value and has no list", () => {
    const p = parseSearch("au:a au:b ti:a ti:b tag:a tag:b in:a in:b lang:a lang:b ai:true ai:false y:2001 y:2002");
    expect([p.author, p.title, p.tag, p.in, p.lang, p.ai, p.date]).toEqual(["b", "b", "b", "b", "b", "false", "2002"]);
    expect([p.themes, p.entities, p.events, p.months]).toEqual([[], [], [], []]);
  });

  test("an empty value is kept in the list, and a type shortcut is no list field", () => {
    expect(parseSearch("theme: theme:A").themes).toEqual(["", "A"]);
    expect(parseSearch("tw:theme:A").themes).toEqual([]);
  });
});
