/**
 * Schema loading from the jit CLI
 *
 * The CLI is the single source of truth for the command schema.
 * No bundled schema file is shipped — `jit --schema` must be available.
 */

import { execFile } from "child_process";
import { promisify } from "util";
import { JIT_BINARY } from "./jit-binary.js";

const execFileAsync = promisify(execFile);

/**
 * Load schema from the jit CLI (`jit --schema`).
 * @returns {Promise<{schema: Object, warnings: string[]}>}
 * @throws {Error} if the jit binary cannot be run or does not support --schema
 */
export async function loadSchema() {
  const warnings = [];

  try {
    const { stdout } = await execFileAsync(JIT_BINARY, ['--schema'], {
      timeout: 5000,
      maxBuffer: 10 * 1024 * 1024,
    });
    return { schema: JSON.parse(stdout), warnings };
  } catch (error) {
    throw new Error(
      "Failed to load schema from jit CLI. Ensure jit is installed and in PATH, or set JIT_BIN.\n" +
      `Underlying error: ${error.message}`
    );
  }
}
