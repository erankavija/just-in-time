#!/usr/bin/env node
/**
 * Test suite for label-related MCP tools (Phase 4)
 * 
 * Tests:
 * - query_by_label
 * - query_strategic
 * - update_issue_labels (add/remove labels)
 * - list_label_namespaces
 * - list_label_values
 */

import { spawn } from 'child_process';
import { readFileSync, mkdirSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { execSync } from 'child_process';

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
    this.testDir = join(tmpdir(), `jit-mcp-label-test-${Date.now()}`);
    mkdirSync(this.testDir, { recursive: true });
    
    // Initialize jit repo in test directory
    execSync('jit init', { cwd: this.testDir });
    
    const serverPath = join(process.cwd(), 'index.js');
    
    return new Promise((resolve, reject) => {
      this.server = spawn('node', [serverPath], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: this.testDir
      });

      this.server.stdout.on('data', (data) => {
        this.responseBuffer += data.toString();
        this.processResponses();
      });

      this.server.stderr.on('data', (data) => {
        // Ignore startup messages
      });

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
        console.error('Parse error:', err.message);
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
        reject(new Error(`Request ${id} timed out`));
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

    // Extract assistant-targeted content (dual-audience response)
    const contents = response.result?.content || [];
    if (contents.length === 0) {
      throw new Error('No content in response');
    }
    
    // Find content for assistant (or last JSON-parseable content)
    let jsonContent = null;
    for (const item of contents.reverse()) {
      try {
        const parsed = JSON.parse(item.text);
        jsonContent = parsed;
        break;
      } catch {
        // Try next item
      }
    }
    
    if (!jsonContent) {
      throw new Error('No parseable JSON content found');
    }
    
    return jsonContent;
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

// Test utilities
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
    if (error.stack) {
      console.error(error.stack.split('\n').slice(1, 4).join('\n'));
    }
    return false;
  }
}

// Tests
async function testQueryByLabel(tester) {
  // Create issues with labels
  await tester.callTool('jit_issue_create', {
    title: 'Milestone task',
    desc: 'Part of v1.0',
    label: ['milestone:v1.0', 'type:task']
  });

  await tester.callTool('jit_issue_create', {
    title: 'Epic task',
    desc: 'Part of auth',
    label: ['epic:auth', 'component:backend']
  });

  await tester.callTool('jit_issue_create', {
    title: 'Bug fix',
    desc: 'Fix login',
    label: ['type:bug']
  });

  // Query by exact label
  const result1 = await tester.callTool('jit_query_label', {
    pattern: 'milestone:v1.0'
  });
  assert(result1.issues.length === 1, 'Should find 1 milestone issue');
  assert(result1.issues[0].title === 'Milestone task', 'Should be milestone task');

  // Query by wildcard
  const result2 = await tester.callTool('jit_query_label', {
    pattern: 'type:*'
  });
  assert(result2.issues.length === 2, 'Should find 2 type-labeled issues');

  // Query with no matches
  const result3 = await tester.callTool('jit_query_label', {
    pattern: 'milestone:v2.0'
  });
  assert(result3.issues.length === 0, 'Should find no v2.0 issues');
}

async function testQueryStrategic(tester) {
  // Create mix of strategic and tactical issues
  const milestone = await tester.callTool('jit_issue_create', {
    title: 'Release v1.0',
    label: ['milestone:v1.0']
  });

  const epic = await tester.callTool('jit_issue_create', {
    title: 'Auth System',
    label: ['epic:auth']
  });

  await tester.callTool('jit_issue_create', {
    title: 'Fix typo',
    label: ['type:bug']
  });

  // Query strategic issues (should include issues from previous tests too)
  const result = await tester.callTool('jit_query_strategic', {});
  
  // Should find at least the 2 we just created (plus any from previous tests)
  assert(result.issues.length >= 2, `Should find at least 2 strategic issues, found ${result.issues.length}`);
  
  const ids = result.issues.map(i => i.id);
  assert(ids.includes(milestone.id), 'Should include milestone');
  assert(ids.includes(epic.id), 'Should include epic');
}

async function testListLabelNamespaces(tester) {
  const result = await tester.callTool('jit_label_namespaces', {});
  
  assert(result.namespaces, 'Should have namespaces object');
  assert(result.namespaces.milestone, 'Should have milestone namespace');
  assert(result.namespaces.epic, 'Should have epic namespace');
  assert(result.namespaces.type, 'Should have type namespace');
  
  // Check properties
  assert(result.namespaces.milestone.strategic === true, 'Milestone should be strategic');
  assert(result.namespaces.type.unique === true, 'Type should be unique');
  assert(result.namespaces.component.unique === false, 'Component should not be unique');
}

async function testListLabelValues(tester) {
  // Create issues with various milestones
  await tester.callTool('jit_issue_create', {
    title: 'Task 1',
    label: ['milestone:v1.0']
  });

  await tester.callTool('jit_issue_create', {
    title: 'Task 2',
    label: ['milestone:v2.0']
  });

  await tester.callTool('jit_issue_create', {
    title: 'Task 3',
    label: ['milestone:v1.0']  // Duplicate
  });

  // List milestone values
  const result = await tester.callTool('jit_label_values', {
    namespace: 'milestone'
  });

  assert(result.values.length === 2, 'Should have 2 unique values');
  assert(result.values.includes('v1.0'), 'Should include v1.0');
  assert(result.values.includes('v2.0'), 'Should include v2.0');
}

async function testAddCustomNamespace(tester) {
  // Add custom strategic namespace
  await tester.callTool('jit_label_add_namespace', {
    name: 'initiative',
    description: 'Company-wide initiatives',
    unique: false,
    strategic: true
  });

  // Verify it was added
  const namespaces = await tester.callTool('jit_label_namespaces', {});
  assert(namespaces.namespaces.initiative, 'Should have initiative namespace');
  assert(namespaces.namespaces.initiative.strategic === true, 'Initiative should be strategic');

  // Create issue with custom namespace
  await tester.callTool('jit_issue_create', {
    title: 'Cloud Migration',
    label: ['initiative:cloud']
  });

  // Verify it appears in strategic query
  const strategic = await tester.callTool('jit_query_strategic', {});
  const hasInitiative = strategic.issues.some(i => i.title === 'Cloud Migration');
  assert(hasInitiative, 'Initiative issue should appear in strategic query');
}

// Main test runner
async function main() {
  console.log('Testing label MCP tools...\n');
  
  const tester = new MCPTester();
  let passed = 0;
  let failed = 0;

  try {
    await tester.start();
    console.log('MCP server started\n');

    // Run tests
    if (await runTest('Query by label (exact match)', () => testQueryByLabel(tester))) passed++; else failed++;
    if (await runTest('Query strategic issues', () => testQueryStrategic(tester))) passed++; else failed++;
    if (await runTest('List label namespaces', () => testListLabelNamespaces(tester))) passed++; else failed++;
    if (await runTest('List label values in namespace', () => testListLabelValues(tester))) passed++; else failed++;
    if (await runTest('Add custom strategic namespace', () => testAddCustomNamespace(tester))) passed++; else failed++;

  } finally {
    await tester.stop();
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch(console.error);
