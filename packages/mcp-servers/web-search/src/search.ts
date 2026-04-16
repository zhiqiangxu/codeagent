import type { SearchResult } from "./types.js";

const DEFAULT_API_URL = "https://api.search.example.com/v1/search";

/** Rate limiter: max N requests per window */
export class RateLimiter {
  private timestamps: number[] = [];
  constructor(
    private maxRequests: number,
    private windowMs: number,
  ) {}

  canProceed(): boolean {
    const now = Date.now();
    this.timestamps = this.timestamps.filter((t) => now - t < this.windowMs);
    return this.timestamps.length < this.maxRequests;
  }

  record(): void {
    this.timestamps.push(Date.now());
  }
}

/** Format search API request */
export function formatSearchRequest(
  query: string,
  apiUrl = DEFAULT_API_URL,
): { url: string; body: Record<string, unknown> } {
  return {
    url: apiUrl,
    body: { q: query, count: 10, format: "json" },
  };
}

/** Parse search API response */
export function parseSearchResponse(
  data: Record<string, unknown>,
): SearchResult[] {
  const results = (data.results ?? data.items ?? []) as Array<
    Record<string, string>
  >;
  return results.map((r) => ({
    title: r.title ?? "",
    url: r.url ?? r.link ?? "",
    snippet: r.snippet ?? r.description ?? "",
  }));
}

/** Execute web search (real implementation calls external API) */
export async function executeSearch(
  query: string,
  fetchFn: typeof fetch = fetch,
  apiUrl?: string,
): Promise<SearchResult[]> {
  const { url, body } = formatSearchRequest(query, apiUrl);
  const resp = await fetchFn(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body),
  });

  if (!resp.ok) {
    throw new Error(`search API error: ${resp.status} ${resp.statusText}`);
  }

  const data = (await resp.json()) as Record<string, unknown>;
  return parseSearchResponse(data);
}
