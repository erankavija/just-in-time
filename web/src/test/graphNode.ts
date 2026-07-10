import type { GraphNode } from '../types/models';

/**
 * Build one node of a `GET /graph` payload for tests.
 *
 * The resolution fields (`type`, `parent`, `children`, `cluster`, `rank`) are
 * server-owned, so a test states them as literal fixture values — the same
 * values the core resolver emits for the graph under test. Deriving them from
 * the edge list inside a test would re-create the client-side resolver the web
 * UI stopped carrying.
 *
 * Defaults describe an untyped, unresolved orphan; override what the case needs.
 */
export function graphNode(id: string, overrides: Partial<GraphNode> = {}): GraphNode {
  return {
    id,
    label: id,
    state: 'ready',
    priority: 'normal',
    labels: [],
    blocked: false,
    type: null,
    parent: null,
    children: [],
    cluster: null,
    rank: 0,
    ...overrides,
  };
}
