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
