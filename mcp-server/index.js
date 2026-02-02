#!/usr/bin/env node

/**
 * JIT MCP Server
 * 
 * Model Context Protocol server for the Just-In-Time issue tracker.
 * Wraps the jit CLI to provide MCP tools for AI agents like Claude.
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { join, dirname } from "path";
import { fileURLToPath } from "url";
import { loadSchema } from "./lib/schema-loader.js";
import { generateTools, parseToolName, getCommandByPath } from "./lib/tool-generator.js";
import { validateArguments } from "./lib/validator.js";
import { executeCommand } from "./lib/cli-executor.js";
import { ConcurrencyLimiter } from "./lib/concurrency.js";

const __dirname = dirname(fileURLToPath(import.meta.url));

// Load JIT schema with runtime preference
const schemaPath = join(__dirname, "jit-schema.json");
const { schema: jitSchema, warnings: schemaWarnings } = await loadSchema(schemaPath);

// Create concurrency limiter (max 10 concurrent commands)
const concurrencyLimiter = new ConcurrencyLimiter(10);

// Create MCP server
const server = new Server(
  {
    name: "jit-mcp-server",
    version: jitSchema.version,
  },
  {
    capabilities: {
      tools: {},
    },
  }
);

// Cache generated tools
let cachedTools = null;

/**
 * Get or generate tools
 */
function getTools() {
  if (!cachedTools) {
    cachedTools = generateTools(jitSchema);
  }
  return cachedTools;
}

// Register tool handlers
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: getTools(),
  };
});

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;
  
  // Parse tool name to command path
  const cmdPath = parseToolName(name);
  
  // Get command definition from schema
  const cmdDef = getCommandByPath(jitSchema, cmdPath);
  
  if (!cmdDef) {
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify({
            success: false,
            error: {
              code: "UNKNOWN_TOOL",
              message: `Tool '${name}' not found in schema`,
            },
          }, null, 2),
        },
      ],
      isError: true,
    };
  }
  
  // Find the tool definition for validation
  const tools = getTools();
  const tool = tools.find(t => t.name === name);
  
  if (!tool) {
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify({
            success: false,
            error: {
              code: "UNKNOWN_TOOL",
              message: `Tool '${name}' not found`,
            },
          }, null, 2),
        },
      ],
      isError: true,
    };
  }
  
  // Validate arguments
  const validation = validateArguments(args || {}, tool.inputSchema);
  
  if (!validation.success) {
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify({
            success: false,
            error: {
              code: "VALIDATION_ERROR",
              message: validation.error,
            },
          }, null, 2),
        },
      ],
      isError: true,
    };
  }
  
  try {
    // Execute command with concurrency limiting
    const result = await concurrencyLimiter.run(async () => {
      return await executeCommand(cmdPath, validation.data, cmdDef);
    });
    
    // Check if result is an error
    if (!result.success) {
      return {
        content: [
          {
            type: "text",
            text: JSON.stringify(result, null, 2),
          },
        ],
        isError: true,
      };
    }
    
    // Return success
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(result.data, null, 2),
        },
      ],
    };
  } catch (error) {
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify({
            success: false,
            error: {
              code: "UNEXPECTED_ERROR",
              message: error.message,
            },
          }, null, 2),
        },
      ],
      isError: true,
    };
  }
});

// Start server
async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
  
  console.error("JIT MCP Server running on stdio");
  console.error(`Version: ${jitSchema.version}`);
  console.error(`Tools: ${getTools().length}`);
  
  // Display schema warnings
  if (schemaWarnings.length > 0) {
    console.error("\n⚠️  Schema Warnings:");
    for (const warning of schemaWarnings) {
      console.error(`  - ${warning}`);
    }
    console.error("");
  }
}

main().catch((error) => {
  console.error("Fatal error:", error);
  process.exit(1);
});
