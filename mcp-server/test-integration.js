#!/usr/bin/env node
/**
 * Integration smoke tests for the MCP server
 *
 * Verifies the MCP protocol wiring end-to-end: request goes in via JSON-RPC,
 * hits the CLI, and a correctly-formatted response comes back. These tests
 * do NOT test CLI domain semantics (state machines, DAG cycles, gate blocking)
 * — that is covered by the Rust test suite.
 */

import { execFile, spawn } from 'child_process';
import { cpSync, mkdirSync, readFileSync, rmSync, writeFileSync } from 'fs';
import { tmpdir } from 'os';
import { join } from 'path';
import { fileURLToPath } from 'url';
import { promisify } from 'node:util';
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

// A type hierarchy is repository configuration too. It is declared as an inline
// table, so this guard reads it back through jit's own config parser rather
// than depending on how the declaration is spelled, and derives the type names
// from whatever the repository declares instead of naming this project's.
const declaredHierarchyTypes = Object.keys(
  JSON.parse(
    (await promisify(execFile)('jit', ['config', 'get', 'type_hierarchy.types', '--json'], {
      cwd: fileURLToPath(new URL('..', import.meta.url)),
      timeout: TIMEOUT,
    })).stdout
  ).value
);
// Prose names a type in either number, so each declared name is matched in its
// singular and plural forms, `y` → `ies` included.
const hierarchyTypeTerm = new RegExp(
  `\\b(?:${declaredHierarchyTypes
    .flatMap(type => [
      type,
      `${type}s`,
      ...(type.endsWith('y') ? [`${type.slice(0, -1)}ies`] : []),
    ])
    .map(escapeRegex)
    .join('|')})\\b`,
  'gi'
);

function namesConfiguredHierarchyTypes(text) {
  return [...new Set(
    [...text.matchAll(hierarchyTypeTerm)].map(([term]) => term.toLowerCase())
  )].sort();
}

// A profile package reaches jit as files in a repository; nothing about one
// lives inside the connected binary. These are the framings that claim otherwise.
const claimsBinaryEmbedding = /\b(?:embedded|embeds|compiled[- ]in|built[- ]in)\b|\bbinary\b/i;

// A description sources the package when it names the package and attributes it
// to the repository. Which words place it inside the repository are the text's
// own choice, so only the two concepts are required.
function sourcesPackageFromRepository(text) {
  return /\bpackages?\b/i.test(text) && /\brepositor(?:y|ies)\b/i.test(text);
}

// The checked-in profile-package fixture the profile tools are exercised over.
// A package is applied from inside the worktree it is applied to, so each case
// stages a copy of this tree in its own test repository.
const PROFILE_PACKAGE_FIXTURE = new URL(
  '../crates/jit/tests/fixtures/profile-packages/planner-asset-only',
  import.meta.url
);

/**
 * Stage a copy of the fixture package under `repo` at the repository-relative
 * `location`, rewritten to declare `id`, a dependency on each of
 * `dependencies`, and an asset target named for `id`.
 *
 * No package this repository authors declares a dependency, so a composition
 * scenario is authored here; renaming the asset target after the id is what
 * keeps two staged packages from publishing the same file.
 *
 * @returns {string} the repository-relative location used by a `path:` selector
 */
function stagePackage(repo, location, id, dependencies = []) {
  const root = join(repo, location);
  cpSync(PROFILE_PACKAGE_FIXTURE, root, { recursive: true });
  const manifestPath = join(root, 'manifest.toml');
  const declared = dependencies.map(dependency => `"${dependency}"`).join(', ');
  const manifest = readFileSync(manifestPath, 'utf8')
    .replace(/^id = ".*"$/m, `id = "${id}"`)
    .replace('[profile]', `dependencies = [${declared}]\n\n[profile]`)
    .replace(/^target = "docs\/.*"$/m, `target = "docs/${id}.txt"`);
  writeFileSync(manifestPath, manifest);
  return location;
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

      // Every connected agent reads the instruction block before it reads any
      // repository's declarations, so the block itself may name no type from a
      // hierarchy. A tool description is held instead to the command it
      // describes, whose own text names the work that command performs.
      assert.ok(declaredHierarchyTypes.length > 0,
        'this repository must declare a type hierarchy for the guard to derive names from');
      assert.deepStrictEqual(
        namesConfiguredHierarchyTypes(instructions), [],
        'SERVER_INSTRUCTIONS must name no type from a declared hierarchy'
      );

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
      // `profile` names an ordered selector; enumeration takes none, because it
      // follows the repository's own records.
      assert.deepStrictEqual(profileInputKeys, {
        jit_profile_apply: ['dry-run', 'json', 'profile', 'set', 'values-file'],
        jit_profile_list: ['json'],
        jit_profile_show: ['json', 'profile'],
      });
      assert.deepStrictEqual(
        tools.find(tool => tool.name === 'jit_profile_apply').inputSchema.required,
        ['profile']
      );
      assert.deepStrictEqual(
        tools.find(tool => tool.name === 'jit_profile_show').inputSchema.required,
        ['profile']
      );
      assert.deepStrictEqual(
        tools.find(tool => tool.name === 'jit_profile_list').inputSchema.required,
        []
      );
      const applyProperties = tools.find(tool => tool.name === 'jit_profile_apply').inputSchema.properties;
      assert.strictEqual(applyProperties['values-file'].type, 'string');
      assert.strictEqual(applyProperties.set.type, 'array');
      assert.strictEqual(applyProperties.set.items.type, 'string');
    });

    await runTest('profile descriptions source the package from the repository, never from the connected binary', async () => {
      const isProfileTool = name => name.startsWith('jit_profile_');
      const curated = Object.entries(CURATION.include)
        .filter(([name]) => isProfileTool(name))
        .map(([name, description]) => [`CURATION.include.${name}`, description]);
      const listed = (await tester.request('tools/list')).result.tools
        .filter(tool => isProfileTool(tool.name))
        .map(({ name, description }) => [`tools/list.${name}`, description]);

      assert.ok(curated.length > 0, 'profile tools should be curated in');
      assert.strictEqual(listed.length, curated.length,
        'every curated profile tool should be advertised');

      // No profile description, curated or advertised, may place a package
      // inside the binary an agent is connected to.
      for (const [source, description] of [...curated, ...listed]) {
        assert.doesNotMatch(description, claimsBinaryEmbedding,
          `${source} must not present a profile package as embedded in the binary`);
      }

      // Where the package is read from is stated by the curated descriptions,
      // which this manifest authors. An advertised description is the command's
      // own text from the CLI schema, so it is held only to the claim above.
      for (const [source, description] of curated) {
        assert.ok(sourcesPackageFromRepository(description),
          `${source} should state that the package is read from the repository`);
      }
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

        // An obtained set of packages, side by side inside the worktree: the
        // named one and the one it declares a dependency on.
        const baseLocation = stagePackage(profileTester.testDir, 'packages/base', 'base');
        const workflowLocation = stagePackage(
          profileTester.testDir, 'packages/workflow', 'workflow', ['base']);

        // Enumeration follows the repository's own applied-profile records, so
        // a repository that has applied nothing names no profile.
        const listed = await profileCall('jit_profile_list');
        assert.strictEqual(listed.count, 0);
        assert.deepStrictEqual(listed.profiles, []);

        const shown = await profileCall('jit_profile_show', {
          profile: [`path:${workflowLocation}`],
        });
        assert.strictEqual(shown.count, 1);
        assert.strictEqual(shown.profiles.length, shown.count);
        assert.strictEqual(shown.profiles[0].manifest.id, 'workflow');
        assert.deepStrictEqual(shown.profiles[0].origin, {
          source: 'directory',
          location: workflowLocation,
        });

        const repeatedShown = await profileCall('jit_profile_show', {
          profile: [`path:${workflowLocation}`, `path:${workflowLocation}`],
        });
        assert.strictEqual(repeatedShown.count, 2);
        assert.deepStrictEqual(
          repeatedShown.profiles.map(profile => profile.manifest.id),
          ['workflow', 'workflow']
        );

        // A preview is derived over one package against the repository in
        // front of it, so the package a repository declaring nothing can be
        // shown is the self-contained one.
        const basePreview = await profileCall('jit_profile_apply', {
          profile: [`path:${baseLocation}`],
          'dry-run': true,
        });
        assert.strictEqual(basePreview.count, 1);
        assert.strictEqual(basePreview.profiles.length, basePreview.count);
        assert.strictEqual(basePreview.profiles[0].status, 'would_apply');

        // An application reports one result per applied package: the packages
        // the named one depends on, then the named one.
        const applied = await profileCall('jit_profile_apply', {
          profile: [`path:${workflowLocation}`],
        });
        assert.strictEqual(applied.count, applied.profiles.length);
        const appliedProfileIds = applied.profiles.map(profile => profile.id);
        const expectedProfileIds = [
          ...shown.profiles[0].manifest.dependency.map(dependency => dependency.id),
          shown.profiles[0].manifest.id,
        ];
        assert.ok(shown.profiles[0].manifest.dependency.length > 0,
          'the named package declares a dependency');
        for (const id of expectedProfileIds) {
          assert.ok(appliedProfileIds.includes(id),
            `application should include declared package ${id}`);
        }
        assert.strictEqual(appliedProfileIds.at(-1), shown.profiles[0].manifest.id);
        assert.strictEqual(applied.profiles.at(-1).status, 'applied');

        // The applied package's own preview names the target it published.
        const preview = await profileCall('jit_profile_apply', {
          profile: ['id:workflow'],
          'dry-run': true,
        });
        assert.strictEqual(preview.count, 1);
        assert.ok(preview.profiles[0].targets.some(target => target.path === 'docs/workflow.txt'));

        const unchanged = await profileCall('jit_profile_apply', {
          profile: ['id:workflow'],
          'dry-run': true,
        });
        assert.strictEqual(unchanged.count, 1);
        assert.strictEqual(unchanged.profiles[0].status, 'unchanged');

        // The bridge preserves one repeated --profile occurrence stream,
        // including interleaved path and recorded-id selectors.
        const alphaLocation = stagePackage(profileTester.testDir, 'packages/alpha', 'alpha');
        const betaLocation = stagePackage(profileTester.testDir, 'packages/beta', 'beta');
        const seeded = await profileCall('jit_profile_apply', {
          profile: [`path:${alphaLocation}`, `path:${betaLocation}`],
        });
        const seededProfileIds = seeded.profiles.map(profile => profile.id);
        assert.deepStrictEqual(seededProfileIds, ['alpha', 'beta']);
        const orderedPreview = await profileCall('jit_profile_apply', {
          profile: [`path:${betaLocation}`, 'id:alpha', `path:${betaLocation}`],
          'dry-run': true,
        });
        assert.strictEqual(orderedPreview.count, 3);
        assert.deepStrictEqual(
          orderedPreview.profiles.map(profile => profile.id),
          ['beta', 'alpha', 'beta']
        );
        const ordered = await profileCall('jit_profile_apply', {
          profile: [`path:${alphaLocation}`, 'id:alpha', `path:${betaLocation}`],
        });
        const orderedProfileIds = ordered.profiles.map(profile => profile.id);
        assert.deepStrictEqual(orderedProfileIds, ['alpha', 'alpha', 'beta']);
        const allAppliedProfileIds = [
          ...appliedProfileIds,
          ...seededProfileIds,
          ...orderedProfileIds,
        ];

        // The records the application wrote are what the repository now names:
        // one per applied package, and no other.
        const recorded = await profileCall('jit_profile_list');
        assert.strictEqual(recorded.count, recorded.profiles.length);
        const expectedRecordedIds = [...new Set(allAppliedProfileIds)].sort();
        assert.deepStrictEqual(
          recorded.profiles.map(profile => profile.id),
          expectedRecordedIds,
          'recorded profiles should contain each applied package exactly once'
        );
        const appliedOrigins = new Map(await Promise.all(
          allAppliedProfileIds.map(async id => {
            const resolved = await profileCall('jit_profile_show', {
              profile: [`id:${id}`],
            });
            return [id, resolved.profiles[0].origin];
          })
        ));
        for (const profile of recorded.profiles) {
          assert.strictEqual(profile.applied, true);
          assert.deepStrictEqual(profile.origin, appliedOrigins.get(profile.id));
        }
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
