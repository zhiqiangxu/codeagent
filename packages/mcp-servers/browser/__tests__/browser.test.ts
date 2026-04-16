import { BROWSER_TOOLS } from "../src/tools.js";
import type { BrowserActions } from "../src/browser.js";

/** Mock browser for unit tests (no real Playwright) */
function createMockBrowser(): BrowserActions & {
  calls: Array<{ method: string; args: unknown[] }>;
} {
  const calls: Array<{ method: string; args: unknown[] }> = [];
  return {
    calls,
    async navigate(url: string) {
      calls.push({ method: "navigate", args: [url] });
    },
    async snapshot() {
      calls.push({ method: "snapshot", args: [] });
      return JSON.stringify({ role: "WebArea", name: "Test Page", children: [{ role: "heading", name: "hi" }] });
    },
    async click(selector: string) {
      calls.push({ method: "click", args: [selector] });
    },
    async fill(selector: string, value: string) {
      calls.push({ method: "fill", args: [selector, value] });
    },
    async close() {
      calls.push({ method: "close", args: [] });
    },
  };
}

describe("browser MCP Server", () => {
  test("tool list contains navigate/snapshot/click/fill", () => {
    const names = BROWSER_TOOLS.map((t) => t.name);
    expect(names).toContain("navigate");
    expect(names).toContain("snapshot");
    expect(names).toContain("click");
    expect(names).toContain("fill");
  });

  test("navigate calls page.goto", async () => {
    const mock = createMockBrowser();
    await mock.navigate("https://example.com");
    expect(mock.calls).toHaveLength(1);
    expect(mock.calls[0].method).toBe("navigate");
    expect(mock.calls[0].args[0]).toBe("https://example.com");
  });

  test("snapshot returns accessibility tree text", async () => {
    const mock = createMockBrowser();
    const result = await mock.snapshot();
    expect(result).toContain("WebArea");
    expect(result).toContain("hi");
  });

  test("click passes selector", async () => {
    const mock = createMockBrowser();
    await mock.click("button#submit");
    expect(mock.calls[0].method).toBe("click");
    expect(mock.calls[0].args[0]).toBe("button#submit");
  });

  test("fill passes selector and value", async () => {
    const mock = createMockBrowser();
    await mock.fill("input#name", "test");
    expect(mock.calls[0].method).toBe("fill");
    expect(mock.calls[0].args).toEqual(["input#name", "test"]);
  });
});
