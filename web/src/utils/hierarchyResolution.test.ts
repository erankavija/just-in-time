import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { resolveHierarchy, type HierarchyInputNode, type HierarchyLevels } from './hierarchyResolution';

/**
 * REQ-03 shared-test-vector equivalence (web side).
 *
 * Loads the SAME fixture the Rust core test reads
 * (`crates/jit/tests/hierarchy_vectors_test.rs`) and asserts the TypeScript port
 * reproduces the committed `expected` output. A passing pair proves the web and
 * core resolvers agree on the canonical DAG-authoritative resolution.
 */

interface ExpectedNode {
  parent: string | null;
  children: string[];
  cluster: string | null;
  rank: number;
}

interface Fixture {
  hierarchy: HierarchyLevels;
  nodes: HierarchyInputNode[];
  expected: Record<string, ExpectedNode>;
}

function loadFixture(): Fixture {
  // Vitest runs with cwd at the web/ package root; the shared fixture lives at
  // the repository root under test-vectors/.
  const path = resolve(process.cwd(), '..', 'test-vectors', 'hierarchy_resolution.json');
  return JSON.parse(readFileSync(path, 'utf-8')) as Fixture;
}

describe('resolveHierarchy shared vectors', () => {
  it('reproduces the committed core resolution', () => {
    const fixture = loadFixture();
    const resolution = resolveHierarchy(fixture.nodes, fixture.hierarchy);

    expect(resolution.size).toBe(Object.keys(fixture.expected).length);

    for (const [id, expected] of Object.entries(fixture.expected)) {
      const actual = resolution.get(id);
      expect(actual, `missing node ${id}`).toBeDefined();
      expect(actual, `resolution mismatch for node ${id}`).toEqual(expected);
    }
  });
});
