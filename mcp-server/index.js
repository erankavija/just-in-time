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
          throw new Error(`${result.error.code}: ${result.error.message}`);
        }
      } catch {}
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
      properties[arg.name] = {
        type: arg.type === "array[string]" ? "array" : arg.type,
        description: arg.description || `${arg.name} parameter`,
      };
      
      if (arg.type === "array[string]") {
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
  
  return {
    name: toolName,
    description: cmd.description,
    inputSchema: {
      type: "object",
      properties,
      required,
    },
  };
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

// Register tool handlers
server.setRequestHandler(ListToolsRequestSchema, async () => {
  return {
    tools: generateTools(),
  };
});

server.setRequestHandler(CallToolRequestSchema, async (request) => {
  const { name, arguments: args } = request.params;
  
  try {
    const result = await executeTool(name, args || {});
    
    return {
      content: [
        {
          type: "text",
          text: JSON.stringify(result, null, 2),
        },
      ],
    };
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
