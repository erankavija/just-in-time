/**
 * CLI command execution with timeouts and error handling
 * 
 * Executes jit CLI commands with proper timeout handling and structured
 * error responses.
 */

import { execFile } from "child_process";
import { promisify } from "util";

const execFileAsync = promisify(execFile);

// Default timeout for CLI commands (30 seconds)
const DEFAULT_TIMEOUT = 30000;

/**
 * Execute jit CLI command and return parsed output
 * @param {string[]} cmdPath - Command path (e.g., ['doc', 'assets', 'list'])
 * @param {Object} args - Command arguments
 * @param {Object} cmdDef - Command definition from schema
 * @param {number} timeout - Timeout in milliseconds
 * @returns {Promise<Object>} Parsed command output
 */
export async function executeCommand(cmdPath, args, cmdDef, timeout = DEFAULT_TIMEOUT) {
  const cliArgs = buildCliArgs(cmdPath, args, cmdDef);
  
  try {
    const { stdout, stderr } = await execFileAsync('jit', cliArgs, {
      timeout,
      maxBuffer: 10 * 1024 * 1024, // 10MB buffer
    });
    
    if (stderr && !stdout) {
      return createErrorResponse('COMMAND_ERROR', stderr);
    }
    
    // Check if command expects JSON output
    const expectsJson = cliArgs.includes('--json');
    
    if (!expectsJson) {
      return createSuccessResponse({ message: stdout.trim() });
    }
    
    // Parse JSON output
    const result = JSON.parse(stdout);
    
    // If response has success/data wrapper, unwrap it
    if (result.success === true && result.data !== undefined) {
      return createSuccessResponse(result.data);
    }
    
    // If response has error, return error
    if (result.success === false && result.error) {
      return createErrorResponse(
        result.error.code || 'ERROR',
        result.error.message
      );
    }
    
    // Otherwise return as-is
    return createSuccessResponse(result);
  } catch (error) {
    // Handle timeout
    if (error.killed && error.signal === 'SIGTERM') {
      return createErrorResponse('TIMEOUT', `Command timed out after ${timeout}ms`);
    }
    
    // Try to parse error stdout for structured errors
    if (error.stdout) {
      try {
        const result = JSON.parse(error.stdout);
        if (!result.success && result.error) {
          return createErrorResponse(
            result.error.code || 'ERROR',
            result.error.message
          );
        }
      } catch {}
    }
    
    // Generic error
    return createErrorResponse('EXECUTION_ERROR', error.message);
  }
}

/**
 * Build CLI arguments array from command path, args, and command definition
 * @param {string[]} cmdPath - Command path (e.g., ['doc', 'assets', 'list'])
 * @param {Object} args - Command arguments
 * @param {Object} cmdDef - Command definition from schema
 * @returns {string[]} CLI arguments array
 */
export function buildCliArgs(cmdPath, args, cmdDef) {
  const cliArgs = [...cmdPath]; // Start with command path
  
  // Get positional argument names from schema
  const positionalArgNames = (cmdDef.args || []).map(arg => arg.name);
  
  // Add positional arguments
  for (const argName of positionalArgNames) {
    const value = args[argName];
    if (value !== undefined && value !== "") {
      cliArgs.push(String(value));
    }
  }
  
  // Add flag arguments
  const positionalSet = new Set(positionalArgNames);
  const hasJsonFlag = cmdDef.flags?.some(flag => flag.name === "json");
  
  for (const [key, value] of Object.entries(args)) {
    // Skip positional args (already added)
    if (positionalSet.has(key)) continue;
    
    if (Array.isArray(value)) {
      for (const item of value) {
        cliArgs.push(`--${key}`, String(item));
      }
    } else if (typeof value === 'boolean') {
      // Boolean flags: only add if true, without value
      if (value) {
        cliArgs.push(`--${key}`);
      }
    } else if (value !== undefined && value !== "") {
      cliArgs.push(`--${key}`, String(value));
    }
  }
  
  // Add --json flag if supported and not already present
  if (hasJsonFlag && !cliArgs.includes('--json') && !args.json) {
    cliArgs.push('--json');
  }
  
  return cliArgs;
}

/**
 * Create success response envelope
 * @param {any} data - Response data
 * @returns {Object} Success response
 */
function createSuccessResponse(data) {
  return {
    success: true,
    data,
  };
}

/**
 * Create error response envelope
 * @param {string} code - Error code
 * @param {string} message - Error message
 * @returns {Object} Error response
 */
function createErrorResponse(code, message) {
  return {
    success: false,
    error: {
      code,
      message,
    },
  };
}
