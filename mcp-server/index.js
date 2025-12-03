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
import { exec } from "child_process";
import { promisify } from "util";
import { readFileSync } from "fs";
import { join, dirname } from "path";
import { fileURLToPath } from "url";

const execAsync = promisify(exec);
const __dirname = dirname(fileURLToPath(import.meta.url));

// Load JIT schema
const schemaPath = join(__dirname, "jit-schema.json");
const jitSchema = JSON.parse(readFileSync(schemaPath, "utf-8"));

/**
 * Execute jit CLI command and return parsed JSON output
 */
async function runJitCommand(args, useJsonFlag = true) {
  const jsonFlag = useJsonFlag ? " --json" : "";
  const cmd = `jit ${args}${jsonFlag}`;
  try {
    const { stdout, stderr } = await execAsync(cmd, {
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    });
    
    if (stderr && !stdout) {
      throw new Error(stderr);
    }
    
    // Commands without --json flag return plain text
    if (!useJsonFlag) {
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
const POSITIONAL_ARG_COMMANDS = {
  "dep_add": ["from", "to"],
  "dep_rm": ["from", "to"],
  "issue_show": ["id"],
  "issue_delete": ["id"],
  "issue_claim": ["id", "assignee"],
  "issue_unclaim": ["id"],
  "issue_update": ["id"],
  "registry_show": ["key"],
  "graph_downstream": ["id"],
  "gate_add": ["id", "gate"],
  "gate_pass": ["id", "gate"],
  "gate_fail": ["id", "gate"],
};

/**
 * Execute MCP tool by mapping to jit CLI command
 */
async function executeTool(name, args) {
  // Parse tool name: jit_<command>_<subcommand> or jit_<command>
  const parts = name.split("_");
  parts.shift(); // Remove 'jit' prefix
  
  let cliArgs;
  let hasJsonFlag;
  
  if (parts.length === 2) {
    // Subcommand: jit_issue_create -> jit issue create
    const [cmd, subcmd] = parts;
    const cmdKey = `${cmd}_${subcmd}`;
    cliArgs = `${cmd} ${subcmd}`;
    hasJsonFlag = supportsJsonFlag(cmd, subcmd);
    
    // Check if this command uses positional arguments
    const positionalArgNames = POSITIONAL_ARG_COMMANDS[cmdKey];
    
    if (positionalArgNames) {
      // Use positional arguments
      const positionalArgs = [];
      for (const argName of positionalArgNames) {
        const value = args[argName];
        if (value !== undefined && value !== "") {
          positionalArgs.push(`"${value}"`);
        }
      }
      cliArgs += positionalArgs.length > 0 ? ` ${positionalArgs.join(" ")}` : "";
      
      // Add any remaining arguments as flags
      const positionalSet = new Set(positionalArgNames);
      const flagArgs = [];
      for (const [key, value] of Object.entries(args)) {
        if (positionalSet.has(key)) continue;
        
        if (Array.isArray(value)) {
          for (const item of value) {
            flagArgs.push(`--${key} "${item}"`);
          }
        } else if (value !== undefined && value !== "") {
          flagArgs.push(`--${key} "${value}"`);
        }
      }
      cliArgs += flagArgs.length > 0 ? ` ${flagArgs.join(" ")}` : "";
    } else {
      // All arguments are flags
      const flagArgs = [];
      for (const [key, value] of Object.entries(args)) {
        if (Array.isArray(value)) {
          for (const item of value) {
            flagArgs.push(`--${key} "${item}"`);
          }
        } else if (value !== undefined && value !== "") {
          flagArgs.push(`--${key} "${value}"`);
        }
      }
      cliArgs += flagArgs.length > 0 ? ` ${flagArgs.join(" ")}` : "";
    }
  } else {
    // Top-level command: jit_status -> jit status
    cliArgs = parts[0];
    hasJsonFlag = supportsJsonFlag(parts[0]);
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
