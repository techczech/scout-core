// Shared grammar fixtures: `fixtures/grammar-cases.json` holds input -> parsed
// query pairs that BOTH this package and the Rust port (`crates/scout-query`)
// must reproduce exactly.
//
// Regenerate the expected values from this implementation after a deliberate
// grammar change (inputs are kept; add a case by adding its input):
//   SCOUT_WRITE_FIXTURES=1 npx vitest run packages/scout-query/grammar.test.ts
import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { describe, expect, test } from "vitest";
import { dateRange, parseSearch } from "./src/query";

interface ParseCase {
  input: string;
  partial: boolean;
  expected?: unknown;
}
interface DateCase {
  input: string;
  expected?: unknown;
}
interface Fixtures {
  about: string;
  parse: ParseCase[];
  date_range: DateCase[];
}

const path = fileURLToPath(new URL("./fixtures/grammar-cases.json", import.meta.url));
const fixtures: Fixtures = JSON.parse(readFileSync(path, "utf8"));

if (process.env.SCOUT_WRITE_FIXTURES) {
  for (const c of fixtures.parse) c.expected = parseSearch(c.input, c.partial);
  for (const c of fixtures.date_range) c.expected = dateRange(c.input);
  writeFileSync(path, JSON.stringify(fixtures, null, 2) + "\n");
}

describe("shared grammar fixtures", () => {
  test("the fixture file has cases", () => {
    expect(fixtures.parse.length).toBeGreaterThan(40);
    expect(fixtures.date_range.length).toBeGreaterThan(5);
  });

  for (const c of fixtures.parse) {
    test(`parseSearch(${JSON.stringify(c.input)}, partial=${c.partial})`, () => {
      expect(c.expected, "run with SCOUT_WRITE_FIXTURES=1 to fill").toBeDefined();
      expect(parseSearch(c.input, c.partial)).toEqual(c.expected);
    });
  }

  for (const c of fixtures.date_range) {
    test(`dateRange(${JSON.stringify(c.input)})`, () => {
      expect(dateRange(c.input)).toEqual(c.expected);
    });
  }
});
