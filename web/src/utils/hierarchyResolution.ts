/**
 * Canonical DAG-authoritative hierarchy resolution — a TypeScript port of the
 * jit core resolver (`crates/jit/src/graph/hierarchy.rs`).
 *
 * The dependency DAG is the single source of truth for containment: a container
 * depends on the work it contains. Membership labels are advisory and are NOT
 * consulted here. This module exists so the web UI shares one canonical
 * resolution with the CLI and exports rather than re-deriving its own; the
 * shared fixture `test-vectors/hierarchy_resolution.json` pins the two
 * implementations together (see `hierarchyResolution.test.ts`).
 *
 * A type is a *container* iff its level is strictly less than the deepest
 * configured level. For each node the resolver produces:
 * - `parent`: the nearest dominating container — a direct container edge (the
 *   container lists the node among its dependencies) outranks one that only
 *   reaches the node transitively through a cross-cutting dependency, then
 *   deepest level, then fewest hops, then smallest id; `null` for a root.
 * - `children`: the inverse of `parent`, sorted by id.
 * - `cluster`: the strategic root of the parent chain, or `null` for an orphan.
 * - `rank`: the longest dependency-path length to an in-set sink.
 */

/** A node in the dependency graph, as consumed by {@link resolveHierarchy}. */
export interface HierarchyInputNode {
  id: string;
  /** The node's `type:` label value, or `null`/`undefined` when it has none. */
  type: string | null | undefined;
  /** Ids of the nodes this node depends on. */
  dependencies: string[];
}

/** The resolved hierarchy facts for one node. */
export interface ResolvedNode {
  parent: string | null;
  children: string[];
  cluster: string | null;
  rank: number;
}

/** Map of type name to hierarchy level (lower = more strategic). */
export type HierarchyLevels = Record<string, number>;

/**
 * Resolve the canonical hierarchy for `nodes` using the dependency DAG.
 *
 * Pure and deterministic: the result depends only on node ids, their dependency
 * edges, and their configured type levels. Returns a map keyed by node id; every
 * input node has an entry.
 */
export function resolveHierarchy(
  nodes: HierarchyInputNode[],
  hierarchy: HierarchyLevels,
): Map<string, ResolvedNode> {
  const byId = new Map<string, HierarchyInputNode>();
  for (const node of nodes) byId.set(node.id, node);

  const levels = Object.values(hierarchy);
  const leafLevel = levels.length > 0 ? Math.max(...levels) : undefined;

  const levelOf = (id: string): number | undefined => {
    const type = byId.get(id)?.type;
    return type != null ? hierarchy[type] : undefined;
  };
  const isContainer = (id: string): boolean => {
    const level = levelOf(id);
    return level !== undefined && leafLevel !== undefined && level < leafLevel;
  };

  // Nearest containing container per node: id -> [level, distance, containerId].
  type Candidate = [number, number, string];
  const best = new Map<string, Candidate>();
  const isBetter = (a: Candidate, b: Candidate): boolean => {
    const aDirect = a[1] === 1;
    const bDirect = b[1] === 1;
    if (aDirect !== bDirect) return aDirect; // a direct container edge wins
    if (a[0] !== b[0]) return a[0] > b[0]; // deeper level wins
    if (a[1] !== b[1]) return a[1] < b[1]; // fewer hops wins
    return a[2] < b[2]; // smaller id wins
  };

  for (const container of nodes) {
    if (!isContainer(container.id)) continue;
    const clevel = levelOf(container.id) as number;

    // BFS over the container's dependency closure; BFS order gives shortest hops.
    const visited = new Set<string>([container.id]);
    const queue: Array<[string, number]> = [[container.id, 0]];
    while (queue.length > 0) {
      const [cur, dist] = queue.shift()!;
      const node = byId.get(cur);
      if (!node) continue;
      for (const dep of node.dependencies) {
        if (!byId.has(dep) || visited.has(dep)) continue;
        visited.add(dep);
        const candidate: Candidate = [clevel, dist + 1, container.id];
        const existing = best.get(dep);
        if (!existing || isBetter(candidate, existing)) best.set(dep, candidate);
        queue.push([dep, dist + 1]);
      }
    }
  }

  // Parent map and its inverse (children).
  const parentOf = new Map<string, string>();
  const childrenOf = new Map<string, string[]>();
  for (const [child, candidate] of best) {
    parentOf.set(child, candidate[2]);
    const kids = childrenOf.get(candidate[2]) ?? [];
    kids.push(child);
    childrenOf.set(candidate[2], kids);
  }
  for (const kids of childrenOf.values()) kids.sort();

  // Cluster: climb the parent chain to the topmost container.
  const clusterOf = (start: string): string | null => {
    let cur = start;
    const seen = new Set<string>();
    for (;;) {
      const parent = parentOf.get(cur);
      if (parent === undefined) break;
      if (seen.has(cur)) break; // defensive: never loop on malformed data
      seen.add(cur);
      cur = parent;
    }
    return isContainer(cur) ? cur : null;
  };

  // Longest-path rank, memoized across the whole set.
  const rankMemo = new Map<string, number>();
  const longestPath = (id: string, visiting: Set<string>): number => {
    const cached = rankMemo.get(id);
    if (cached !== undefined) return cached;
    if (visiting.has(id)) return 0; // defensive cycle guard
    visiting.add(id);
    let best = 0;
    const node = byId.get(id);
    if (node) {
      for (const dep of node.dependencies) {
        if (byId.has(dep)) best = Math.max(best, 1 + longestPath(dep, visiting));
      }
    }
    visiting.delete(id);
    rankMemo.set(id, best);
    return best;
  };

  const resolved = new Map<string, ResolvedNode>();
  for (const node of nodes) {
    resolved.set(node.id, {
      parent: parentOf.get(node.id) ?? null,
      children: childrenOf.get(node.id) ?? [],
      cluster: clusterOf(node.id),
      rank: longestPath(node.id, new Set<string>()),
    });
  }
  return resolved;
}
