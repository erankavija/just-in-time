/**
 * The `jit` executable the bridge and its test suites run.
 *
 * `JIT_BIN` names it explicitly; without it the first `jit` on PATH is used.
 */
export const JIT_BINARY = process.env.JIT_BIN || 'jit';
