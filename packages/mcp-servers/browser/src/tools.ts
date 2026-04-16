/** Browser tool definitions for MCP */

export interface ToolDef {
  name: string;
  description: string;
  inputSchema: Record<string, unknown>;
}

export const BROWSER_TOOLS: ToolDef[] = [
  {
    name: "navigate",
    description: "Navigate to a URL",
    inputSchema: {
      type: "object",
      properties: {
        url: { type: "string", description: "URL to navigate to" },
      },
      required: ["url"],
    },
  },
  {
    name: "snapshot",
    description: "Get the accessibility tree of the current page",
    inputSchema: {
      type: "object",
      properties: {},
    },
  },
  {
    name: "click",
    description: "Click an element on the page",
    inputSchema: {
      type: "object",
      properties: {
        selector: {
          type: "string",
          description: "CSS selector of the element to click",
        },
      },
      required: ["selector"],
    },
  },
  {
    name: "fill",
    description: "Fill a form field",
    inputSchema: {
      type: "object",
      properties: {
        selector: {
          type: "string",
          description: "CSS selector of the input element",
        },
        value: { type: "string", description: "Value to fill" },
      },
      required: ["selector", "value"],
    },
  },
];
