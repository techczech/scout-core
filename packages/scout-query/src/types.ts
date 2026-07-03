export interface RegexFilter {
  source: string;
  flags: string;
}

export type SearchMode = "keyword" | "semantic";
export type SortMode = "matches" | "recent" | "oldest";
