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
import { loadSchema } from "./lib/schema-loader.js";
import { generateTools, generateDefaultTools, parseToolName, getCommandByPath } from "./lib/tool-generator.js";
import { validateArguments } from "./lib/validator.js";
import { executeCommand, getTimeoutForCommand } from "./lib/cli-executor.js";
import { ConcurrencyLimiter } from "./lib/concurrency.js";

// Load JIT schema from CLI (single source of truth)
const { schema: jitSchema, warnings: schemaWarnings } = await loadSchema();

// Create concurrency limiter (max 10 concurrent commands)
const concurrencyLimiter = new ConcurrencyLimiter(10);

// Response mode: controls how tool results are returned.
//   "content"    — Summary + JSON in a single text content block (default).
//                   Works with all MCP clients including Claude Code.
//   "structured" — Summary in content, typed data in structuredContent with
//                   outputSchema. Requires client support (MCP 2025-06-18+).
//                   As of Feb 2026 most clients ignore content when
//                   structuredContent is present, so the summary is lost.
const RESPONSE_MODE = process.env.JIT_MCP_RESPONSE_MODE || "content";

// Server instructions for AI agents
const SERVER_INSTRUCTIONS = `JIT (Just-In-Time) is a repository-local issue tracker designed for AI agents.

Key concepts:
- Issues have states: backlog → ready → in_progress → done (or rejected)
- Dependencies are CRITICAL: use jit_dep_add to express "B needs A done first"
- Gates are quality checkpoints (tests, clippy, fmt, code-review) that must pass
- Labels organize issues: type:task/story/epic, epic:*, milestone:*, component:*
- Claims/leases prevent concurrent edits across multiple agents

Label hierarchy:
- type:epic → contains stories, has epic:* label for grouping
- type:story → contains tasks, has story:* label linking to parent
- type:task → leaf work items, should have epic:* or story:* label
- type:bug → defects, should link to epic:* for tracking

Common workflows:
1. Find work: jit_query_available (unassigned ready issues)
2. Claim issue: jit_issue_claim or jit_claim_acquire  
3. Check details: jit_issue_show (includes labels, gates, documents)
4. Check dependencies: jit_graph_deps (what blocks this issue)
5. Inspect gate runs: jit_gate_check-all
6. Complete: jit_issue_update with state=done

Tips:
- Short IDs work: "92bf3a9b" instead of full UUID
- Dependencies matter more than labels for workflow
- jit_recover cleans up stale locks and corrupted state
- jit_validate checks repository consistency
- Always check gates before marking done: jit_gate_check-all`;

/**
 * Generate a human-readable summary for tool results.
 * Uses the `message` field from CLI JSON output (single source of truth).
 * @param {string} toolName - The MCP tool name
 * @param {object} result - The result data
 * @returns {string} Human-readable summary
 */
function generateUserSummary(toolName, result) {
  if (result && result.message) {
    return result.message;
  }
  if (typeof result === 'string') {
    return result.length > 200 ? result.substring(0, 197) + '...' : result;
  }
  return `Command ${toolName.replace(/^jit_/, 'jit ').replace(/_/g, ' ')} completed`;
}

/**
 * Compact result data for assistant consumption.
 * Generic rules only — no command-specific branches.
 * @param {string} _toolName - The MCP tool name (unused, kept for API compat)
 * @param {object} result - The full result data
 * @returns {object} Compacted result
 */
function compactForAssistant(_toolName, result) {
  const MAX_ARRAY_ITEMS = 20;
  const MAX_STRING_CHARS = 500;

  // Top-level array: truncate if too long
  if (Array.isArray(result)) {
    if (result.length > MAX_ARRAY_ITEMS) {
      return {
        count: result.length,
        items: result.slice(0, MAX_ARRAY_ITEMS),
        truncated: true,
      };
    }
    return result;
  }

  if (result && typeof result === 'object') {
    const compacted = { ...result };

    // Truncate known large string fields (stdout/stderr from gate checks)
    for (const key of ['stdout', 'stderr']) {
      if (typeof compacted[key] === 'string' && compacted[key].length > MAX_STRING_CHARS) {
        compacted[key] = compacted[key].substring(0, MAX_STRING_CHARS) + '... (truncated)';
      }
    }

    // Truncate known large array fields
    for (const key of ['issues', 'results', 'dependencies', 'dependents', 'roots']) {
      if (Array.isArray(compacted[key]) && compacted[key].length > MAX_ARRAY_ITEMS) {
        compacted[key] = compacted[key].slice(0, MAX_ARRAY_ITEMS);
        compacted.truncated = true;
      }
    }

    return compacted;
  }

  return result;
}

/**
 * Format a successful tool result based on the configured response mode.
 * @param {string} summary - Human-readable summary
 * @param {object} data - Compacted result data
 * @returns {object} MCP CallToolResult
 */
function formatSuccessResult(summary, data) {
  if (RESPONSE_MODE === "structured") {
    return {
      content: [
        { type: "text", text: summary },
      ],
      structuredContent: data,
    };
  }

  // Default "content" mode: summary + JSON in a single text block
  return {
    content: [
      {
        type: "text",
        text: summary + "\n" + JSON.stringify(data, null, 2),
      },
    ],
  };
}

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
    instructions: SERVER_INSTRUCTIONS,
  }
);

// Cache generated tools
let cachedAllTools = null;
let cachedListedTools = null;

/**
 * Get or generate all tools (used for command lookup and execution)
 */
function getAllTools() {
  if (!cachedAllTools) {
    const includeOutputSchema = RESPONSE_MODE === "structured";
    cachedAllTools = generateTools(jitSchema, includeOutputSchema);
  }
  return cachedAllTools;
}

/**
 * Get tools to advertise in tools/list.
 * Returns all tools if JIT_MCP_ALL_TOOLS is set, otherwise the curated default set.
 */
function getListedTools() {
  if (!cachedListedTools) {
    if (process.env.JIT_MCP_ALL_TOOLS) {
      cachedListedTools = getAllTools();
    } else {
      const includeOutputSchema = RESPONSE_MODE === "structured";
      cachedListedTools = generateDefaultTools(jitSchema, includeOutputSchema);
    }
  }
  return cachedListedTools;
}

// Register tool handlers
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: getListedTools(),
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
  
  // Find the tool definition for validation (search all tools, not just listed ones)
  const tools = getAllTools();
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
      const timeout = getTimeoutForCommand(cmdPath);
      return await executeCommand(cmdPath, validation.data, cmdDef, timeout);
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
    
    // Generate user-friendly summary and compact data for assistant
    const userSummary = generateUserSummary(name, result.data);
    const compactData = compactForAssistant(name, result.data);

    return formatSuccessResult(userSummary, compactData);
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
  console.error(`Tools: ${getListedTools().length} listed (${getAllTools().length} total)`);
  
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
