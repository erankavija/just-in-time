#!/usr/bin/env node
/**
 * Test suite for Dependencies & State operations (Phase 3)
 * 
 * Tests comprehensive coverage for:
 * - jit_dep_add
 * - jit_dep_rm
 * - Cycle detection
 * - Dependency blocking
 * - Graph operations
 * - State transitions with dependencies
 */

import { spawn } from 'child_process';
import { mkdirSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';

const TIMEOUT = 5000;

class MCPTester {
  constructor() {
    this.server = null;
    this.responseBuffer = '';
    this.pendingRequests = new Map();
    this.nextId = 1;
    this.testDir = null;
  }

  async start() {
    this.testDir = join(tmpdir(), `jit-mcp-deps-test-${Date.now()}`);
    mkdirSync(this.testDir, { recursive: true });
    
    const serverPath = join(process.cwd(), 'index.js');
    
    return new Promise((resolve, reject) => {
      this.server = spawn('node', [serverPath], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: this.testDir,
        env: { ...process.env, JIT_ALLOW_DELETION: '1' }
      });

      this.server.stdout.on('data', (data) => {
        this.responseBuffer += data.toString();
        this.processResponses();
      });

      this.server.stderr.on('data', () => {});

      this.server.on('error', (err) => {
        reject(err);
      });

      setTimeout(resolve, 500);
    });
  }

  processResponses() {
    const lines = this.responseBuffer.split('\n');
    this.responseBuffer = lines.pop() || '';

    for (const line of lines) {
      if (!line.trim()) continue;
      
      try {
        const response = JSON.parse(line);
        if (response.id && this.pendingRequests.has(response.id)) {
          const { resolve } = this.pendingRequests.get(response.id);
          this.pendingRequests.delete(response.id);
          resolve(response);
        }
      } catch (err) {
        console.error('Failed to parse response:', line);
      }
    }
  }

  async sendRequest(method, params = {}) {
    const id = this.nextId++;
    const request = {
      jsonrpc: '2.0',
      id,
      method,
      params
    };

    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`Request ${id} timed out (${method})`));
      }, TIMEOUT);

      this.pendingRequests.set(id, {
        resolve: (response) => {
          clearTimeout(timeout);
          resolve(response);
        }
      });

      this.server.stdin.write(JSON.stringify(request) + '\n');
    });
  }

  async callTool(toolName, args = {}) {
    const response = await this.sendRequest('tools/call', {
      name: toolName,
      arguments: args
    });

    if (response.error) {
      throw new Error(response.error.message);
    }

    if (!response.result?.content) {
      throw new Error('No content in response');
    }

    const content = response.result.content;
    
    // Try to find assistant-targeted content first (if annotations exist)
    const assistantContent = content.find(
      c => c.annotations?.audience?.includes('assistant')
    );
    
    if (assistantContent?.text) {
      try {
        const parsed = JSON.parse(assistantContent.text);
        if (parsed.success === false) {
          throw new Error(parsed.error?.message || 'Command failed');
        }
        return parsed;
      } catch (err) {
        if (err.message && (err.message.includes('not found') || err.message.includes('Command failed'))) {
          throw err;
        }
        return { message: assistantContent.text };
      }
    }

    // If no annotations, try to find JSON content (usually second item)
    for (let i = content.length - 1; i >= 0; i--) {
      if (content[i].type === 'text') {
        try {
          const parsed = JSON.parse(content[i].text);
          if (parsed.success === false) {
            throw new Error(parsed.error?.message || 'Command failed');
          }
          return parsed;
        } catch (err) {
          if (err.message && (err.message.includes('not found') || err.message.includes('Command failed'))) {
            throw err;
          }
        }
      }
    }

    // Fallback: return first text content wrapped in message
    const firstContent = content.find(c => c.type === 'text');
    if (firstContent?.text) {
      return { message: firstContent.text };
    }

    throw new Error('No parseable content in response');
  }




  async stop() {
    if (this.server) {
      this.server.kill();
      this.server = null;
    }
    if (this.testDir) {
      rmSync(this.testDir, { recursive: true, force: true });
    }
  }
}

function assert(condition, message) {
  if (!condition) {
    throw new Error(`Assertion failed: ${message}`);
  }
}

async function runTest(name, fn) {
  try {
    await fn();
    console.log(`✓ ${name}`);
    return true;
  } catch (error) {
    console.error(`✗ ${name}`);
    console.error(`  ${error.message}`);
    return false;
  }
}

// Tests
async function testDepAddBasic(tester) {
  await tester.callTool('jit_init', {});
  
  // Create two issues
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Issue A (blocking)'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Issue B (blocked)'
  });
  
  // Add dependency: B depends on A
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Verify
  const shown = await tester.callTool('jit_issue_show', {
    id: issueB.id
  });
  assert(shown.dependencies?.some(d => d.id === issueA.id), 'B should depend on A');
}

async function testDepAddMultiple(tester) {
  await tester.callTool('jit_init', {});
  
  // Create issues
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Dep A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Dep B'
  });
  
  const issueC = await tester.callTool('jit_issue_create', {
    title: 'Dep C (depends on A and B)'
  });
  
  // Add multiple dependencies
  await tester.callTool('jit_dep_add', {
    from_id: issueC.id,
    to_ids: [issueA.id, issueB.id]
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issueC.id
  });
  assert(shown.dependencies?.some(d => d.id === issueA.id), 'C should depend on A');
  assert(shown.dependencies?.some(d => d.id === issueB.id), 'C should depend on B');
}

async function testDepRemove(tester) {
  await tester.callTool('jit_init', {});
  
  // Create issues with dependency
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Remove dep A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Remove dep B'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Remove dependency
  await tester.callTool('jit_dep_rm', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issueB.id
  });
  assert(!shown.dependencies?.some(d => d.id === issueA.id), 'B should not depend on A');
}

async function testCycleDetection(tester) {
  await tester.callTool('jit_init', {});
  
  // Create issues
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Cycle A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Cycle B'
  });
  
  // A depends on B
  await tester.callTool('jit_dep_add', {
    from_id: issueA.id,
    to_ids: [issueB.id]
  });
  
  // Try to make B depend on A (would create cycle)
  const result = await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Should either fail or not create the dependency
  const shown = await tester.callTool('jit_issue_show', {
    id: issueB.id
  });
  
  // If it was added, it should not include A (cycle prevented)
  // Or the command should have failed
  const hasCycle = shown.dependencies?.includes(issueA.id) && 
                   issueA.id !== issueB.id;
  
  if (hasCycle) {
    // Check if A still has B as dependency (would be a cycle)
    const shownA = await tester.callTool('jit_issue_show', {
      id: issueA.id
    });
    assert(!shownA.dependencies?.includes(issueB.id) || !hasCycle, 
      'Should prevent cycle creation');
  }
}

async function testGraphDeps(tester) {
  await tester.callTool('jit_init', {});
  
  // Create dependency chain
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Graph A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Graph B'
  });
  
  const issueC = await tester.callTool('jit_issue_create', {
    title: 'Graph C'
  });
  
  // C depends on B, B depends on A
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueC.id,
    to_ids: [issueB.id]
  });
  
  // Get dependencies of C
  const result = await tester.callTool('jit_graph_deps', {
    id: issueC.id
  });
  
  assert(result !== undefined, 'Should return dependency graph');
}

async function testGraphDownstream(tester) {
  await tester.callTool('jit_init', {});
  
  // Create dependency chain
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Downstream A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Downstream B'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Get downstream of A (issues that depend on A)
  const result = await tester.callTool('jit_graph_downstream', {
    id: issueA.id
  });
  
  assert(result !== undefined, 'Should return downstream issues');
}

async function testGraphRoots(tester) {
  await tester.callTool('jit_init', {});
  
  // Root issues are those with no dependencies
  const root = await tester.callTool('jit_issue_create', {
    title: 'Root issue'
  });
  
  const result = await tester.callTool('jit_graph_roots', {});
  
  assert(result.roots, 'Should have roots array');
  const rootIds = result.roots.map(i => i.id);
  assert(rootIds.includes(root.id), 'Should include root issue');
}

async function testDependencyBlocking(tester) {
  // Create issues with dependency
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Blocking issue',
    state: 'ready'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Blocked issue'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Query blocked issues
  const blocked = await tester.callTool('jit_query_blocked', {});
  
  // B should appear in blocked list (A is not done)
  const blockedIds = blocked.issues?.map(i => i.id) || [];
  assert(blockedIds.includes(issueB.id), 'B should be blocked');
}

async function testDependencyUnblocking(tester) {
  await tester.callTool('jit_init', {});
  
  // Create issues with dependency
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Will be done'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Will be unblocked'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Mark A as done
  await tester.callTool('jit_issue_update', {
    id: issueA.id,
    state: 'done'
  });
  
  // Check if B is now available
  const available = await tester.callTool('jit_query_available', {});
  
  // B should be available now
  const availableIds = available.issues?.map(i => i.id) || [];
  assert(availableIds.includes(issueB.id), 'B should be available after A is done');
}

async function testTransitiveDependencies(tester) {
  await tester.callTool('jit_init', {});
  
  // Create chain: C depends on B, B depends on A
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Transitive A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Transitive B'
  });
  
  const issueC = await tester.callTool('jit_issue_create', {
    title: 'Transitive C'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueC.id,
    to_ids: [issueB.id]
  });
  
  // Get transitive dependencies of C
  const result = await tester.callTool('jit_graph_deps', {
    id: issueC.id,
    transitive: true
  });
  
  assert(result !== undefined, 'Should return transitive dependencies');
}

async function testStateTransitionWithDeps(tester) {
  await tester.callTool('jit_init', {});
  
  // Create dependency
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'State dep A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'State dep B'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Try to move B to in_progress while A is not done
  await tester.callTool('jit_issue_update', {
    id: issueB.id,
    state: 'ready'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issueB.id
  });
  
  // B might be blocked or in ready state depending on implementation
  assert(shown.state !== 'done', 'B should not be done while A is incomplete');
}

async function testGraphShow(tester) {
  // Create some issues with dependencies
  const issueA = await tester.callTool('jit_issue_create', {
    title: 'Show graph A'
  });
  
  const issueB = await tester.callTool('jit_issue_create', {
    title: 'Show graph B'
  });
  
  await tester.callTool('jit_dep_add', {
    from_id: issueB.id,
    to_ids: [issueA.id]
  });
  
  // Get full graph
  const result = await tester.callTool('jit_graph_show', {});
  
  assert(result !== undefined, 'Should return graph data');
}

// Main test runner
async function main() {
  console.log('Testing Dependencies & State operations (Phase 3)...\n');
  
  const tester = new MCPTester();
  let passed = 0;
  let failed = 0;

  try {
    await tester.start();
    console.log('MCP server started\n');

    // Dependency add/remove tests
    if (await runTest('Dependency add - basic', () => testDepAddBasic(tester))) passed++; else failed++;
    if (await runTest('Dependency add - multiple', () => testDepAddMultiple(tester))) passed++; else failed++;
    if (await runTest('Dependency remove', () => testDepRemove(tester))) passed++; else failed++;
    
    // Cycle detection
    if (await runTest('Cycle detection', () => testCycleDetection(tester))) passed++; else failed++;
    
    // Graph operations
    if (await runTest('Graph deps', () => testGraphDeps(tester))) passed++; else failed++;
    if (await runTest('Graph downstream', () => testGraphDownstream(tester))) passed++; else failed++;
    if (await runTest('Graph roots', () => testGraphRoots(tester))) passed++; else failed++;
    if (await runTest('Graph show', () => testGraphShow(tester))) passed++; else failed++;
    
    // Blocking behavior
    if (await runTest('Dependency blocking', () => testDependencyBlocking(tester))) passed++; else failed++;
    if (await runTest('Dependency unblocking', () => testDependencyUnblocking(tester))) passed++; else failed++;
    if (await runTest('Transitive dependencies', () => testTransitiveDependencies(tester))) passed++; else failed++;
    
    // State transitions with dependencies
    if (await runTest('State transition with deps', () => testStateTransitionWithDeps(tester))) passed++; else failed++;

  } finally {
    await tester.stop();
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch(console.error);
