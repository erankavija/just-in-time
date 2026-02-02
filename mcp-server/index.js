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
import { execFile } from "child_process";
import { promisify } from "util";
import { readFileSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const execFileAsync = promisify(execFile);
const __dirname = dirname(fileURLToPath(import.meta.url));

// Load JIT schema
const schemaPath = join(__dirname, "jit-schema.json");
const jitSchema = JSON.parse(readFileSync(schemaPath, "utf-8"));

/**
 * Execute jit CLI command and return parsed JSON output
 * @param {string[]} cmdArgs - Array of command arguments (not including 'jit')
 * @param {boolean} useJsonFlag - Whether to add --json flag
 */
async function runJitCommand(cmdArgs, useJsonFlag = true) {
  const args = [...cmdArgs];
  if (useJsonFlag) {
    args.push('--json');
  }
  
  // Check if command will output JSON
  const expectsJson = useJsonFlag || args.includes('--json');
  
  try {
    const { stdout, stderr } = await execFileAsync('jit', args, {
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    });
    
    if (stderr && !stdout) {
      throw new Error(stderr);
    }
    
    // Commands without --json flag return plain text
    if (!expectsJson) {
      return { message: stdout.trim() };
    }
    
    // JIT CLI returns JSON - may be wrapped or unwrapped
    const result = JSON.parse(stdout);
    
    // If response has success/data wrapper, unwrap it
    if (result.success === true && result.data !== undefined) {
      return result.data;
    }
    
    // If response has error, throw it
    if (result.success === false && result.error) {
      throw new Error(`${result.error.code || 'ERROR'}: ${result.error.message}`);
    }
    
    // Otherwise return as-is
    return result;
  } catch (error) {
    if (error.stdout) {
      try {
        const result = JSON.parse(error.stdout);
        if (!result.success && result.error) {
          const errMsg = `${result.error.code}: ${result.error.message}`;
          const suggestions = result.error.suggestions;
          if (suggestions && suggestions.length > 0) {
            throw new Error(`${errMsg}\nSuggestions: ${suggestions.join(', ')}`);
          }
          throw new Error(errMsg);
        }
      } catch (parseError) {
        // If it's our error, rethrow it; otherwise continue with original
        if (parseError.message.includes(':')) {
          throw parseError;
        }
      }
    }
    throw error;
  }
}

/**
 * Generate MCP tool definition from JIT schema command
 */
function generateToolFromCommand(name, cmd, parentPath = "") {
  const toolName = parentPath ? `jit_${parentPath}_${name}` : `jit_${name}`;
  
  const properties = {};
  const required = [];
  
  // Add arguments as properties
  if (cmd.args) {
    for (const arg of cmd.args) {
      // Handle array types: convert array<string> or array[string] to proper JSON Schema
      const isArray = arg.type === "array<string>" || arg.type === "array[string]" || arg.type === "array";
      
      properties[arg.name] = {
        type: isArray ? "array" : arg.type,
        description: arg.description || `${arg.name} parameter`,
      };
      
      if (isArray) {
        properties[arg.name].items = { type: "string" };
      }
      
      if (arg.default !== undefined) {
        properties[arg.name].default = arg.default;
      }
      
      if (arg.required) {
        required.push(arg.name);
      }
    }
  }
  
  // Add flags as properties
  if (cmd.flags) {
    for (const flag of cmd.flags) {
      // Handle array types: convert array<string> or array[string] to proper JSON Schema
      const isArray = flag.type === "array<string>" || flag.type === "array[string]" || flag.type === "array";
      
      properties[flag.name] = {
        type: isArray ? "array" : flag.type,
        description: flag.description || `${flag.name} flag`,
      };
      
      if (isArray) {
        properties[flag.name].items = { type: "string" };
      }
      
      if (flag.required) {
        required.push(flag.name);
      }
    }
  }
  
  const tool = {
    name: toolName,
    description: cmd.description,
    inputSchema: {
      type: "object",
      properties,
      required,
    },
  };
  
  // Add output schema if available
  if (cmd.output?.success_schema) {
    tool.outputSchema = cmd.output.success_schema;
  }
  
  return tool;
}

/**
 * Generate all MCP tools from JIT schema
 */
function generateTools() {
  const tools = [];
  
  for (const [cmdName, cmd] of Object.entries(jitSchema.commands)) {
    if (cmd.subcommands) {
      // Handle subcommands (issue, dep, gate, etc.)
      for (const [subName, subCmd] of Object.entries(cmd.subcommands)) {
        tools.push(generateToolFromCommand(subName, subCmd, cmdName));
      }
    } else {
      // Handle top-level commands (init, status, validate)
      tools.push(generateToolFromCommand(cmdName, cmd));
    }
  }
  
  return tools;
}

/**
 * Check if command supports --json flag by examining schema
 */
function supportsJsonFlag(cmdName, subCmdName = null) {
  const cmd = jitSchema.commands[cmdName];
  if (!cmd) return false;
  
  let targetCmd = cmd;
  if (subCmdName && cmd.subcommands) {
    targetCmd = cmd.subcommands[subCmdName];
    if (!targetCmd) return false;
  }
  
  // Check if flags array contains json flag
  return targetCmd.flags?.some(flag => flag.name === "json") || false;
}

/**
 * Commands that use positional arguments (not flags)
 * Format: "cmd_subcmd": ["arg1", "arg2"]
 */
/**
 * Get positional argument names for a command from the schema
 */
function getPositionalArgs(cmd, subcmd) {
  let cmdDef;
  
  if (subcmd) {
    cmdDef = jitSchema.commands[cmd]?.subcommands?.[subcmd];
  } else {
    cmdDef = jitSchema.commands[cmd];
  }
  
  if (!cmdDef || !cmdDef.args) {
    return [];
  }
  
  // Return names of all positional args in order
  return cmdDef.args.map(arg => arg.name);
}

/**
 * Execute MCP tool by mapping to jit CLI command
 */
async function executeTool(name, args) {
  // Parse tool name: jit_<command>_<subcommand> or jit_<command>
  const parts = name.split("_");
  parts.shift(); // Remove 'jit' prefix
  
  const cliArgs = [];
  let hasJsonFlag;
  
  if (parts.length >= 2) {
    // Subcommand: jit_issue_create -> jit issue create
    // Handle multi-word subcommands: jit_label_add_namespace -> jit label add-namespace
    const cmd = parts[0];
    const subcmdParts = parts.slice(1);
    const subcmd = subcmdParts.join("-"); // Convert underscores back to hyphens
    cliArgs.push(cmd, subcmd);
    hasJsonFlag = supportsJsonFlag(cmd, subcmd);
    
    // Get positional arguments from schema (use hyphens for lookup)
    const positionalArgNames = getPositionalArgs(cmd, subcmd);
    
    if (positionalArgNames) {
      // Add positional arguments
      for (const argName of positionalArgNames) {
        const value = args[argName];
        if (value !== undefined && value !== "") {
          cliArgs.push(value);
        }
      }
      
      // Add any remaining arguments as flags
      const positionalSet = new Set(positionalArgNames);
      for (const [key, value] of Object.entries(args)) {
        if (positionalSet.has(key)) continue;
        
        if (Array.isArray(value)) {
          for (const item of value) {
            cliArgs.push(`--${key}`, item);
          }
        } else if (typeof value === 'boolean') {
          // Boolean flags: only add if true, without value
          if (value) {
            cliArgs.push(`--${key}`);
          }
        } else if (value !== undefined && value !== "") {
          cliArgs.push(`--${key}`, value);
        }
      }
    } else {
      // All arguments are flags
      for (const [key, value] of Object.entries(args)) {
        if (Array.isArray(value)) {
          for (const item of value) {
            cliArgs.push(`--${key}`, item);
          }
        } else if (typeof value === 'boolean') {
          // Boolean flags: only add if true, without value
          if (value) {
            cliArgs.push(`--${key}`);
          }
        } else if (value !== undefined && value !== "") {
          cliArgs.push(`--${key}`, value);
        }
      }
    }
  } else {
    // Top-level command: jit_status -> jit status  or jit_search -> jit search
    const cmd = parts[0];
    cliArgs.push(cmd);
    
    // Get positional arguments from schema
    const positionalArgNames = getPositionalArgs(cmd, null);
    
    if (positionalArgNames) {
      // Add positional arguments
      for (const argName of positionalArgNames) {
        const value = args[argName];
        if (value !== undefined && value !== "") {
          cliArgs.push(value);
        }
      }
      
      // Add any remaining arguments as flags
      const positionalSet = new Set(positionalArgNames);
      for (const [key, value] of Object.entries(args)) {
        if (positionalSet.has(key)) continue;
        
        if (Array.isArray(value)) {
          for (const item of value) {
            cliArgs.push(`--${key}`, item);
          }
        } else if (typeof value === 'boolean') {
          // Boolean flags: only add if true, without value
          if (value) {
            cliArgs.push(`--${key}`);
          }
        } else if (value !== undefined && value !== "") {
          cliArgs.push(`--${key}`, value);
        }
      }
    }
    
    // Check if json flag was already added by user
    if (args.json === true) {
      hasJsonFlag = false; // Don't add --json again in runJitCommand
    } else {
      hasJsonFlag = supportsJsonFlag(cmd);
    }
  }
  
  // Override: if --json is already in cliArgs, don't add it again
  if (cliArgs.includes('--json')) {
    hasJsonFlag = false;
  }
  
  return await runJitCommand(cliArgs, hasJsonFlag);
}

/**
 * Generate a human-readable summary for the UI from command results
 * @param {string} toolName - The MCP tool name that was called
 * @param {object} result - The full result data from jit CLI
 * @returns {string} A concise summary for users
 */
function generateUserSummary(toolName, result) {
  // Extract command parts from tool name
  const parts = toolName.replace('jit_', '').split('_');
  const command = parts[0];
  const subcommand = parts.slice(1).join('-');
  
  // Handle different command types
  if (command === 'status') {
    return `Status: ${result.open} open, ${result.ready} ready, ${result.in_progress} in progress, ${result.done} done, ${result.blocked} blocked`;
  }
  
  if (command === 'issue') {
    if (subcommand === 'create') {
      return `Created issue: ${result.id.substring(0, 8)} - ${result.title}`;
    }
    if (subcommand === 'show') {
      const gates = result.gates_required?.length || 0;
      const gatesPassed = Object.values(result.gates_status || {}).filter(g => g.status === 'passed').length;
      return `Issue ${result.id.substring(0, 8)}: ${result.title} [${result.state}${gates > 0 ? `, ${gatesPassed}/${gates} gates` : ''}]`;
    }
    if (subcommand === 'update') {
      return `Updated issue ${result.id.substring(0, 8)}: ${result.title} → ${result.state}`;
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
      return `Deleted issue ${result.id.substring(0, 8)}`;
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
  
  if (command === 'validate') {
    if (result.valid === true) {
      return `✓ Validation passed`;
    } else if (result.valid === false) {
      const validations = result.validations || [];
      const failed = validations.filter(v => !v.valid);
      return `✗ Validation failed (${failed.length} issue${failed.length !== 1 ? 's' : ''})`;
    }
  }
  
  if (command === 'graph') {
    if (subcommand === 'show') {
      if (Array.isArray(result)) {
        return `Graph: ${result.length} issue${result.length !== 1 ? 's' : ''} in tree`;
      }
      const deps = result.dependencies?.length || 0;
      const blocked = result.blocked_by?.length || 0;
      return `Issue ${result.id?.substring(0, 8)}: ${deps} dependencies, ${blocked} blocked by`;
    }
    if (subcommand === 'deps' || subcommand === 'dependencies') {
      const deps = result.dependencies?.length || (Array.isArray(result) ? result.length : 0);
      return `${deps} direct dependenc${deps !== 1 ? 'ies' : 'y'}`;
    }
    if (subcommand === 'downstream') {
      const count = result.dependents?.length || (Array.isArray(result) ? result.length : 0);
      return `${count} downstream dependent${count !== 1 ? 's' : ''}`;
    }
    if (subcommand === 'roots') {
      const count = result.roots?.length || result.count || (Array.isArray(result) ? result.length : 0);
      return `${count} root issue${count !== 1 ? 's' : ''}`;
    }
    if (subcommand === 'export') {
      return `Graph exported`;
    }
  }
  
  if (command === 'registry') {
    if (subcommand === 'list') {
      const count = result.gates?.length || (Array.isArray(result) ? result.length : 0);
      return `${count} gate${count !== 1 ? 's' : ''} in registry`;
    }
    if (subcommand === 'show') {
      return `Gate ${result.key}: ${result.title}`;
    }
    if (subcommand === 'add') {
      return `Added gate ${result.key} to registry`;
    }
    if (subcommand === 'remove') {
      return `Removed gate from registry`;
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
  const MAX_DESCRIPTION_LENGTH = 200;
  
  // Helper to truncate descriptions
  const truncateDesc = (desc) => {
    if (!desc || typeof desc !== 'string') return desc;
    return desc.length > MAX_DESCRIPTION_LENGTH 
      ? desc.substring(0, MAX_DESCRIPTION_LENGTH) + '...' 
      : desc;
  };
  
  // Helper to compact an issue object
  const compactIssue = (issue) => {
    if (!issue || typeof issue !== 'object') return issue;
    const result = {
      id: issue.id,
      title: issue.title,
      state: issue.state,
      priority: issue.priority,
      // Omit full description, dependencies list, gates_status details
    };
    // Only include optional fields if present
    if (issue.assignee) result.assignee = issue.assignee;
    if (issue.labels?.length) result.labels = issue.labels;
    if (issue.blocked_reasons?.length) result.blocked_reasons = issue.blocked_reasons;
    return result;
  };
  
  // Extract command parts
  const parts = toolName.replace('jit_', '').split('_');
  const command = parts[0];
  const subcommand = parts.slice(1).join('-');
  
  // For gate check-all, return summary only
  if (command === 'gate' && subcommand === 'check-all') {
    if (Array.isArray(result)) {
      return result.map(g => ({
        gate_key: g.gate_key,
        status: g.status,
        duration_ms: g.duration_ms,
        // Omit stdout/stderr which can be huge
      }));
    }
  }
  
  // For query commands, compact issues and limit count
  if (command === 'query') {
    if (result.issues && Array.isArray(result.issues)) {
      return {
        count: result.count,
        filters: result.filters,
        issues: result.issues.slice(0, MAX_ARRAY_ITEMS).map(compactIssue),
        truncated: result.issues.length > MAX_ARRAY_ITEMS,
      };
    }
    if (Array.isArray(result)) {
      return {
        count: result.length,
        issues: result.slice(0, MAX_ARRAY_ITEMS).map(compactIssue),
        truncated: result.length > MAX_ARRAY_ITEMS,
      };
    }
  }
  
  // For issue search, compact results
  if (command === 'issue' && subcommand === 'search') {
    if (Array.isArray(result)) {
      return {
        count: result.length,
        issues: result.slice(0, MAX_ARRAY_ITEMS).map(compactIssue),
        truncated: result.length > MAX_ARRAY_ITEMS,
      };
    }
  }
  
  // For graph commands, compact issue data
  if (command === 'graph') {
    if (result.dependencies && Array.isArray(result.dependencies)) {
      return {
        ...result,
        dependencies: result.dependencies.slice(0, MAX_ARRAY_ITEMS).map(compactIssue),
        truncated: result.dependencies.length > MAX_ARRAY_ITEMS,
      };
    }
    if (Array.isArray(result)) {
      return result.slice(0, MAX_ARRAY_ITEMS).map(compactIssue);
    }
  }
  
  // For issue show, truncate description
  if (command === 'issue' && subcommand === 'show') {
    return {
      ...result,
      description: truncateDesc(result.description),
    };
  }
  
  // Default: return as-is for small results, truncate large arrays
  if (Array.isArray(result) && result.length > MAX_ARRAY_ITEMS) {
    return {
      count: result.length,
      items: result.slice(0, MAX_ARRAY_ITEMS),
      truncated: true,
    };
  }
  
  return result;
}

// Create MCP server
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

// Register tool handlers
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: generateTools(),
  };
});

// Build a lookup for tools with output schemas
const toolOutputSchemas = new Map();
for (const tool of generateTools()) {
  if (tool.outputSchema) {
    toolOutputSchemas.set(tool.name, tool.outputSchema);
  }
}

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;
  
  try {
    const result = await executeTool(name, args || {});
    const userSummary = generateUserSummary(name, result);
    
    // Check if this tool has a defined output schema
    const hasOutputSchema = toolOutputSchemas.has(name);
    
    // Smart compaction applied to reduce token usage
    const compactResult = compactForAssistant(name, result);
    
    if (hasOutputSchema) {
      // Use structuredContent for clients that support it
      // Also include JSON in content for backwards compatibility
      return {
        content: [
          {
            type: "text",
            text: userSummary,
            annotations: {
              audience: ["user"]
            }
          },
          {
            type: "text",
            text: JSON.stringify(compactResult, null, 2),
            annotations: {
              audience: ["assistant"]
            }
          }
        ],
        structuredContent: compactResult,
      };
    } else {
      // Fallback: return both user summary and JSON in content
      const fullData = JSON.stringify(compactResult, null, 2);
      
      return {
        content: [
          {
            type: "text",
            text: userSummary + "\n",
            annotations: {
              audience: ["user"]
            }
          },
          {
            type: "text",
            text: fullData,
            annotations: {
              audience: ["assistant"]
            }
          },
        ],
      };
    }
  } catch (error) {
    return {
      content: [
        {
          type: "text",
          text: `Error: ${error.message}`,
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
  console.error(`Tools: ${generateTools().length}`);
}

main().catch((error) => {
  console.error("Fatal error:", error);
  process.exit(1);
});
