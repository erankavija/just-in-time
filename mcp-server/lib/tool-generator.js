/**
 * Tool generation from JIT schema
 * 
 * Generates MCP tool definitions from JIT schema, including support for
 * nested subcommands (e.g., doc.assets.list).
 */

/**
 * Generate MCP tool definition from JIT schema command
 * @param {string[]} path - Command path (e.g., ['doc', 'assets', 'list'])
 * @param {Object} cmd - Command definition from schema
 * @returns {Object} MCP tool definition
 */
function generateToolFromCommand(path, cmd) {
  const toolName = `jit_${path.join('_')}`;
  
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
  
  return {
    name: toolName,
    description: cmd.description,
    inputSchema: {
      type: "object",
      properties,
      required,
    },
    // Store path for later CLI mapping
    _commandPath: path,
  };
}

/**
 * Recursively generate tools from schema commands
 * @param {Object} commands - Commands object from schema
 * @param {string[]} parentPath - Parent command path
 * @returns {Object[]} Array of tool definitions
 */
function generateToolsRecursive(commands, parentPath = []) {
  const tools = [];
  
  for (const [cmdName, cmd] of Object.entries(commands)) {
    const currentPath = [...parentPath, cmdName];
    
    if (cmd.subcommands) {
      // Has subcommands - recurse
      tools.push(...generateToolsRecursive(cmd.subcommands, currentPath));
    } else {
      // Leaf command - generate tool
      tools.push(generateToolFromCommand(currentPath, cmd));
    }
  }
  
  return tools;
}

/**
 * Generate all MCP tools from JIT schema
 * @param {Object} schema - JIT schema object
 * @returns {Object[]} Array of MCP tool definitions
 */
export function generateTools(schema) {
  return generateToolsRecursive(schema.commands);
}

/**
 * Parse tool name back to command path
 * @param {string} toolName - MCP tool name (e.g., 'jit_doc_assets_list')
 * @returns {string[]} Command path (e.g., ['doc', 'assets', 'list'])
 */
export function parseToolName(toolName) {
  // Remove 'jit_' prefix and split
  const withoutPrefix = toolName.replace(/^jit_/, '');
  return withoutPrefix.split('_');
}

/**
 * Get command definition from schema by path
 * @param {Object} schema - JIT schema object
 * @param {string[]} path - Command path
 * @returns {Object|null} Command definition or null if not found
 */
export function getCommandByPath(schema, path) {
  let current = schema.commands;
  
  for (let i = 0; i < path.length; i++) {
    const segment = path[i];
    
    if (!current[segment]) {
      return null;
    }
    
    if (i === path.length - 1) {
      // Last segment - return the command
      return current[segment];
    }
    
    // Move to subcommands
    current = current[segment].subcommands;
    if (!current) {
      return null;
    }
  }
  
  return null;
}
