import { describe, it, expect, vi, beforeEach } from 'vitest';
import { render, waitFor, act } from '@testing-library/react';
import dagre from 'dagre';
import { GraphView } from '../GraphView';
import { apiClient } from '../../../api/client';
import type { GraphData, GraphNode } from '../../../types/models';

/**
 * The stable-layout cache in GraphView skips Dagre when nothing that feeds
 * clustering or layout has changed.  Its fingerprint therefore has to cover the
 * hierarchy resolution the server sends (`type`, `parent`, `children`,
 * `cluster`, `rank`) alongside the node and edge sets: an issue whose `type:`
 * label is retyped keeps every node id and every edge, yet lands in a different
 * cluster at a different rank.
 *
 * `dagre.layout` is the observable: the full-layout branch calls it, the
 * in-place data patch does not.
 */

const { setNodes, setEdges } = vi.hoisted(() => ({ setNodes: vi.fn(), setEdges: vi.fn() }));

vi.mock('reactflow', () => ({
  default: () => null,
  Controls: () => null,
  Background: () => null,
  useNodesState: () => [[], setNodes, vi.fn()],
  useEdgesState: () => [[], setEdges, vi.fn()],
  MarkerType: { ArrowClosed: 'arrowclosed' },
  Position: { Left: 'left', Right: 'right' },
}));

vi.mock('dagre', () => ({
  default: {
    graphlib: {
      Graph: vi.fn().mockImplementation(function () {
        return {
          setDefaultEdgeLabel: vi.fn(),
          setGraph: vi.fn(),
          setNode: vi.fn(),
          setEdge: vi.fn(),
          node: vi.fn(() => ({ x: 0, y: 0 })),
        };
      }),
    },
    layout: vi.fn(),
  },
}));

vi.mock('../../../api/client', () => ({
  apiClient: {
    getGraph: vi.fn(),
    getHierarchy: vi.fn(),
  },
}));

const getGraph = vi.mocked(apiClient.getGraph);
const getHierarchy = vi.mocked(apiClient.getHierarchy);
const dagreLayout = vi.mocked(dagre.layout);

// Stable identity across rerenders: a fresh array would reset the fingerprint
// through the view-settings effect and mask what these tests assert.
const NO_LABEL_FILTERS: string[] = [];

function pendingGraph() {
  let resolve!: (graph: GraphData) => void;
  const promise = new Promise<GraphData>(complete => { resolve = complete; });
  return { promise, resolve };
}

const node = (overrides: Partial<GraphNode> & Pick<GraphNode, 'id'>): GraphNode => ({
  short_id: overrides.id,
  label: `Issue ${overrides.id}`,
  state: 'ready',
  priority: 'normal',
  labels: [],
  blocked: false,
  type: 'task',
  parent: null,
  children: [],
  cluster: null,
  rank: 0,
  ...overrides,
});

/** Epic `e1` containing task `t1`, which depends on task `t2`. */
const baseGraph = (): GraphData => ({
  count: 3,
  nodes: [
    node({ id: 'e1', type: 'epic', children: ['t1'], cluster: 'e1', rank: 2 }),
    node({ id: 't1', parent: 'e1', cluster: 'e1', rank: 1 }),
    node({ id: 't2', parent: 'e1', cluster: 'e1', rank: 0 }),
  ],
  edges: [
    { from: 'e1', to: 't1' },
    { from: 't1', to: 't2' },
  ],
});

/**
 * Renders with `layoutAlgorithm="dagre"`, settles the async hierarchy fetch and
 * first load, then returns the Dagre call count to measure subsequent loads
 * against.
 */
const renderAndSettle = async () => {
  const view = render(
    <GraphView layoutAlgorithm="dagre" labelFilters={NO_LABEL_FILTERS} version={1} />
  );
  await waitFor(() => expect(dagreLayout).toHaveBeenCalled());
  await act(async () => {
    await Promise.resolve();
  });
  return { view, baseline: dagreLayout.mock.calls.length };
};

/** Pushes `next` as the served graph and bumps `version` to force a re-fetch. */
const refetchWith = async (
  view: Awaited<ReturnType<typeof renderAndSettle>>['view'],
  next: GraphData
) => {
  getGraph.mockResolvedValue(next);
  const callsBefore = getGraph.mock.calls.length;
  view.rerender(
    <GraphView layoutAlgorithm="dagre" labelFilters={NO_LABEL_FILTERS} version={2} />
  );
  await waitFor(() => expect(getGraph.mock.calls.length).toBeGreaterThan(callsBefore));
  await act(async () => {
    await Promise.resolve();
  });
};

describe('GraphView stable-layout cache', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    // GraphView reads and writes viewport/expansion state through localStorage,
    // which this jsdom environment leaves undefined. Without a stub the loader
    // throws before it records the layout fingerprint.
    const store = new Map<string, string>();
    vi.stubGlobal('localStorage', {
      getItem: (key: string) => store.get(key) ?? null,
      setItem: (key: string, value: string) => void store.set(key, value),
      removeItem: (key: string) => void store.delete(key),
      clear: () => store.clear(),
    });
    getGraph.mockResolvedValue(baseGraph());
    getHierarchy.mockResolvedValue({
      types: { epic: 2, story: 3, task: 4 },
      strategic_types: ['epic'],
      icons: {},
    });
  });

  it('test_graph_view_reports_full_nodes_with_an_active_label_filter', async () => {
    const onGraphNodes = vi.fn();
    render(<GraphView
      layoutAlgorithm="dagre"
      labelFilters={['team:visible']}
      onGraphNodes={onGraphNodes}
    />);

    await waitFor(() => expect(onGraphNodes).toHaveBeenCalledWith(baseGraph().nodes));
  });

  it('test_graph_view_reports_only_the_latest_version_graph', async () => {
    const oldVersion = pendingGraph();
    const newVersion = pendingGraph();
    getGraph.mockReset().mockReturnValueOnce(oldVersion.promise).mockReturnValueOnce(newVersion.promise);
    getHierarchy.mockImplementation(() => new Promise(() => {}));
    const onGraphNodes = vi.fn();
    const view = render(<GraphView
      layoutAlgorithm="dagre"
      labelFilters={NO_LABEL_FILTERS}
      version={1}
      onGraphNodes={onGraphNodes}
    />);
    view.rerender(<GraphView
      layoutAlgorithm="dagre"
      labelFilters={NO_LABEL_FILTERS}
      version={2}
      onGraphNodes={onGraphNodes}
    />);

    const latest = { count: 1, nodes: [node({ id: 'latest' })], edges: [] };
    await act(async () => newVersion.resolve(latest));
    expect(onGraphNodes).toHaveBeenCalledWith(latest.nodes);
    await act(async () => oldVersion.resolve(baseGraph()));
    expect(onGraphNodes).toHaveBeenCalledTimes(1);
  });

  it('re-runs layout when a served cluster changes but nodes and edges do not', async () => {
    const { view, baseline } = await renderAndSettle();

    // `t2` is retyped and reparented out of the epic. Same three node ids, same
    // two edges; only the resolution the server sends differs.
    const reclustered = baseGraph();
    reclustered.nodes[2] = node({
      id: 't2',
      type: 'story',
      parent: null,
      cluster: null,
      rank: 0,
    });

    await refetchWith(view, reclustered);

    expect(dagreLayout.mock.calls.length).toBeGreaterThan(baseline);
  });

  it('re-runs layout when a served rank changes but nodes and edges do not', async () => {
    const { view, baseline } = await renderAndSettle();

    const reranked = baseGraph();
    reranked.nodes[1] = node({ id: 't1', parent: 'e1', cluster: 'e1', rank: 7 });

    await refetchWith(view, reranked);

    expect(dagreLayout.mock.calls.length).toBeGreaterThan(baseline);
  });

  it('updates refreshed node data while retaining layout positions', async () => {
    const { view } = await renderAndSettle();
    const initial = setNodes.mock.calls.map(call => call[0]).find(Array.isArray);
    if (!initial) throw new Error('Initial layout was not applied');

    const repainted = baseGraph();
    repainted.nodes[1] = node({ id: 't1', parent: 'e1', cluster: 'e1', rank: 1, state: 'done' });
    await refetchWith(view, repainted);

    const patch = setNodes.mock.calls.map(call => call[0]).find(value => typeof value === 'function');
    expect(patch).toBeDefined();
    const refreshed = patch(initial);
    const original = initial.find((n: { id: string }) => n.id === 't1');
    const updated = refreshed.find((n: { id: string }) => n.id === 't1');
    expect(updated.position).toEqual(original.position);
    expect(updated.style.border).not.toBe(original.style.border);
  });

  it('preserves positions when only presentational node data changes', async () => {
    const { view, baseline } = await renderAndSettle();

    // A state transition and an assignment: nothing that feeds layout.
    const repainted = baseGraph();
    repainted.nodes[1] = node({
      id: 't1',
      parent: 'e1',
      cluster: 'e1',
      rank: 1,
      state: 'done',
      assignee: 'agent:worker-1',
    });

    await refetchWith(view, repainted);

    expect(dagreLayout.mock.calls.length).toBe(baseline);
  });
});
