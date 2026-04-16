import * as readline from "node:readline";
import type {
  JsonRpcRequest,
  JsonRpcResponse,
  ToolDef,
  ContentPart,
} from "./types.js";
import { executeSearch, RateLimiter } from "./search.js";

const TOOLS: ToolDef[] = [
  {
    name: "web_search",
    description: "Search the web for information",
    inputSchema: {
      type: "object",
      properties: {
        query: { type: "string", description: "Search query" },
      },
      required: ["query"],
    },
  },
];

const rateLimiter = new RateLimiter(10, 60_000); // 10 req/min

function makeResponse(id: number, result: unknown): JsonRpcResponse {
  return { jsonrpc: "2.0", id, result };
}

function makeError(
  id: number,
  code: number,
  message: string,
): JsonRpcResponse {
  return { jsonrpc: "2.0", id, error: { code, message } };
}

async function handleRequest(req: JsonRpcRequest): Promise<JsonRpcResponse> {
  switch (req.method) {
    case "initialize":
      return makeResponse(req.id, {
        capabilities: { tools: true },
      });

    case "tools/list":
      return makeResponse(req.id, { tools: TOOLS });

    case "tools/call": {
      const params = req.params as {
        name: string;
        arguments: Record<string, unknown>;
      };
      if (params.name !== "web_search") {
        return makeError(req.id, -32601, `unknown tool: ${params.name}`);
      }

      if (!rateLimiter.canProceed()) {
        return makeResponse(req.id, {
          content: [
            { type: "text", text: "rate limited, please try again later" },
          ] as ContentPart[],
          isError: true,
        });
      }
      rateLimiter.record();

      try {
        const query = params.arguments.query as string;
        const results = await executeSearch(query);
        const text = results
          .map((r, i) => `${i + 1}. [${r.title}](${r.url})\n   ${r.snippet}`)
          .join("\n\n");

        return makeResponse(req.id, {
          content: [{ type: "text", text: text || "No results found." }],
        });
      } catch (err) {
        return makeResponse(req.id, {
          content: [
            {
              type: "text",
              text: `search API error: ${(err as Error).message}`,
            },
          ],
          isError: true,
        });
      }
    }

    default:
      return makeError(req.id, -32601, `method not found: ${req.method}`);
  }
}

// stdio JSON-RPC server
const rl = readline.createInterface({ input: process.stdin });

rl.on("line", async (line: string) => {
  try {
    const req = JSON.parse(line) as JsonRpcRequest;
    const resp = await handleRequest(req);
    process.stdout.write(JSON.stringify(resp) + "\n");
  } catch {
    // malformed JSON, ignore
  }
});
