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
 * @param {Object} globalTypes - Global type definitions from schema.types
 * @param {boolean} includeOutputSchema - Whether to include outputSchema in tool definition
 * @returns {Object} MCP tool definition
 */
function generateToolFromCommand(path, cmd, globalTypes = {}, includeOutputSchema = true) {
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
  
  const tool = {
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

  // Add outputSchema from CLI schema when available (only in structured mode)
  if (includeOutputSchema && cmd.output?.success_schema) {
    tool.outputSchema = toMcpOutputSchema(cmd.output.success_schema, globalTypes);
  }

  return tool;
}

/**
 * Convert a CLI success_schema to an MCP-compatible outputSchema.
 * MCP requires type: 'object' at the root. Inline $ref definitions are
 * flattened since MCP outputSchema doesn't support $ref.
 * @param {Object} schema - JSON Schema from CLI
 * @param {Object} globalTypes - Global type definitions from schema.types
 * @returns {Object} MCP-compatible output schema
 */
function toMcpOutputSchema(schema, globalTypes = {}) {
  // Clone to avoid mutating the original
  const out = JSON.parse(JSON.stringify(schema));

  // Remove JSON Schema draft identifier (not needed in MCP)
  delete out.$schema;
  delete out.title;

  // Merge local definitions with global types for resolution
  const allDefinitions = {
    ...globalTypes,
    ...(out.definitions || {})
  };

  // Recursively resolve all $ref references
  const resolved = resolveRefs(out, allDefinitions);
  
  // Remove definitions section (no longer needed)
  delete resolved.definitions;
  
  return ensureObjectType(resolved);
}

/**
 * Recursively resolve all $ref references in a schema object.
 * @param {any} obj - Schema object or value to process
 * @param {Object} definitions - All available definitions (local + global)
 * @param {Set} visiting - Set of refs being resolved (for cycle detection)
 * @returns {any} Schema with all $ref resolved
 */
function resolveRefs(obj, definitions, visiting = new Set()) {
  // Handle primitives and null
  if (obj === null || typeof obj !== 'object') {
    return obj;
  }

  // Handle arrays
  if (Array.isArray(obj)) {
    return obj.map(item => resolveRefs(item, definitions, visiting));
  }

  // Handle $ref objects
  if (obj.$ref && typeof obj.$ref === 'string') {
    const refPath = obj.$ref;
    
    // Extract definition name from #/definitions/Name or #/types/Name
    const match = refPath.match(/^#\/(definitions|types)\/(.+)$/);
    if (match) {
      const defName = match[2];
      
      // Prevent infinite recursion
      if (visiting.has(defName)) {
        console.error(`Warning: Circular $ref detected: ${defName}`);
        return { type: 'object' }; // Fallback for circular refs
      }
      
      const definition = definitions[defName];
      if (definition) {
        // Resolve nested refs in the definition
        visiting.add(defName);
        const resolved = resolveRefs(definition, definitions, visiting);
        visiting.delete(defName);
        return resolved;
      } else {
        console.error(`Warning: Definition not found: ${defName}`);
        return { type: 'object' }; // Fallback for missing definitions
      }
    }
    
    // Unknown $ref format, leave as-is (will likely cause an error, but at least we tried)
    console.error(`Warning: Unrecognized $ref format: ${refPath}`);
    return obj;
  }

  // Handle regular objects: recursively process all properties
  const result = {};
  for (const [key, value] of Object.entries(obj)) {
    result[key] = resolveRefs(value, definitions, visiting);
  }
  return result;
}

/**
 * Ensure schema has type: 'object' at root (MCP requirement).
 * @param {Object} schema
 * @returns {Object}
 */
function ensureObjectType(schema) {
  if (!schema.type) {
    schema.type = 'object';
  }
  return schema;
}

/**
 * Recursively generate tools from schema commands
 * @param {Object} commands - Commands object from schema
 * @param {string[]} parentPath - Parent command path
 * @param {Object} globalTypes - Global type definitions from schema.types
 * @param {boolean} includeOutputSchema - Whether to include outputSchema in tool definitions
 * @returns {Object[]} Array of tool definitions
 */
function generateToolsRecursive(commands, parentPath = [], globalTypes = {}, includeOutputSchema = true) {
  const tools = [];
  
  for (const [cmdName, cmd] of Object.entries(commands)) {
    const currentPath = [...parentPath, cmdName];
    
    if (cmd.subcommands) {
      // Has subcommands - recurse
      tools.push(...generateToolsRecursive(cmd.subcommands, currentPath, globalTypes, includeOutputSchema));
    } else {
      // Leaf command - generate tool
      tools.push(generateToolFromCommand(currentPath, cmd, globalTypes, includeOutputSchema));
    }
  }
  
  return tools;
}

/**
 * Generate all MCP tools from JIT schema
 * @param {Object} schema - JIT schema object
 * @param {boolean} includeOutputSchema - Whether to include outputSchema in tool definitions (default: true)
 * @returns {Object[]} Array of MCP tool definitions
 */
export function generateTools(schema, includeOutputSchema = true) {
  return generateToolsRecursive(schema.commands, [], schema.types || {}, includeOutputSchema);
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
