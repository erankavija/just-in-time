#!/usr/bin/env node
/**
 * Integration smoke tests for the MCP server
 *
 * Verifies the MCP protocol wiring end-to-end: request goes in via JSON-RPC,
 * hits the CLI, and a correctly-formatted response comes back. These tests
 * do NOT test CLI domain semantics (state machines, DAG cycles, gate blocking)
 * — that is covered by the Rust test suite.
 */

import { spawn } from 'child_process';
import { mkdirSync, readFileSync, rmSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { strict as assert } from 'node:assert';
import { CURATION, getCommandByPath } from './lib/tool-generator.js';
import { loadSchema } from './lib/schema-loader.js';

const TIMEOUT = 5000;

// The CLI schema the server under test also loads, used to derive expectations
// instead of restating command shapes here.
const { schema } = await loadSchema();

// Item kinds are repository configuration, not MCP vocabulary. Read the
// repository's declaration so this guard never hardcodes this project's kinds.
const itemKindsConfig = readFileSync(new URL('../.jit/config.toml', import.meta.url), 'utf8');
const itemKinds = [...itemKindsConfig.matchAll(/^\[item_kinds\.([^\]]+)\]/gm)]
  .map(([, kind]) => kind);
const itemKindForms = new Map();
for (const kind of itemKinds) {
  for (const form of [kind, kind.endsWith('s') ? kind : `${kind}s`]) {
    itemKindForms.set(form.toLowerCase(), kind);
  }
}
const itemKindTerm = new RegExp(
  `\\b(?:${[...itemKindForms.keys()].map(escapeRegex).join('|')})\\b`, 'gi'
);
const itemKindEnumeration = new RegExp(
  `\\b(?:${[...itemKindForms.keys()].map(escapeRegex).join('|')})\\b\\s*(?:,|and)\\s*` +
  `\\b(?:${[...itemKindForms.keys()].map(escapeRegex).join('|')})\\b`, 'i'
);

function escapeRegex(value) {
  return value.replace(/[.*+?^${}()|[\]\\]/g, '\\$&');
}

function presentsConfiguredKindVocabulary(text) {
  const mentionedKinds = new Set(
    [...text.matchAll(itemKindTerm)].map(([term]) => itemKindForms.get(term.toLowerCase()))
  );
  return mentionedKinds.size >= 2 && itemKindEnumeration.test(text);
}

// ---------------------------------------------------------------------------
// MCPTester — spawns an isolated MCP server for testing
// ---------------------------------------------------------------------------

class MCPTester {
  constructor() {
    this.server = null;
    this.responseBuffer = '';
    this.pendingRequests = new Map();
    this.nextId = 1;
    this.testDir = null;
    this.stderr = '';
  }

  async start() {
    this.testDir = join(tmpdir(), `jit-mcp-integ-${Date.now()}`);
    mkdirSync(this.testDir, { recursive: true });

    const serverPath = join(process.cwd(), 'index.js');

    return new Promise((resolve, reject) => {
      this.server = spawn('node', [serverPath], {
        stdio: ['pipe', 'pipe', 'pipe'],
        cwd: this.testDir,
        env: { ...process.env, JIT_ALLOW_DELETION: '1' },
      });

      this.server.stdout.on('data', (data) => {
        this.responseBuffer += data.toString();
        this._processResponses();
      });

      this.server.stderr.on('data', (data) => { this.stderr += data.toString(); });
      this.server.on('error', reject);

      setTimeout(resolve, 500);
    });
  }

  _processResponses() {
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
      } catch {}
    }
  }

  async request(method, params = {}) {
    const id = this.nextId++;
    return new Promise((resolve, reject) => {
      const timeout = setTimeout(() => {
        this.pendingRequests.delete(id);
        reject(new Error(`Request ${id} timed out (${method})`));
      }, TIMEOUT);

      this.pendingRequests.set(id, {
        resolve: (response) => { clearTimeout(timeout); resolve(response); },
      });

      this.server.stdin.write(JSON.stringify({ jsonrpc: '2.0', id, method, params }) + '\n');
    });
  }

  /** Call a tool and return the raw MCP result (content array, isError flag). */
  async callToolRaw(toolName, args = {}) {
    const response = await this.request('tools/call', { name: toolName, arguments: args });
    if (response.error) throw new Error(response.error.message);
    return response.result;
  }

  /** Call a tool and extract the data (works in both content and structured modes). */
  async callTool(toolName, args = {}) {
    const result = await this.callToolRaw(toolName, args);

    // Structured mode: data in structuredContent
    if (result.structuredContent) {
      return result.structuredContent;
    }

    // Content mode: JSON is embedded in text (after summary line)
    if (result.content) {
      for (const item of result.content) {
        if (item.type !== 'text') continue;
        // Find the first JSON object/array in the text
        const jsonMatch = item.text.match(/(\{[\s\S]*\}|\[[\s\S]*\])\s*$/);
        if (jsonMatch) {
          try {
            const parsed = JSON.parse(jsonMatch[1]);
            if (parsed.success === false) throw new Error(parsed.error?.message || 'Command failed');
            return parsed;
          } catch (err) {
            if (!(err instanceof SyntaxError)) throw err;
          }
        }
      }
    }

    throw new Error('No data in response');
  }

  async stop() {
    if (this.server) { this.server.kill(); this.server = null; }
    if (this.testDir) {
      try { rmSync(this.testDir, { recursive: true, force: true }); } catch {}
    }
  }
}

// ---------------------------------------------------------------------------
// Test infrastructure
// ---------------------------------------------------------------------------

let passed = 0;
let failed = 0;

async function runTest(name, fn) {
  try {
    await fn();
    console.log(`  \u2713 ${name}`);
    passed++;
  } catch (err) {
    console.log(`  \u2717 ${name}`);
    console.error(`    ${err.message}`);
    failed++;
  }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

async function main() {
  console.log('\nMCP Server Integration Tests\n');

  const tester = new MCPTester();

  try {
    await tester.start();
    console.log('Server started\n');

    // -- Protocol handshake --------------------------------------------------

    console.log('Protocol');

    await runTest('startup connects the stdio transport and reports its tool counts', () => {
      // index.js writes the readiness banner only after server.connect(transport)
      // resolves, so the banner is evidence the SDK transport came up against a
      // schema loaded from the CLI.
      assert.ok(!tester.stderr.includes('Fatal error:'),
        `startup reported a fatal error: ${tester.stderr}`);
      assert.ok(/^Version: \S+$/m.test(tester.stderr),
        `startup should report the schema version: ${tester.stderr}`);
      const counts = tester.stderr.match(/Tools: (\d+) listed \((\d+) total\)/);
      assert.ok(counts, `startup should report tool counts: ${tester.stderr}`);
      const [, listed, total] = counts.map(Number);
      assert.strictEqual(listed, Object.keys(CURATION.include).length,
        'startup should list exactly the curated include set');
      assert.ok(total >= listed, 'total tool count should cover the listed set');
    });

    await runTest('initialize returns server info', async () => {
      const resp = await tester.request('initialize', {
        protocolVersion: '2024-11-05',
        capabilities: {},
        clientInfo: { name: 'test', version: '1.0.0' },
      });
      assert.ok(resp.result);
      assert.strictEqual(resp.result.serverInfo.name, 'jit-mcp-server');
      assert.ok(resp.result.protocolVersion);
    });

    await runTest('test_adopter_facing_text_defers_vocabulary_to_repository_configuration', async () => {
      const resp = await tester.request('initialize', {
        protocolVersion: '2024-11-05',
        capabilities: {},
        clientInfo: { name: 'vocabulary-test', version: '1.0.0' },
      });
      const instructions = resp.result.instructions;
      assert.match(instructions, /Gates are quality checkpoints.*repository's configuration/);
      assert.doesNotMatch(instructions, /\(tests, clippy, fmt, code-review\)/);

      const listed = await tester.request('tools/list');
      const descriptionSources = [
        ['SERVER_INSTRUCTIONS', instructions],
        ...Object.entries(CURATION.include)
          .map(([name, description]) => [`CURATION.include.${name}`, description]),
        ...listed.result.tools
          .map(({ name, description }) => [`tools/list.${name}`, description]),
      ];

      // These are the complete top-level tool-description inputs that define
      // the default client surface: server instructions, every curated
      // manifest entry, and every generated description returned by tools/list.
      // CURATION.policy and CURATION.exclude are not sent to MCP clients;
      // nested input-schema descriptions document parameters, not the tool.
      for (const [source, description] of descriptionSources) {
        assert.doesNotMatch(
          description,
          /@(?:[^/\s]+)?\/[^/\s]+\//,
          `${source} must not advertise a qualified item address or prefix`
        );
        assert.ok(
          !presentsConfiguredKindVocabulary(description),
          `${source} must not present configured item kinds as fixed vocabulary`
        );
      }
    });

    await runTest('tools/list returns curated default set with correct shape', async () => {
      const resp = await tester.request('tools/list');
      const tools = resp.result.tools;
      // tools/list advertises exactly the manifest include set, and that set
      // stays under the ceiling the manifest documents.
      const curatedNames = Object.keys(CURATION.include).sort();
      assert.deepStrictEqual(tools.map(t => t.name).sort(), curatedNames,
        'tools/list should advertise the curated-tools.json include set');
      assert.ok(tools.length <= CURATION.max_curated_tools,
        `curated set (${tools.length}) exceeds max_curated_tools (${CURATION.max_curated_tools})`);
      for (const tool of tools) {
        assert.ok(tool.name.startsWith('jit_'));
        assert.ok(tool.description);
        assert.strictEqual(tool.inputSchema.type, 'object');
      }
      // Core workflow tools must be present
      const names = new Set(tools.map(t => t.name));
      for (const required of [
        'jit_status',
        'jit_issue_create',
        'jit_issue_show',
        'jit_query_available',
        'jit_dep_add',
        'jit_gate_status-all',
        'jit_profile_list',
        'jit_profile_show',
        'jit_profile_apply',
      ]) {
        assert.ok(names.has(required), `missing core tool: ${required}`);
      }
      for (const deferred of [
        'jit_profile_install',
        'jit_profile_compose',
        'jit_profile_upgrade',
        'jit_profile_remove',
        'jit_profile_diff',
      ]) {
        assert.ok(!names.has(deferred), `deferred profile lifecycle tool leaked: ${deferred}`);
      }

      const profileInputKeys = Object.fromEntries(
        tools
          .filter(tool => tool.name.startsWith('jit_profile_'))
          .map(tool => [tool.name, Object.keys(tool.inputSchema.properties).sort()])
      );
      assert.deepStrictEqual(profileInputKeys, {
        jit_profile_apply: ['dry-run', 'id', 'json'],
        jit_profile_list: ['json'],
        jit_profile_show: ['id', 'json'],
      });
      assert.deepStrictEqual(
        tools.find(tool => tool.name === 'jit_profile_apply').inputSchema.required,
        ['id']
      );
      assert.deepStrictEqual(
        tools.find(tool => tool.name === 'jit_profile_show').inputSchema.required,
        ['id']
      );
      assert.deepStrictEqual(
        tools.find(tool => tool.name === 'jit_profile_list').inputSchema.required,
        []
      );
    });

    // -- Error handling ------------------------------------------------------

    console.log('\nError handling');

    await runTest('unknown tool returns isError with UNKNOWN_TOOL code', async () => {
      const result = await tester.callToolRaw('jit_does_not_exist', {});
      assert.ok(result.isError);
      const parsed = JSON.parse(result.content[0].text);
      assert.strictEqual(parsed.success, false);
      assert.strictEqual(parsed.error.code, 'UNKNOWN_TOOL');
    });

    await runTest('missing required args returns VALIDATION_ERROR before CLI', async () => {
      // Initialize repo first
      await tester.callToolRaw('jit_init', {});

      const result = await tester.callToolRaw('jit_doc_assets_list', {});
      assert.ok(result.isError);
      const parsed = JSON.parse(result.content[0].text);
      assert.strictEqual(parsed.error.code, 'VALIDATION_ERROR');
      // The rejection names the argument the client omitted; the wording of the
      // rest of the message belongs to the validation library.
      const required = getCommandByPath(schema, ['doc', 'assets', 'list']).args
        .filter(arg => arg.required)
        .map(arg => arg.name);
      assert.ok(required.length > 0, 'fixture command should declare a required argument');
      for (const name of required) {
        assert.ok(parsed.error.message.includes(name),
          `validation error should name '${name}': ${parsed.error.message}`);
      }
    });

    await runTest('CLI error returns structured error envelope', async () => {
      await tester.callToolRaw('jit_init', {});
      const result = await tester.callToolRaw('jit_issue_show', { ids: ['NONEXISTENT_999'] });
      assert.ok(result.isError);
      const parsed = JSON.parse(result.content[0].text);
      assert.strictEqual(parsed.success, false);
      assert.ok(parsed.error.code);
      assert.ok(parsed.error.message);
    });

    // -- Response formatting -------------------------------------------------

    console.log('\nResponse formatting');

    await runTest('init returns the Git-attributes disposition', async () => {
      const initialized = await tester.callTool('jit_init', {});
      assert.strictEqual(initialized.gitattributes_status, 'not_applicable');
    });

    await runTest('default mode: content has summary + JSON, no structuredContent', async () => {
      await tester.callToolRaw('jit_init', {});
      await tester.callTool('jit_issue_create', { title: 'Content mode test' });

      const result = await tester.callToolRaw('jit_status', {});
      assert.ok(!result.isError);

      // Single content block with summary + serialized JSON
      assert.ok(result.content.length >= 1, 'should have content');
      const text = result.content[0].text;
      assert.ok(text.includes('open') || text.includes('ready') || text.includes('done'),
        'content should include human summary');
      assert.ok(text.includes('"total"'), 'content should include serialized JSON');

      // No structuredContent in default mode
      assert.strictEqual(result.structuredContent, undefined,
        'should not have structuredContent in content mode');
    });

    await runTest('default mode: tools do not declare outputSchema', async () => {
      const resp = await tester.request('tools/list');
      const tools = resp.result.tools;
      const withSchema = tools.filter(t => t.outputSchema);
      assert.strictEqual(withSchema.length, 0,
        `in content mode, tools should not have outputSchema, but ${withSchema.length} do`);
    });

    // -- CRUD round-trip (arg mapping + response parsing) --------------------

    console.log('\nCRUD round-trip');

    await runTest('create and show issue verifies arg mapping and response parsing', async () => {
      await tester.callToolRaw('jit_init', {});

      const created = await tester.callTool('jit_issue_create', {
        title: 'Integration test issue',
        priority: 'high',
        label: ['type:task'],
      });
      assert.ok(created.id, 'create should return id');
      assert.strictEqual(created.title, 'Integration test issue');
      assert.strictEqual(created.priority, 'high');

      const shown = await tester.callTool('jit_issue_show', { ids: [created.id] });
      assert.strictEqual(shown.id, created.id);
      assert.strictEqual(shown.title, 'Integration test issue');
      assert.ok(shown.labels.includes('type:task'));
    });

    await runTest('profile tools list, inspect, dry-run, apply, and reach exact no-op', async () => {
      const profileTester = new MCPTester();
      await profileTester.start();
      const profileCall = async (name, args = {}) => {
        try {
          return await profileTester.callTool(name, args);
        } catch (err) {
          throw new Error(`${name}: ${err.message}`);
        }
      };

      try {
        await profileTester.callToolRaw('jit_init', {});

        const listed = await profileCall('jit_profile_list');
        assert.strictEqual(listed.count, 1);
        assert.strictEqual(listed.profiles[0].id, 'jit-dogfood');
        assert.strictEqual(listed.profiles[0].applied, false);

        const shown = await profileCall('jit_profile_show', { id: 'jit-dogfood' });
        assert.strictEqual(shown.manifest.profile.id, 'jit-dogfood');
        const liveSources = shown.manifest['live-source'];
        assert.ok(Array.isArray(liveSources) && liveSources.length > 0,
          'the reported manifest declares live-source roots');
        const roots = liveSources.map(source => source && source.root);
        assert.ok(roots.every(root => typeof root === 'string' && root.length > 0),
          'every reported live-source declaration names a root');
        const liveAssets = shown.manifest.asset.filter(asset =>
          asset.source.startsWith('assets/live/'));
        assert.ok(liveAssets.length > 0, 'the reported manifest declares live assets');
        const targetUnderRoot = (target, root) =>
          target === root || target.startsWith(`${root}/`);
        assert.ok(liveAssets.every(asset => roots.some(root =>
          targetUnderRoot(asset.target, root))),
        'every reported live asset target is accounted for by a reported root');
        assert.ok(roots.every(root => liveAssets.some(asset =>
          targetUnderRoot(asset.target, root))),
        'every reported root accounts for a reported live asset target');
        assert.deepStrictEqual(shown.origin, { source: 'embedded' });

        const preview = await profileCall('jit_profile_apply', {
          id: 'jit-dogfood',
          'dry-run': true,
        });
        assert.strictEqual(preview.status, 'would_apply');
        assert.ok(preview.targets.some(target =>
          target.path === 'contrib/gates/ai-review.sh' && target.executable === true));

        const applied = await profileCall('jit_profile_apply', { id: 'jit-dogfood' });
        assert.strictEqual(applied.status, 'applied');

        const unchanged = await profileCall('jit_profile_apply', {
          id: 'jit-dogfood',
          'dry-run': true,
        });
        assert.strictEqual(unchanged.status, 'unchanged');
      } finally {
        await profileTester.stop();
      }
    });

  } finally {
    await tester.stop();
  }

  console.log(`\n${passed} passed, ${failed} failed`);
  process.exit(failed > 0 ? 1 : 0);
}

main().catch(err => {
  console.error('Test suite failed:', err);
  process.exit(1);
});
