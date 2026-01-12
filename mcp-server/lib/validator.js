/**
 * Input validation using Zod
 * 
 * Validates tool call arguments against the schema before execution.
 */

import { z } from "zod";

/**
 * Convert JSON Schema property to Zod schema
 * @param {Object} property - JSON Schema property definition
 * @param {boolean} isRequired - Whether the property is required
 * @returns {z.ZodType} Zod schema
 */
function jsonSchemaToZod(property, isRequired) {
  let schema;
  
  switch (property.type) {
    case "string":
      schema = z.string();
      break;
    case "number":
      schema = z.number();
      break;
    case "integer":
      schema = z.number().int();
      break;
    case "boolean":
      schema = z.boolean();
      break;
    case "array":
      if (property.items) {
        const itemSchema = jsonSchemaToZod(property.items, true);
        schema = z.array(itemSchema);
      } else {
        schema = z.array(z.any());
      }
      break;
    default:
      schema = z.any();
  }
  
  // Add default value if present
  if (property.default !== undefined) {
    schema = schema.default(property.default);
  }
  
  // Make optional if not required
  if (!isRequired) {
    schema = schema.optional();
  }
  
  return schema;
}

/**
 * Create Zod validator from tool's input schema
 * @param {Object} inputSchema - Tool's JSON Schema input schema
 * @returns {z.ZodObject} Zod object schema
 */
export function createValidator(inputSchema) {
  const shape = {};
  const required = new Set(inputSchema.required || []);
  
  for (const [propName, propDef] of Object.entries(inputSchema.properties || {})) {
    shape[propName] = jsonSchemaToZod(propDef, required.has(propName));
  }
  
  return z.object(shape);
}

/**
 * Validate arguments against tool schema
 * @param {Object} args - Arguments to validate
 * @param {Object} inputSchema - Tool's JSON Schema input schema
 * @returns {{success: boolean, data?: Object, error?: string}}
 */
export function validateArguments(args, inputSchema) {
  try {
    const validator = createValidator(inputSchema);
    const validated = validator.parse(args);
    return { success: true, data: validated };
  } catch (error) {
    if (error instanceof z.ZodError) {
      const issues = error.issues.map(issue => 
        `${issue.path.join('.')}: ${issue.message}`
      ).join('; ');
      return { 
        success: false, 
        error: `Validation failed: ${issues}` 
      };
    }
    return { 
      success: false, 
      error: `Validation error: ${error.message}` 
    };
  }
}
