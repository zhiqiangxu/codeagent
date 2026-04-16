/** Browser abstraction over Playwright for testability */

export interface BrowserActions {
  navigate(url: string): Promise<void>;
  snapshot(): Promise<string>;
  click(selector: string): Promise<void>;
  fill(selector: string, value: string): Promise<void>;
  close(): Promise<void>;
}

/** Real Playwright implementation */
export async function createPlaywrightBrowser(): Promise<BrowserActions> {
  const { chromium } = await import("playwright");
  const browser = await chromium.launch({ headless: true });
  const context = await browser.newContext();
  const page = await context.newPage();

  return {
    async navigate(url: string) {
      await page.goto(url);
    },
    async snapshot(): Promise<string> {
      return await page.accessibility.snapshot().then(
        (tree) => JSON.stringify(tree, null, 2),
        () => "accessibility tree unavailable",
      );
    },
    async click(selector: string) {
      await page.click(selector);
    },
    async fill(selector: string, value: string) {
      await page.fill(selector, value);
    },
    async close() {
      await browser.close();
    },
  };
}
