/**
 * Schema loading with runtime preference
 * 
 * Loads JIT schema either from runtime CLI (`jit --schema`) or falls back
 * to bundled schema file. Warns when versions differ.
 */

import { execFile } from "child_process";
import { promisify } from "util";
import { readFileSync } from "fs";

const execFileAsync = promisify(execFile);

/**
 * Load schema from jit CLI at runtime
 * @returns {Promise<Object>} Schema object from jit --schema
 */
async function loadSchemaFromCli() {
  try {
    const { stdout } = await execFileAsync('jit', ['--schema'], {
      timeout: 5000,
      maxBuffer: 10 * 1024 * 1024,
    });
    return JSON.parse(stdout);
  } catch (error) {
    // CLI not available or doesn't support --schema
    return null;
  }
}

/**
 * Load schema from bundled file
 * @param {string} schemaPath - Path to bundled schema file
 * @returns {Object} Schema object
 */
function loadSchemaFromFile(schemaPath) {
  return JSON.parse(readFileSync(schemaPath, "utf-8"));
}

/**
 * Load schema with preference for runtime CLI
 * @param {string} bundledSchemaPath - Path to bundled schema file
 * @returns {Promise<{schema: Object, warnings: string[]}>}
 */
export async function loadSchema(bundledSchemaPath) {
  const warnings = [];
  
  // Try loading from CLI first
  const cliSchema = await loadSchemaFromCli();
  const bundledSchema = loadSchemaFromFile(bundledSchemaPath);
  
  if (!cliSchema) {
    warnings.push(
      "Could not load schema from jit CLI (--schema not supported or jit not in PATH). " +
      "Using bundled schema. Schema may be out of sync with installed jit binary."
    );
    return { schema: bundledSchema, warnings };
  }
  
  // Check version mismatch
  if (cliSchema.version !== bundledSchema.version) {
    warnings.push(
      `Schema version mismatch: CLI reports ${cliSchema.version}, ` +
      `bundled schema is ${bundledSchema.version}. Using CLI schema.`
    );
  }
  
  return { schema: cliSchema, warnings };
}
