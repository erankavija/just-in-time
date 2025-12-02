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
async function runJitCommand(args) {
  const cmd = `jit ${args} --json`;
  try {
    const { stdout, stderr } = await execAsync(cmd, {
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    });
    
    if (stderr && !stdout) {
      throw new Error(stderr);
    }
    
    const result = JSON.parse(stdout);
    
    if (!result.success) {
      throw new Error(result.error?.message || "Command failed");
    }
    
    return result.data;
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
 * Execute MCP tool by mapping to jit CLI command
 */
async function executeTool(name, args) {
  // Parse tool name: jit_<command>_<subcommand> or jit_<command>
  const parts = name.split("_");
  parts.shift(); // Remove 'jit' prefix
  
  let cliArgs;
  
  if (parts.length === 2) {
    // Subcommand: jit_issue_create -> jit issue create
    const [cmd, subcmd] = parts;
    cliArgs = `${cmd} ${subcmd}`;
    
    // Build arguments
    const argPairs = [];
    for (const [key, value] of Object.entries(args)) {
      if (Array.isArray(value)) {
        // Handle array arguments like --gate
        for (const item of value) {
          argPairs.push(`--${key} "${item}"`);
        }
      } else if (value !== undefined && value !== "") {
        argPairs.push(`--${key} "${value}"`);
      }
    }
    
    cliArgs += argPairs.length > 0 ? ` ${argPairs.join(" ")}` : "";
  } else {
    // Top-level command: jit_status -> jit status
    cliArgs = parts[0];
  }
  
  return await runJitCommand(cliArgs);
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
