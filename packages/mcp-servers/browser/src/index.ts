import * as readline from "node:readline";
import { BROWSER_TOOLS } from "./tools.js";
import type { BrowserActions } from "./browser.js";
import { createPlaywrightBrowser } from "./browser.js";

interface JsonRpcRequest {
  jsonrpc: "2.0";
  id: number;
  method: string;
  params?: Record<string, unknown>;
}

interface JsonRpcResponse {
  jsonrpc: "2.0";
  id: number;
  result?: unknown;
  error?: { code: number; message: string };
}

let browser: BrowserActions | null = null;

async function getBrowser(): Promise<BrowserActions> {
  if (!browser) {
    browser = await createPlaywrightBrowser();
  }
  return browser;
}

async function handleRequest(req: JsonRpcRequest): Promise<JsonRpcResponse> {
  switch (req.method) {
    case "initialize":
      return { jsonrpc: "2.0", id: req.id, result: { capabilities: { tools: true } } };

    case "tools/list":
      return { jsonrpc: "2.0", id: req.id, result: { tools: BROWSER_TOOLS } };

    case "tools/call": {
      const params = req.params as { name: string; arguments: Record<string, string> };
      const b = await getBrowser();

      try {
        let text = "";
        switch (params.name) {
          case "navigate":
            await b.navigate(params.arguments.url);
            text = `Navigated to ${params.arguments.url}`;
            break;
          case "snapshot":
            text = await b.snapshot();
            break;
          case "click":
            await b.click(params.arguments.selector);
            text = `Clicked ${params.arguments.selector}`;
            break;
          case "fill":
            await b.fill(params.arguments.selector, params.arguments.value);
            text = `Filled ${params.arguments.selector} with "${params.arguments.value}"`;
            break;
          default:
            return {
              jsonrpc: "2.0",
              id: req.id,
              error: { code: -32601, message: `unknown tool: ${params.name}` },
            };
        }
        return {
          jsonrpc: "2.0",
          id: req.id,
          result: { content: [{ type: "text", text }] },
        };
      } catch (err) {
        return {
          jsonrpc: "2.0",
          id: req.id,
          result: {
            content: [{ type: "text", text: (err as Error).message }],
            isError: true,
          },
        };
      }
    }

    default:
      return {
        jsonrpc: "2.0",
        id: req.id,
        error: { code: -32601, message: `method not found: ${req.method}` },
      };
  }
}

const rl = readline.createInterface({ input: process.stdin });
rl.on("line", async (line: string) => {
  try {
    const req = JSON.parse(line) as JsonRpcRequest;
    const resp = await handleRequest(req);
    process.stdout.write(JSON.stringify(resp) + "\n");
  } catch {
    // malformed JSON
  }
});

process.on("exit", () => {
  browser?.close();
});
