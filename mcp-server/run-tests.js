#!/usr/bin/env node
/**
 * Runs the unit and integration suites against the `jit` built from this
 * checkout (debug profile, reusing `target/`), whatever `jit` PATH resolves.
 * `npm run test:suites` runs them against `JIT_BIN` or the `jit` on PATH.
 */

import { spawnSync } from 'node:child_process';
import { fileURLToPath } from 'node:url';

const workspaceRoot = fileURLToPath(new URL('..', import.meta.url));
const mcpRoot = fileURLToPath(new URL('.', import.meta.url));

const build = spawnSync(
  'cargo',
  ['build', '-p', 'jit', '--bin', 'jit', '--message-format=json-render-diagnostics'],
  { cwd: workspaceRoot, encoding: 'utf8', stdio: ['ignore', 'pipe', 'inherit'], maxBuffer: 64 * 1024 * 1024 }
);
if (build.status !== 0) {
  console.error('cargo build of the jit binary failed');
  process.exit(build.status ?? 1);
}

const executable = build.stdout
  .split('\n')
  .filter(line => line.startsWith('{'))
  .map(line => JSON.parse(line))
  .filter(message => message.reason === 'compiler-artifact'
    && message.target.name === 'jit'
    && message.target.kind.includes('bin')
    && message.executable)
  .map(message => message.executable)
  .at(-1);
if (!executable) {
  console.error('cargo reported no jit binary artifact');
  process.exit(1);
}
console.log(`Testing against ${executable}`);

for (const suite of ['test-unit.js', 'test-integration.js']) {
  const result = spawnSync('node', [suite], {
    cwd: mcpRoot,
    stdio: 'inherit',
    env: { ...process.env, JIT_BIN: executable },
  });
  if (result.status !== 0) process.exit(result.status ?? 1);
}
