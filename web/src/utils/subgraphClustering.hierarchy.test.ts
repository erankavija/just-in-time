import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';
import { assignNodesToClusters } from './subgraphClustering';
import { resolveHierarchy, type HierarchyLevels } from './hierarchyResolution';
import type { GraphNode, GraphEdge } from '../types/models';

/**
 * REQ-03 (F1): the exercised clustering path is a pure projection of the
 * canonical resolution.
 *
 * `subgraphClustering` no longer traverses the graph itself — it derives cluster
 * membership from `hierarchyResolution` (the single source of containment facts,
 * itself vector-verified against the jit core). This test pins that projection on
 * the SAME shared fixture the core/web equivalence tests use: for every node,
 * the cluster `assignNodesToClusters` places it in must equal the container
 * reached by walking up the canonical parent chain to the nearest epic-level
 * ancestor. Any divergence between the presentation and the canonical facts fails
 * here.
 */

interface FixtureNode {
  id: string;
  type: string | null;
  dependencies: string[];
}

interface Fixture {
  hierarchy: HierarchyLevels;
  nodes: FixtureNode[];
}

const EPIC_LEVEL = 2;

function loadFixture(): Fixture {
  const path = resolve(process.cwd(), '..', 'test-vectors', 'hierarchy_resolution.json');
  return JSON.parse(readFileSync(path, 'utf-8')) as Fixture;
}

function toGraphNode(node: FixtureNode): GraphNode {
  return {
    id: node.id,
    label: node.id,
    state: 'backlog',
    priority: 'normal',
    blocked: false,
    labels: node.type ? [`type:${node.type}`] : [],
  };
}

describe('subgraphClustering is a projection of hierarchyResolution', () => {
  it('assigns every node to the container its canonical parent chain reaches', () => {
    const fixture = loadFixture();
    const graphNodes = fixture.nodes.map(toGraphNode);
    const edges: GraphEdge[] = fixture.nodes.flatMap((n) =>
      n.dependencies.map((to) => ({ from: n.id, to })),
    );

    // Independent derivation straight from the canonical facts.
    const resolution = resolveHierarchy(
      fixture.nodes.map((n) => ({ id: n.id, type: n.type, dependencies: n.dependencies })),
      fixture.hierarchy,
    );
    const levelOf = (id: string): number | undefined => {
      const type = fixture.nodes.find((n) => n.id === id)?.type;
      return type ? fixture.hierarchy[type] : undefined;
    };
    const derivedOwner = (id: string): string | null => {
      let cur: string | null = id;
      const seen = new Set<string>();
      while (cur !== null && !seen.has(cur)) {
        if (levelOf(cur) === EPIC_LEVEL) return cur;
        seen.add(cur);
        cur = resolution.get(cur)?.parent ?? null;
      }
      return null;
    };

    // Actual assignment produced by the clustering path.
    const clustered = assignNodesToClusters(graphNodes, edges, fixture.hierarchy, EPIC_LEVEL);
    const actualOwner = new Map<string, string | null>();
    for (const node of graphNodes) actualOwner.set(node.id, null);
    for (const cluster of clustered.clusters.values()) {
      for (const member of cluster.nodes) actualOwner.set(member.id, cluster.containerId);
    }

    for (const node of fixture.nodes) {
      expect(actualOwner.get(node.id), `cluster owner of ${node.id}`).toBe(derivedOwner(node.id));
    }

    // Spot-check the fixture's cross-cutting case: `tx` is a direct child of epic
    // `cx`, so it clusters to `cx` even though the deeper story `sy` reaches it
    // transitively via `ty → tx`.
    expect(actualOwner.get('tx')).toBe('cx');
  });
});
