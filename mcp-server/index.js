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
5. Run gates: jit_gate_check-all
6. Complete: jit_issue_update with state=done

Tips:
- Short IDs work: "92bf3a9b" instead of full UUID
- Dependencies matter more than labels for workflow
- jit_recover cleans up stale locks and corrupted state
- jit_validate checks repository consistency
- Always check gates before marking done: jit_gate_check-all`;

/**
 * Generate a human-readable summary for tool results
 * @param {string} toolName - The MCP tool name
 * @param {object} result - The result data
 * @returns {string} Human-readable summary
 */
function generateUserSummary(toolName, result) {
  // Extract command parts from tool name
  const parts = toolName.replace('jit_', '').split('_');
  const command = parts[0];
  const subcommand = parts.slice(1).join('-');
  
  // Handle different command types
  if (command === 'status') {
    return `Status: ${result.open || 0} open, ${result.ready || 0} ready, ${result.in_progress || 0} in progress, ${result.done || 0} done, ${result.blocked || 0} blocked`;
  }
  
  if (command === 'issue') {
    if (subcommand === 'create') {
      return `Created issue: ${result.id?.substring(0, 8)} - ${result.title}`;
    }
    if (subcommand === 'show') {
      const gates = result.gates_required?.length || 0;
      const gatesPassed = Object.values(result.gates_status || {}).filter(g => g.status === 'passed').length;
      return `Issue ${result.id?.substring(0, 8)}: ${result.title} [${result.state}${gates > 0 ? `, ${gatesPassed}/${gates} gates` : ''}]`;
    }
    if (subcommand === 'update') {
      return `Updated issue ${result.id?.substring(0, 8)}: ${result.title} → ${result.state}`;
    }
    if (subcommand === 'search') {
      const count = result.length || result.count || 0;
      return `Found ${count} issue${count !== 1 ? 's' : ''} matching query`;
    }
    if (subcommand === 'claim') {
      return `Claimed issue ${result.id?.substring(0, 8) || result.issue_id?.substring(0, 8)}`;
    }
    if (subcommand === 'claim-next') {
      if (result.id) {
        return `Claimed next issue: ${result.id.substring(0, 8)} - ${result.title}`;
      }
      return `No issues available to claim`;
    }
    if (subcommand === 'delete') {
      return `Deleted issue ${result.id?.substring(0, 8)}`;
    }
  }
  
  if (command === 'query') {
    const count = result.count || result.issues?.length || 0;
    const queryType = subcommand || 'issues';
    return `Found ${count} ${queryType === 'all' ? 'issue' : queryType}${count !== 1 ? 's' : ''}`;
  }
  
  if (command === 'gate') {
    if (subcommand === 'pass' || subcommand === 'fail') {
      return `Gate ${result.gate_key || 'unknown'} ${subcommand}ed for issue ${result.issue_id?.substring(0, 8)}`;
    }
    if (subcommand === 'check') {
      return `Gate ${result.gate_key}: ${result.status || (result.passed ? 'passed' : 'failed')}`;
    }
    if (subcommand === 'check-all') {
      if (Array.isArray(result)) {
        const passed = result.filter(g => g.status === 'passed').length;
        const failed = result.filter(g => g.status === 'failed').length;
        const total = result.length;
        if (failed > 0) {
          return `Gates: ${passed}/${total} passed, ${failed} failed`;
        }
        return `All ${total} gates passed`;
      }
      return `Gate check completed`;
    }
    if (subcommand === 'list' || subcommand === 'define' || subcommand === 'show') {
      if (result.gates) {
        return `${result.gates.length} gate definition(s)`;
      }
      if (result.key) {
        return `Gate ${result.key}: ${result.title}`;
      }
    }
    if (subcommand === 'add') {
      return `Added gate(s) to issue ${result.issue_id?.substring(0, 8) || 'unknown'}`;
    }
    if (subcommand === 'remove') {
      return `Removed gate from issue`;
    }
  }
  
  if (command === 'dep') {
    if (subcommand === 'add') {
      if (result.added_count) {
        return `Added ${result.added_count} dependenc${result.added_count !== 1 ? 'ies' : 'y'}`;
      }
      return `Added dependency: ${result.from_id?.substring(0, 8)} → ${result.to_id?.substring(0, 8)}`;
    }
    if (subcommand === 'rm') {
      return `Removed dependency: ${result.from_id?.substring(0, 8)} ↛ ${result.to_id?.substring(0, 8)}`;
    }
  }
  
  if (command === 'claim') {
    if (subcommand === 'acquire') {
      return `Acquired lease ${result.lease_id?.substring(0, 8)} on issue ${result.issue_id?.substring(0, 8)}`;
    }
    if (subcommand === 'release') {
      return `Released lease ${result.lease_id?.substring(0, 8)}`;
    }
    if (subcommand === 'renew') {
      return `Renewed lease ${result.lease_id?.substring(0, 8)}`;
    }
    if (subcommand === 'heartbeat') {
      return `Heartbeat sent for lease ${result.lease_id?.substring(0, 8)}`;
    }
    if (subcommand === 'status') {
      const count = result.leases?.length || 0;
      return `${count} active lease${count !== 1 ? 's' : ''}`;
    }
    if (subcommand === 'list') {
      const count = Array.isArray(result) ? result.length : result.leases?.length || 0;
      return `${count} lease${count !== 1 ? 's' : ''} found`;
    }
    if (subcommand === 'force-evict') {
      return `Force-evicted lease`;
    }
  }
  
  if (command === 'doc') {
    if (subcommand === 'list') {
      const count = result.count || result.documents?.length || 0;
      return `${count} document${count !== 1 ? 's' : ''} attached`;
    }
    if (subcommand === 'add') {
      return `Added document reference`;
    }
    if (subcommand === 'remove') {
      return `Removed document reference`;
    }
    if (subcommand === 'show') {
      return `Document content retrieved`;
    }
    if (subcommand === 'check-links') {
      const broken = result.broken_links?.length || 0;
      if (broken === 0) {
        return `✓ All links valid`;
      }
      return `Found ${broken} broken link${broken !== 1 ? 's' : ''}`;
    }
  }
  
  if (command === 'graph') {
    if (subcommand === 'deps') {
      const count = result.dependencies?.length || result.count || 0;
      return `${count} dependenc${count !== 1 ? 'ies' : 'y'}`;
    }
    if (subcommand === 'downstream') {
      const count = result.dependents?.length || result.count || 0;
      return `${count} dependent${count !== 1 ? 's' : ''}`;
    }
    if (subcommand === 'roots') {
      const count = Array.isArray(result) ? result.length : result.roots?.length || 0;
      return `${count} root issue${count !== 1 ? 's' : ''}`;
    }
  }
  
  if (command === 'validate') {
    if (result.valid === true) {
      return `✓ Validation passed`;
    } else if (result.valid === false) {
      const validations = result.validations || [];
      const failed = validations.filter(v => !v.valid);
      return `✗ Validation failed (${failed.length} issue${failed.length !== 1 ? 's' : ''})`;
    }
  }
  
  if (command === 'label') {
    if (subcommand === 'namespaces') {
      const count = Array.isArray(result) ? result.length : result.namespaces?.length || 0;
      return `${count} label namespace${count !== 1 ? 's' : ''}`;
    }
    if (subcommand === 'values') {
      const count = Array.isArray(result) ? result.length : result.values?.length || 0;
      return `${count} value${count !== 1 ? 's' : ''} in namespace`;
    }
  }
  
  if (command === 'worktree') {
    if (subcommand === 'info') {
      return `Worktree: ${result.worktree_id || result.id || 'unknown'}`;
    }
    if (subcommand === 'list') {
      const count = Array.isArray(result) ? result.length : result.worktrees?.length || 0;
      return `${count} worktree${count !== 1 ? 's' : ''}`;
    }
  }
  
  if (command === 'recover') {
    return `Recovery: ${result.stale_locks_cleaned || 0} locks cleaned, ${result.expired_leases_evicted || 0} leases evicted`;
  }
  
  if (command === 'search') {
    const count = result.results?.length || result.count || (Array.isArray(result) ? result.length : 0);
    return `Found ${count} result${count !== 1 ? 's' : ''}`;
  }
  
  // Default: try to be smart about the result
  if (result.message) {
    return result.message;
  }
  
  if (typeof result === 'string') {
    return result.length > 100 ? result.substring(0, 97) + '...' : result;
  }
  
  // Generic fallback
  return `Command ${toolName.replace('jit_', 'jit ').replace(/_/g, ' ')} completed`;
}

/**
 * Compact result data for assistant consumption
 * Reduces context usage by trimming verbose fields and limiting arrays
 * @param {string} toolName - The MCP tool name
 * @param {object} result - The full result data
 * @returns {object} Compacted result
 */
function compactForAssistant(toolName, result) {
  const MAX_ARRAY_ITEMS = 20;
  
  // Extract command parts
  const parts = toolName.replace('jit_', '').split('_');
  const command = parts[0];
  const subcommand = parts.slice(1).join('-');
  
  // For gate check-all, return summary only (omit huge stdout/stderr)
  if (command === 'gate' && subcommand === 'check-all') {
    if (Array.isArray(result)) {
      return result.map(g => ({
        gate_key: g.gate_key,
        status: g.status,
        duration_ms: g.duration_ms,
      }));
    }
  }
  
  // CLI now returns compact results by default for query and search commands
  // We just need to truncate large arrays
  if (Array.isArray(result) && result.length > MAX_ARRAY_ITEMS) {
    return {
      count: result.length,
      items: result.slice(0, MAX_ARRAY_ITEMS),
      truncated: true,
    };
  }
  
  // For results with .issues array, truncate if needed
  if (result.issues && Array.isArray(result.issues) && result.issues.length > MAX_ARRAY_ITEMS) {
    return {
      ...result,
      issues: result.issues.slice(0, MAX_ARRAY_ITEMS),
      truncated: true,
    };
  }
  
  // Default: pass through as-is
  return result;
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
    
    // Generate user-friendly summary and compact data for assistant
    const userSummary = generateUserSummary(name, result.data);
    const compactData = compactForAssistant(name, result.data);
    
    // Return success with dual-audience content
    return {
      content: [
        {
          type: "text",
          text: userSummary,
          annotations: {
            audience: ["user"],
          },
        },
        {
          type: "text",
          text: JSON.stringify(compactData, null, 2),
          annotations: {
            audience: ["assistant"],
          },
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
