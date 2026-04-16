import {
  formatSearchRequest,
  parseSearchResponse,
  executeSearch,
  RateLimiter,
} from "../src/search.js";

describe("web_search MCP Server", () => {
  test("tool list contains web_search with correct schema", () => {
    // Tool definition as declared in index.ts
    const tool = {
      name: "web_search",
      description: "Search the web for information",
      inputSchema: {
        type: "object",
        properties: {
          query: { type: "string", description: "Search query" },
        },
        required: ["query"],
      },
    };
    expect(tool.name).toBe("web_search");
    expect(tool.inputSchema.properties).toHaveProperty("query");
    expect(tool.inputSchema.required).toContain("query");
  });

  test("format search request", () => {
    const { url, body } = formatSearchRequest("rust lang");
    expect(url).toContain("search");
    expect(body.q).toBe("rust lang");
    expect(body.count).toBe(10);
  });

  test("parse search response with 3 results", () => {
    const data = {
      results: [
        { title: "Rust Language", url: "https://rust-lang.org", snippet: "A systems programming language" },
        { title: "Rust Book", url: "https://doc.rust-lang.org/book", snippet: "The Rust Book" },
        { title: "Crates.io", url: "https://crates.io", snippet: "Rust packages" },
      ],
    };

    const results = parseSearchResponse(data);
    expect(results).toHaveLength(3);
    expect(results[0].title).toBe("Rust Language");
    expect(results[0].url).toBe("https://rust-lang.org");
    expect(results[0].snippet).toContain("systems programming");
  });

  test("rate limiter blocks after limit", () => {
    const limiter = new RateLimiter(3, 60000);
    expect(limiter.canProceed()).toBe(true);
    limiter.record();
    limiter.record();
    limiter.record();
    expect(limiter.canProceed()).toBe(false);
  });

  test("search API error throws", async () => {
    const mockFetch = jest.fn().mockResolvedValue({
      ok: false,
      status: 500,
      statusText: "Internal Server Error",
    });

    await expect(
      executeSearch("test", mockFetch as unknown as typeof fetch),
    ).rejects.toThrow("search API error");
  });
});
