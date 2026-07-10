/**
 * Integration tests for clusteredGraphLayout using realistic repository data.
 *
 * These tests use structures similar to actual epics in the jit repository
 * to verify the clustering algorithm works correctly with real-world complexity.
 *
 * Every node carries the resolution the server ships for that graph
 * (`type`/`parent`/`children`/`cluster`/`rank`), stated literally.
 */

import { describe, it, expect } from 'vitest';
import { graphNode } from '../test/graphNode';
import type { GraphNode, GraphEdge } from '../types/models';
import type { HierarchyLevelMap } from '../types/subgraphCluster';
import { prepareClusteredGraphForReactFlow } from './clusteredGraphLayout';

describe('clusteredGraphLayout - Integration Tests', () => {
  // Helper to create edges
  const edge = (from: string, to: string): GraphEdge => ({ from, to });

  // Standard hierarchy from jit config
  const hierarchy: HierarchyLevelMap = {
    milestone: 1,
    epic: 2,
    story: 3,
    task: 4,
    bug: 4,
  };

  it('should cluster complex epic with multiple stories and cross-epic dependencies', () => {
    /**
     * Simulates Epic ad601a15 "Enable parallel multi-agent work with git worktrees"
     * which has 55 transitive dependencies across 3 stories with many tasks.
     *
     * Structure:
     * - Epic E1 (parallel-work)
     *   - Story S1 (recovery)
     *     - Task T1, T2, T3
     *   - Story S2 (claims)
     *     - Task T4, T5, T6
     *   - Story S3 (worktree-foundation)
     *     - Task T7, T8
     * - Epic E2 (other epic) with Task T9
     *
     * Cross-epic dependency: T4 → T9 (Epic E1's task depends on Epic E2's task)
     */

    const nodes: GraphNode[] = [
      graphNode('E1', { type: 'epic', children: ['S1', 'S2', 'S3'], cluster: 'E1', rank: 4 }),
      graphNode('S1', { type: 'story', parent: 'E1', children: ['T1', 'T2', 'T3'], cluster: 'E1', rank: 2 }),
      graphNode('S2', { type: 'story', parent: 'E1', children: ['T4', 'T5', 'T6'], cluster: 'E1', rank: 3 }),
      graphNode('S3', { type: 'story', parent: 'E1', children: ['T7', 'T8'], cluster: 'E1', rank: 1 }),
      graphNode('T1', { type: 'task', parent: 'S1', cluster: 'E1', rank: 0 }),
      graphNode('T2', { type: 'task', parent: 'S1', cluster: 'E1', rank: 1 }),
      graphNode('T3', { type: 'task', parent: 'S1', cluster: 'E1', rank: 0 }),
      graphNode('T4', { type: 'task', parent: 'S2', cluster: 'E1', rank: 1 }),
      graphNode('T5', { type: 'task', parent: 'S2', cluster: 'E1', rank: 2 }),
      graphNode('T6', { type: 'task', parent: 'S2', cluster: 'E1', rank: 0 }),
      graphNode('T7', { type: 'task', parent: 'S3', cluster: 'E1', rank: 0 }),
      graphNode('T8', { type: 'task', parent: 'S3', cluster: 'E1', rank: 0 }),
      graphNode('E2', { type: 'epic', children: ['T9'], cluster: 'E2', rank: 1 }),
      graphNode('T9', { type: 'task', parent: 'E2', cluster: 'E2', rank: 0 }),
    ];

    const edges: GraphEdge[] = [
      // Epic E1 depends on stories (container edges)
      edge('E1', 'S1'),
      edge('E1', 'S2'),
      edge('E1', 'S3'),
      // Stories depend on tasks
      edge('S1', 'T1'),
      edge('S1', 'T2'),
      edge('S1', 'T3'),
      edge('S2', 'T4'),
      edge('S2', 'T5'),
      edge('S2', 'T6'),
      edge('S3', 'T7'),
      edge('S3', 'T8'),
      // Epic E2 structure
      edge('E2', 'T9'),
      // Cross-epic dependency (realistic: claims task depends on another epic's work)
      edge('T4', 'T9'),
      // Internal dependencies
      edge('T2', 'T1'),
      edge('T5', 'T4'),
    ];

    const result = prepareClusteredGraphForReactFlow(
      nodes,
      edges,
      hierarchy,
      { E1: true, E2: true, S1: true, S2: true, S3: true } // Expand all to see sub-clusters
    );

    // Should create 2 primary clusters + 3 sub-clusters (one per story)
    // Primary: E1, E2
    // Sub: S1, S2, S3 (stories within E1)
    expect(result.clusters).toHaveLength(5);

    // Check we have the expected cluster container IDs
    const clusterIds = result.clusters.map(c => c.containerId).sort();
    expect(clusterIds).toEqual(['E1', 'E2', 'S1', 'S2', 'S3']);

    // Epic E1 cluster should contain all its children
    const e1Cluster = result.clusters.find(c => c.containerId === 'E1');
    expect(e1Cluster).toBeDefined();
    expect(e1Cluster!.nodes).toHaveLength(12); // E1 + 3 stories + 8 tasks

    // Epic E2 cluster should contain only T9 (no stories)
    const e2Cluster = result.clusters.find(c => c.containerId === 'E2');
    expect(e2Cluster).toBeDefined();
    expect(e2Cluster!.nodes).toHaveLength(2); // E2 + T9

    // Cross-cluster edge T4→T9 should be preserved
    expect(result.crossClusterEdges).toContainEqual(edge('T4', 'T9'));

    // All nodes should be visible (nothing collapsed)
    expect(result.visibleNodes).toHaveLength(14);

    // All edges between visible nodes should be present
    expect(result.visibleEdges.length).toBeGreaterThan(10);

    // No virtual edges (nothing collapsed)
    expect(result.virtualEdges).toHaveLength(0);
  });

  it('should aggregate edges when story is collapsed', () => {
    /**
     * When Story S1 is collapsed, edges from its tasks should bubble up to S1.
     *
     * Before collapse:
     *   Epic E1 → Story S1 → Task T1 → Task T9 (in Epic E2)
     *                      → Task T2
     *
     * After collapse:
     *   Epic E1 → Story S1 ⊞ → Task T9 (in Epic E2)
     *   (T1, T2 hidden; edge T1→T9 becomes S1→T9)
     */

    const nodes: GraphNode[] = [
      graphNode('E1', { type: 'epic', children: ['S1'], cluster: 'E1', rank: 3 }),
      graphNode('S1', { type: 'story', parent: 'E1', children: ['T1', 'T2'], cluster: 'E1', rank: 2 }),
      graphNode('T1', { type: 'task', parent: 'S1', cluster: 'E1', rank: 1 }),
      graphNode('T2', { type: 'task', parent: 'S1', cluster: 'E1', rank: 0 }),
      graphNode('E2', { type: 'epic', children: ['T9'], cluster: 'E2', rank: 1 }),
      graphNode('T9', { type: 'task', parent: 'E2', cluster: 'E2', rank: 0 }),
    ];

    const edges: GraphEdge[] = [
      edge('E1', 'S1'),
      edge('S1', 'T1'),
      edge('S1', 'T2'),
      edge('T1', 'T9'), // Cross-cluster
      edge('E2', 'T9'),
    ];

    const result = prepareClusteredGraphForReactFlow(
      nodes,
      edges,
      hierarchy,
      { S1: false, E1: true, E2: true } // E1 and E2 expanded, S1 collapsed
    );

    // T1 and T2 should be hidden
    expect(result.visibleNodes).toHaveLength(4); // E1, S1, E2, T9
    expect(result.visibleNodes.map(n => n.id)).toContain('S1');
    expect(result.visibleNodes.map(n => n.id)).not.toContain('T1');
    expect(result.visibleNodes.map(n => n.id)).not.toContain('T2');

    // Should create virtual edge S1→T9 (aggregated from T1→T9)
    expect(result.virtualEdges).toHaveLength(1);
    expect(result.virtualEdges[0]).toMatchObject({
      from: 'S1',
      to: 'T9',
      count: 1,
    });
    expect(result.virtualEdges[0].sourceEdgeIds).toContain('T1→T9');
  });

  it('should handle nested collapse (story inside epic)', () => {
    /**
     * When Epic E1 is collapsed, ALL its children (stories + tasks) should be hidden.
     * Edges from hidden tasks should bubble all the way up to E1.
     *
     * Before collapse:
     *   Epic E1 → Story S1 → Task T1 → Task T9 (in Epic E2)
     *
     * After collapse:
     *   Epic E1 ⊞ → Task T9 (in Epic E2)
     *   (S1, T1 hidden; edge T1→T9 becomes E1→T9)
     */

    const nodes: GraphNode[] = [
      graphNode('E1', { type: 'epic', children: ['S1'], cluster: 'E1', rank: 3 }),
      graphNode('S1', { type: 'story', parent: 'E1', children: ['T1'], cluster: 'E1', rank: 2 }),
      graphNode('T1', { type: 'task', parent: 'S1', cluster: 'E1', rank: 1 }),
      graphNode('E2', { type: 'epic', children: ['T9'], cluster: 'E2', rank: 1 }),
      graphNode('T9', { type: 'task', parent: 'E2', cluster: 'E2', rank: 0 }),
    ];

    const edges: GraphEdge[] = [
      edge('E1', 'S1'),
      edge('S1', 'T1'),
      edge('T1', 'T9'),
      edge('E2', 'T9'),
    ];

    const result = prepareClusteredGraphForReactFlow(
      nodes,
      edges,
      hierarchy,
      { E1: false, E2: true } // E1 collapsed, E2 expanded
    );

    // Only E1, E2, T9 should be visible (S1 and T1 hidden)
    expect(result.visibleNodes).toHaveLength(3);
    expect(result.visibleNodes.map(n => n.id)).toEqual(
      expect.arrayContaining(['E1', 'E2', 'T9'])
    );

    // Virtual edge E1→T9 (aggregated from T1→T9, bubbled up through S1)
    expect(result.virtualEdges).toHaveLength(1);
    expect(result.virtualEdges[0]).toMatchObject({
      from: 'E1',
      to: 'T9',
      count: 1,
    });
  });

  it('should handle milestone → epic → story → task hierarchy', () => {
    /**
     * Full 4-level hierarchy as configured in jit.
     *
     * Milestone M1 → Epic E1 → Story S1 → Task T1
     *                                    → Task T2
     *             → Epic E2 → Story S2 → Task T3
     */

    const nodes: GraphNode[] = [
      graphNode('M1', { type: 'milestone', children: ['E1', 'E2'], cluster: 'M1', rank: 3 }),
      graphNode('E1', { type: 'epic', parent: 'M1', children: ['S1'], cluster: 'M1', rank: 2 }),
      graphNode('E2', { type: 'epic', parent: 'M1', children: ['S2'], cluster: 'M1', rank: 2 }),
      graphNode('S1', { type: 'story', parent: 'E1', children: ['T1', 'T2'], cluster: 'M1', rank: 1 }),
      graphNode('S2', { type: 'story', parent: 'E2', children: ['T3'], cluster: 'M1', rank: 1 }),
      graphNode('T1', { type: 'task', parent: 'S1', cluster: 'M1', rank: 0 }),
      graphNode('T2', { type: 'task', parent: 'S1', cluster: 'M1', rank: 0 }),
      graphNode('T3', { type: 'task', parent: 'S2', cluster: 'M1', rank: 0 }),
    ];

    const edges: GraphEdge[] = [
      edge('M1', 'E1'),
      edge('M1', 'E2'),
      edge('E1', 'S1'),
      edge('S1', 'T1'),
      edge('S1', 'T2'),
      edge('E2', 'S2'),
      edge('S2', 'T3'),
    ];

    const result = prepareClusteredGraphForReactFlow(
      nodes,
      edges,
      hierarchy,
      { E1: true, E2: true, S1: true, S2: true } // Expand epics and stories
    );

    // Should create 2 primary clusters + 2 sub-clusters (S1, S2)
    // Primary: E1, E2
    // Sub: S1 (in E1), S2 (in E2)
    expect(result.clusters.length).toBe(4);

    // Epic E1 cluster should contain its stories and tasks
    const e1Cluster = result.clusters.find(c => c.containerId === 'E1');
    expect(e1Cluster).toBeDefined();
    expect(e1Cluster!.nodes.map(n => n.id)).toContain('E1');
    expect(e1Cluster!.nodes.map(n => n.id)).toContain('S1');
    expect(e1Cluster!.nodes.map(n => n.id)).toContain('T1');
    expect(e1Cluster!.nodes.map(n => n.id)).toContain('T2');

    // Epic E2 cluster should contain its stories and tasks
    const e2Cluster = result.clusters.find(c => c.containerId === 'E2');
    expect(e2Cluster).toBeDefined();
    expect(e2Cluster!.nodes.map(n => n.id)).toContain('E2');
    expect(e2Cluster!.nodes.map(n => n.id)).toContain('S2');
    expect(e2Cluster!.nodes.map(n => n.id)).toContain('T3');

    // Milestone M1 should be an orphan node (not in any cluster)
    expect(result.orphanNodes.map(n => n.id)).toContain('M1');

    // All nodes visible
    expect(result.visibleNodes).toHaveLength(8);
  });

  it('should preserve multiple cross-cluster edges', () => {
    /**
     * Multiple tasks in Epic E1 depend on tasks in Epic E2.
     * All cross-cluster edges should be tracked.
     */

    const nodes: GraphNode[] = [
      graphNode('E1', { type: 'epic', children: ['T1', 'T2'], cluster: 'E1', rank: 2 }),
      graphNode('T1', { type: 'task', parent: 'E1', cluster: 'E1', rank: 1 }),
      graphNode('T2', { type: 'task', parent: 'E1', cluster: 'E1', rank: 1 }),
      graphNode('E2', { type: 'epic', children: ['T3', 'T4'], cluster: 'E2', rank: 1 }),
      graphNode('T3', { type: 'task', parent: 'E2', cluster: 'E2', rank: 0 }),
      graphNode('T4', { type: 'task', parent: 'E2', cluster: 'E2', rank: 0 }),
    ];

    const edges: GraphEdge[] = [
      edge('E1', 'T1'),
      edge('E1', 'T2'),
      edge('E2', 'T3'),
      edge('E2', 'T4'),
      // Cross-cluster dependencies
      edge('T1', 'T3'),
      edge('T1', 'T4'),
      edge('T2', 'T3'),
    ];

    const result = prepareClusteredGraphForReactFlow(
      nodes,
      edges,
      hierarchy,
      {}
    );

    // Should have 3 cross-cluster edges
    expect(result.crossClusterEdges).toHaveLength(3);
    expect(result.crossClusterEdges).toEqual(
      expect.arrayContaining([
        edge('T1', 'T3'),
        edge('T1', 'T4'),
        edge('T2', 'T3'),
      ])
    );
  });

  it('should handle orphan nodes (no epic parent)', () => {
    /**
     * Task T2 has no epic parent (orphaned).
     * Should be tracked separately in orphanNodes.
     */

    const nodes: GraphNode[] = [
      graphNode('E1', { type: 'epic', children: ['T1'], cluster: 'E1', rank: 1 }),
      graphNode('T1', { type: 'task', parent: 'E1', cluster: 'E1', rank: 0 }),
      graphNode('T2', { type: 'task', rank: 0 }), // Orphan: no parent, no cluster
    ];

    const edges: GraphEdge[] = [
      edge('E1', 'T1'),
      // T2 has no incoming edges from any epic/story
    ];

    const result = prepareClusteredGraphForReactFlow(
      nodes,
      edges,
      hierarchy,
      {}
    );

    // T2 should be in orphanNodes
    expect(result.orphanNodes).toHaveLength(1);
    expect(result.orphanNodes[0].id).toBe('T2');

    // T2 should still be in visibleNodes
    expect(result.visibleNodes.map(n => n.id)).toContain('T2');
  });
});
