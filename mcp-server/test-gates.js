#!/usr/bin/env node
/**
 * Test suite for Gate operations (Phase 2)
 * 
 * Tests comprehensive coverage for:
 * - jit_gate_define
 * - jit_gate_add/remove
 * - jit_gate_pass/fail
 * - jit_gate_check/check-all
 * - jit_gate_list
 * - Gate validation and blocking behavior
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
    this.testDir = join(tmpdir(), `jit-mcp-gate-test-${Date.now()}`);
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
async function testGateDefineBasic(tester) {
  await tester.callTool('jit_init', {});
  
  const result = await tester.callTool('jit_gate_define', {
    key: 'test-basic',
    title: 'Test Basic Gate',
    description: 'A basic test gate'
  });
  
  assert(result.success !== false, 'Should succeed');
}

async function testGateDefineWithStage(tester) {
  await tester.callTool('jit_gate_define', {
    key: 'precheck-gate',
    title: 'Precheck Gate',
    description: 'Runs before work starts',
    stage: 'precheck'
  });
  
  const list = await tester.callTool('jit_gate_list', {});
  const gate = list.gates?.find(g => g.key === 'precheck-gate');
  assert(gate, 'Should find precheck-gate');
  assert(gate.stage === 'precheck', 'Should have precheck stage');
}

async function testGateDefineWithMode(tester) {
  await tester.callTool('jit_gate_define', {
    key: 'auto-gate',
    title: 'Auto Gate',
    description: 'Automatic gate',
    mode: 'auto',
    'checker-command': 'echo "test"'
  });
  
  const list = await tester.callTool('jit_gate_list', {});
  const gate = list.gates?.find(g => g.key === 'auto-gate');
  assert(gate, 'Should find auto-gate');
  assert(gate.mode === 'auto', 'Should have auto mode');
}

async function testGateList(tester) {
  // Define multiple gates
  await tester.callTool('jit_gate_define', {
    key: 'gate1',
    title: 'Gate 1',
    description: 'First gate'
  });
  
  await tester.callTool('jit_gate_define', {
    key: 'gate2',
    title: 'Gate 2',
    description: 'Second gate'
  });
  
  const result = await tester.callTool('jit_gate_list', {});
  assert(result.gates, 'Should have gates array');
  assert(result.gates.length >= 2, 'Should have at least 2 gates');
  
  const keys = result.gates.map(g => g.key);
  assert(keys.includes('gate1'), 'Should include gate1');
  assert(keys.includes('gate2'), 'Should include gate2');
}

async function testGateShow(tester) {
  await tester.callTool('jit_gate_define', {
    key: 'show-test',
    title: 'Show Test Gate',
    description: 'Gate for show test'
  });
  
  const result = await tester.callTool('jit_gate_show', {
    key: 'show-test'
  });
  
  assert(result.key === 'show-test', 'Should have correct key');
  assert(result.title === 'Show Test Gate', 'Should have correct title');
  assert(result.description === 'Gate for show test', 'Should have correct description');
}

async function testGateAddToIssue(tester) {
  await tester.callTool('jit_init', {});

  // Define gate
  await tester.callTool('jit_gate_define', {
    key: 'add-gate-test',
    title: 'Add Gate Test',
    description: 'Test adding gate'
  });
  
  // Create issue
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Issue for gate add'
  });
  
  // Add gate
  await tester.callTool('jit_gate_add', {
    id: issue.id,
    gate_keys: ['add-gate-test']
  });
  
  // Verify
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.gates_required?.includes('add-gate-test'), 'Should have gate');
}

async function testGateAddMultipleToIssue(tester) {
  await tester.callTool('jit_init', {});

  // Define gates with unique names
  await tester.callTool('jit_gate_define', {
    key: 'add-multi-test-1',
    title: 'Multi 1',
    description: 'First multi gate'
  });
  
  await tester.callTool('jit_gate_define', {
    key: 'add-multi-test-2',
    title: 'Multi 2',
    description: 'Second multi gate'
  });
  
  // Create issue
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Issue for multiple gates'
  });
  
  // Add multiple gates
  await tester.callTool('jit_gate_add', {
    id: issue.id,
    gate_keys: ['add-multi-test-1', 'add-multi-test-2']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.gates_required?.includes('add-multi-test-1'), 'Should have multi1');
  assert(shown.gates_required?.includes('add-multi-test-2'), 'Should have multi2');
}

async function testGateRemoveFromIssue(tester) {
  await tester.callTool('jit_init', {});

  // Create issue with gate
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Issue for gate remove',
    gate: ['test-basic']
  });
  
  // Remove gate using issue update
  await tester.callTool('jit_issue_update', {
    id: issue.id,
    'remove-gate': ['test-basic']
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(!shown.gates_required?.includes('test-basic'), 'Should not have gate');
}

async function testGatePass(tester) {
  await tester.callTool('jit_init', {});

  // Create issue with gate
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Gate pass test',
    gate: ['test-basic']
  });
  
  // Pass gate
  await tester.callTool('jit_gate_pass', {
    id: issue.id,
    gate_key: 'test-basic'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.gates_status?.['test-basic']?.status === 'passed', 'Gate should be passed');
}

async function testGateFail(tester) {
  await tester.callTool('jit_init', {});

  // Define a gate
  await tester.callTool('jit_gate_define', {
    key: 'fail-gate',
    title: 'Fail Gate',
    description: 'Gate to fail'
  });
  
  // Create issue with gate
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Gate fail test',
    gate: ['fail-gate']
  });
  
  // Fail gate
  await tester.callTool('jit_gate_fail', {
    id: issue.id,
    gate_key: 'fail-gate'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.gates_status?.['fail-gate']?.status === 'failed', 'Gate should be failed');
}

async function testGatePassWithActor(tester) {
  await tester.callTool('jit_init', {});

  await tester.callTool('jit_init', {});

  // Define a gate
  await tester.callTool('jit_gate_define', {
    key: 'actor-gate',
    title: 'Actor Gate',
    description: 'Gate with actor'
  });
  
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Actor test',
    gate: ['actor-gate']
  });
  
  await tester.callTool('jit_gate_pass', {
    id: issue.id,
    gate_key: 'actor-gate',
    by: 'agent:test-worker'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  const gateStatus = shown.gates_status?.['actor-gate'];
  assert(gateStatus?.status === 'passed', 'Gate should be passed');
  assert(gateStatus?.updated_by === 'agent:test-worker', 'Should record actor');
}

async function testGateBlocksStateTransition(tester) {
  await tester.callTool('jit_init', {});

  // Create issue with gate
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Blocking test',
    gate: ['test-basic']
  });
  
  // Try to transition to done without passing gate
  const result = await tester.callTool('jit_issue_update', {
    id: issue.id,
    state: 'done'
  });
  
  // Should either fail or stay in gated state
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.state !== 'done', 'Should not transition to done with failing gate');
}

async function testGateAllowsTransitionWhenPassed(tester) {
  await tester.callTool('jit_init', {});

  // Create issue with gate
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Allow transition test',
    gate: ['test-basic']
  });
  
  // Pass gate
  await tester.callTool('jit_gate_pass', {
    id: issue.id,
    gate_key: 'test-basic'
  });
  
  // Try to transition to done
  await tester.callTool('jit_issue_update', {
    id: issue.id,
    state: 'done'
  });
  
  const shown = await tester.callTool('jit_issue_show', {
    id: issue.id
  });
  assert(shown.state === 'done', 'Should transition to done when gate passes');
}

async function testGateCheckManual(tester) {
  await tester.callTool('jit_init', {});

  // Manual gates can't be checked automatically, should indicate that
  const issue = await tester.callTool('jit_issue_create', {
    title: 'Check manual',
    gate: ['test-basic']
  });
  
  const result = await tester.callTool('jit_gate_check', {
    id: issue.id,
    gate_key: 'test-basic'
  });
  
  // Should return information (success or indicating manual gate)
  assert(result !== undefined, 'Should return result');
}

async function testGateRemoveFromRegistry(tester) {
  // Define a gate
  await tester.callTool('jit_gate_define', {
    key: 'removable-gate',
    title: 'Removable Gate',
    description: 'Gate to remove'
  });
  
  // Remove from registry
  await tester.callTool('jit_registry_remove', {
    key: 'removable-gate'
  });
  
  // Verify it's gone
  const list = await tester.callTool('jit_gate_list', {});
  const gate = list.gates?.find(g => g.key === 'removable-gate');
  assert(!gate, 'Gate should be removed from registry');
}

// Main test runner
async function main() {
  console.log('Testing Gate operations (Phase 2)...\n');
  
  const tester = new MCPTester();
  let passed = 0;
  let failed = 0;

  try {
    await tester.start();
    console.log('MCP server started\n');

    // Gate Definition tests
    if (await runTest('Gate define - basic', () => testGateDefineBasic(tester))) passed++; else failed++;
    if (await runTest('Gate define - with stage', () => testGateDefineWithStage(tester))) passed++; else failed++;
    if (await runTest('Gate define - with mode', () => testGateDefineWithMode(tester))) passed++; else failed++;
    
    // Gate List/Show tests
    if (await runTest('Gate list', () => testGateList(tester))) passed++; else failed++;
    if (await runTest('Gate show', () => testGateShow(tester))) passed++; else failed++;
    
    // Gate Add/Remove tests
    if (await runTest('Gate add to issue', () => testGateAddToIssue(tester))) passed++; else failed++;
    if (await runTest('Gate add multiple to issue', () => testGateAddMultipleToIssue(tester))) passed++; else failed++;
    if (await runTest('Gate remove from issue', () => testGateRemoveFromIssue(tester))) passed++; else failed++;
    
    // Gate Pass/Fail tests
    if (await runTest('Gate pass', () => testGatePass(tester))) passed++; else failed++;
    if (await runTest('Gate fail', () => testGateFail(tester))) passed++; else failed++;
    if (await runTest('Gate pass with actor', () => testGatePassWithActor(tester))) passed++; else failed++;
    
    // Gate Blocking tests
    if (await runTest('Gate blocks state transition', () => testGateBlocksStateTransition(tester))) passed++; else failed++;
    if (await runTest('Gate allows transition when passed', () => testGateAllowsTransitionWhenPassed(tester))) passed++; else failed++;
    
    // Gate Check tests
    if (await runTest('Gate check manual', () => testGateCheckManual(tester))) passed++; else failed++;
    
    // Gate Registry tests
    if (await runTest('Gate remove from registry', () => testGateRemoveFromRegistry(tester))) passed++; else failed++;

  } finally {
    await tester.stop();
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch(console.error);
