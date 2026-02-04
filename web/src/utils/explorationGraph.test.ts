import { describe, it, expect } from 'vitest';
import { createHierarchyConfig } from '../types/hierarchy';
import type { GraphNode, GraphEdge } from '../types/models';
import type { WindowConfig } from '../types/explorationGraph';
import {
  buildExplorationGraph,
  applyPrimaryTierWindowing,
  applyProgressiveDisclosure,
  isVirtualNode,
} from './explorationGraph';

describe('explorationGraph', () => {
  // Test data: default hierarchy with 15 milestones
  const milestones: GraphNode[] = Array.from({ length: 15 }, (_, i) => ({
    id: `m${i}`,
    label: `Milestone v${i}.0`,
    state: i < 10 ? 'done' : 'ready',
    priority: 'high',
    labels: ['type:milestone', `milestone:v${i}.0`],
    blocked: false,
  }));

  const epics: GraphNode[] = [
    {
      id: 'e1',
      label: 'Epic Auth',
      state: 'done',
      priority: 'high',
      labels: ['type:epic', 'milestone:v10.0', 'epic:auth'],
      blocked: false,
    },
    {
      id: 'e2',
      label: 'Epic Payments',
      state: 'in_progress',
      priority: 'high',
      labels: ['type:epic', 'milestone:v10.0', 'epic:payments'],
      blocked: false,
    },
    {
      id: 'e3',
      label: 'Epic Mobile',
      state: 'ready',
      priority: 'normal',
      labels: ['type:epic', 'milestone:v11.0', 'epic:mobile'],
      blocked: false,
    },
  ];

  // 20 stories under epic auth (to test budget)
  const stories: GraphNode[] = Array.from({ length: 20 }, (_, i) => ({
    id: `s${i}`,
    label: `Story ${i}`,
    state: 'ready',
    priority: i < 5 ? 'high' : 'normal',
    labels: ['type:story', 'milestone:v10.0', 'epic:auth', `story:s${i}`],
    blocked: i === 0, // First story is blocked for testing
  }));

  const tasks: GraphNode[] = [
    {
      id: 't1',
      label: 'Task Login',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task', 'milestone:v10.0', 'epic:auth', 'story:s0'],
      blocked: false,
    },
    {
      id: 't2',
      label: 'Task JWT',
      state: 'ready',
      priority: 'normal',
      labels: ['type:task', 'milestone:v10.0', 'epic:auth', 'story:s0'],
      blocked: false,
    },
  ];

  const unassigned: GraphNode[] = [
    {
      id: 'u1',
      label: 'Unassigned task',
      state: 'backlog',
      priority: 'low',
      labels: ['type:task'],
      blocked: false,
    },
  ];

  const allNodes = [...milestones, ...epics, ...stories, ...tasks, ...unassigned];

  const edges: GraphEdge[] = [
    { from: 'e1', to: 'm10' },
    { from: 'e2', to: 'm10' },
    { from: 'e3', to: 'm11' },
    { from: 's0', to: 'e1' },
    { from: 's1', to: 'e1' },
    { from: 't1', to: 's0' },
    { from: 't2', to: 's0' },
  ];

  const hierarchyConfig = createHierarchyConfig(['milestone', 'epic']);

  describe('isVirtualNode', () => {
    it('should identify virtual nodes', () => {
      const virtualNode = {
        id: 'bucket-0-5',
        virtualType: { type: 'collapsed_primary_bucket' as const, range: [0, 5], counts: { total: 10 } },
        label: 'Milestones 1-5',
      };
      expect(isVirtualNode(virtualNode)).toBe(true);
    });

    it('should identify real nodes', () => {
      expect(isVirtualNode(milestones[0])).toBe(false);
    });
  });

  describe('applyPrimaryTierWindowing', () => {
    const windowConfig: WindowConfig = {
      visiblePrimaryTierCount: 10,
      secondaryTierBudget: 20,
      tacticalBudget: 8,
    };

    it('should show all nodes when total tiers <= window size', () => {
      // Only 5 milestones
      const smallNodes = milestones.slice(0, 5);
      const result = applyPrimaryTierWindowing(
        smallNodes,
        hierarchyConfig,
        windowConfig,
        new Set()
      );

      expect(result.visibleNodes).toHaveLength(5);
      expect(result.virtualBuckets).toHaveLength(0);
    });

    it('should create collapsed bucket for older tiers', () => {
      const result = applyPrimaryTierWindowing(
        milestones,
        hierarchyConfig,
        windowConfig,
        new Set()
      );

      // Last 10 milestones visible (indices 5-14)
      expect(result.visibleNodes).toHaveLength(10);
      expect(result.virtualBuckets).toHaveLength(1);

      const bucket = result.virtualBuckets[0];
      expect(bucket.virtualType.type).toBe('collapsed_primary_bucket');
      expect(bucket.virtualType.range).toEqual([0, 4]); // Indices 0-4 collapsed
      expect(bucket.virtualType.counts.primary).toBe(5);
    });

    it('should expand bucket when in expansion state', () => {
      const expansionState = new Set([0]); // Bucket starting at 0 is expanded
      const result = applyPrimaryTierWindowing(
        milestones,
        hierarchyConfig,
        windowConfig,
        expansionState
      );

      // All 15 milestones visible (bucket expanded)
      expect(result.visibleNodes).toHaveLength(15);
      expect(result.virtualBuckets).toHaveLength(0);
    });

    it('should handle unassigned nodes (index -1)', () => {
      const result = applyPrimaryTierWindowing(
        [...milestones, ...unassigned],
        hierarchyConfig,
        windowConfig,
        new Set()
      );

      // Last 10 milestones + unassigned
      expect(result.visibleNodes.some(n => n.id === 'u1')).toBe(true);
    });

    it('should work with no primary tier (minimal hierarchy)', () => {
      const minimalConfig = createHierarchyConfig(['milestone']);
      const result = applyPrimaryTierWindowing(
        milestones.slice(0, 5),
        minimalConfig,
        windowConfig,
        new Set()
      );

      // All nodes at index 0, no windowing
      expect(result.visibleNodes).toHaveLength(5);
      expect(result.virtualBuckets).toHaveLength(0);
    });
  });

  describe('applyProgressiveDisclosure', () => {
    const windowConfig: WindowConfig = {
      visiblePrimaryTierCount: 10,
      secondaryTierBudget: 20,
      tacticalBudget: 8,
    };

    it('should collapse tactical items by default', () => {
      // Epic with 20 stories, budget is 8
      const nodesInTier = [epics[0], ...stories];
      const result = applyProgressiveDisclosure(
        nodesInTier,
        allNodes,
        edges,
        hierarchyConfig,
        windowConfig,
        new Set(), // No expansions
        new Set()  // No selected path
      );

      // Should have epic + collapsed group virtual node (stories hidden)
      const virtualNodes = result.nodes.filter(isVirtualNode);
      expect(virtualNodes.length).toBeGreaterThan(0);
      
      const collapsedGroup = virtualNodes.find(
        v => v.virtualType.type === 'collapsed_secondary_group'
      );
      expect(collapsedGroup).toBeDefined();
    });

    it('should show top-scored tactical items when expanded', () => {
      const nodesInTier = [epics[0], ...stories];
      const expansionState = new Set(['e1']); // Epic auth expanded
      
      const result = applyProgressiveDisclosure(
        nodesInTier,
        allNodes,
        edges,
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      // Should show epic + up to 8 top-scored stories
      const realStoryNodes = result.nodes.filter(
        n => !isVirtualNode(n) && n.labels.some(l => l.startsWith('type:story'))
      );
      expect(realStoryNodes.length).toBeLessThanOrEqual(8);
      expect(realStoryNodes.length).toBeGreaterThan(0);

      // Blocked story should be in top results (high relevance)
      expect(realStoryNodes.some(n => n.id === 's0')).toBe(true);
    });

    it('should create more node when budget exceeded', () => {
      const nodesInTier = [epics[0], ...stories];
      const expansionState = new Set(['e1']);
      
      const result = applyProgressiveDisclosure(
        nodesInTier,
        allNodes,
        edges,
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      // Should have a "more" node for the remaining 12 stories (20 - 8)
      const moreNode = result.nodes.find(
        n => isVirtualNode(n) && n.virtualType.type === 'more_node'
      );
      expect(moreNode).toBeDefined();
      if (moreNode && isVirtualNode(moreNode)) {
        expect(moreNode.virtualType.count).toBe(12);
      }
    });

    it('should handle minimal hierarchy (no secondary tier)', () => {
      const minimalConfig = createHierarchyConfig(['milestone']);
      const nodesInTier = [milestones[10], ...tasks];
      
      const result = applyProgressiveDisclosure(
        nodesInTier,
        allNodes,
        edges,
        minimalConfig,
        windowConfig,
        new Set(),
        new Set()
      );

      // Tasks collapse directly under milestone
      expect(result.nodes.some(n => n.id === 'm10')).toBe(true);
    });
  });

  describe('buildExplorationGraph', () => {
    const windowConfig: WindowConfig = {
      visiblePrimaryTierCount: 10,
      secondaryTierBudget: 20,
      tacticalBudget: 8,
    };

    const expansionState = {
      expandedPrimaryBuckets: new Set<number>(),
      expandedSecondaryGroups: new Set<string>(),
    };

    it('should build exploration graph for large dataset', () => {
      const graph = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      expect(graph.nodes.length).toBeGreaterThan(0);
      expect(graph.nodes.length).toBeLessThan(allNodes.length); // Some collapsed
      
      // Should have virtual nodes
      const virtualCount = graph.nodes.filter(isVirtualNode).length;
      expect(virtualCount).toBeGreaterThan(0);
    });

    it('should preserve edges between visible nodes', () => {
      const graph = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      // Edges between visible nodes should be preserved
      expect(graph.edges.length).toBeGreaterThan(0);
    });

    it('should be deterministic', () => {
      const graph1 = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      const graph2 = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      expect(graph1.nodes.map(n => n.id)).toEqual(graph2.nodes.map(n => n.id));
    });

    it('should handle small graphs without collapsing', () => {
      const smallNodes = [...milestones.slice(0, 5), epics[0]];
      const smallEdges = [{ from: 'e1', to: 'm0' }];

      const graph = buildExplorationGraph(
        { nodes: smallNodes, edges: smallEdges },
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      // Small graph shouldn't need virtual nodes
      const virtualCount = graph.nodes.filter(isVirtualNode).length;
      expect(virtualCount).toBe(0);
    });

    it('should respect expansion state', () => {
      const withExpansion = {
        expandedPrimaryBuckets: new Set([0]),
        expandedSecondaryGroups: new Set(['e1']),
      };

      const graphExpanded = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        withExpansion,
        new Set()
      );

      const graphCollapsed = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        expansionState,
        new Set()
      );

      // Expanded graph should have more real nodes
      const expandedRealCount = graphExpanded.nodes.filter(n => !isVirtualNode(n)).length;
      const collapsedRealCount = graphCollapsed.nodes.filter(n => !isVirtualNode(n)).length;
      expect(expandedRealCount).toBeGreaterThan(collapsedRealCount);
    });

    it('should use selected path for relevance scoring', () => {
      const selectedPath = new Set(['s0']); // Story 0 selected
      const withExpansion = {
        expandedPrimaryBuckets: new Set<number>(),
        expandedSecondaryGroups: new Set(['e1']), // Epic expanded to show stories
      };

      const graph = buildExplorationGraph(
        { nodes: allNodes, edges },
        hierarchyConfig,
        windowConfig,
        withExpansion,
        selectedPath
      );

      // Selected story should be in visible nodes
      expect(graph.nodes.some(n => n.id === 's0')).toBe(true);
    });
  });
});
