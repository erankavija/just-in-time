//! Canonical hierarchy resolution over the dependency DAG.
//!
//! # The DAG is authoritative; membership labels are advisory
//!
//! Containment in JIT can be read two ways, and they can disagree:
//!
//! 1. **The dependency DAG** — a container issue *depends on* the work it
//!    contains (an epic depends on its stories/tasks, a milestone on its epics).
//!    Following a container's outgoing dependency edges into more-tactical nodes
//!    yields its contents.
//! 2. **Membership labels** — an advisory grouping tag (`epic:auth`,
//!    `milestone:v1.0`) an issue carries to say "I belong to this group".
//!
//! This module resolves hierarchy **exclusively from the dependency DAG**. The
//! dependency graph is the single source of truth for parent, children, cluster,
//! and rank; membership labels are never consulted here. Where a label claims a
//! membership the DAG does not back, that disagreement is *reported* (see
//! [`detect_membership_divergences`]) rather than silently reconciled — the label
//! does not change the resolved hierarchy.
//!
//! Historically this logic lived only in the web UI's TypeScript, so any other
//! tool had to re-port it to match jit's semantics. Lifting it into the core
//! library gives the CLI, the web UI, and exports one canonical resolver.
//!
//! # What "container" means
//!
//! Container-ness comes from the configured [type hierarchy
//! levels](crate::type_hierarchy::HierarchyConfig), never from label namespaces
//! (INV-DOMAIN-AGNOSTIC). A type is a **container** iff its level is strictly
//! less than the deepest (leaf) configured level. With the default
//! `milestone=1, epic=2, story=3, task=4`, the leaf level is `4`, so
//! `milestone`, `epic`, and `story` are containers and `task` is a leaf. Nodes
//! whose type is unknown to the config (or that carry no `type:` label) have no
//! level and are treated as leaves.
//!
//! # Resolution rules
//!
//! For every node the resolver produces a [`NodeHierarchy`]:
//!
//! - **parent** — the *nearest dominating container*: among all containers whose
//!   dependency closure includes the node, the one that most directly contains
//!   it. A container linked by a **direct** dependency edge (it lists the node
//!   among its own dependencies) outranks one that only reaches the node
//!   transitively through a cross-cutting dependency — so a task stays under the
//!   epic that directly owns it even when a deeper container elsewhere reaches it
//!   via a cross edge. Among equally-direct candidates the deepest level wins,
//!   then the fewest hops from container to node, then the lexicographically
//!   smallest container id.
//! - **children** — the inverse of `parent`: the nodes whose resolved parent is
//!   this node, sorted by id. A node that is a direct dependency of a container
//!   but resolves to a *nearer* container is that nearer container's child, so
//!   `parent`/`children` always form a consistent forest.
//! - **cluster** — the strategic root of the node's parent chain: follow `parent`
//!   pointers upward to the topmost container. A leaf with no container ancestor
//!   has no cluster (`None`); a root container is its own cluster.
//! - **rank** — the longest dependency path length from the node to a sink (a
//!   node with no in-set dependencies). Sinks have rank `0`. Used as a stable
//!   layout depth.
//!
//! ## Defined edge cases
//!
//! - **Root nodes** (no container ancestor): `parent = None`. A root *container*
//!   is its own `cluster`; a root *leaf* has `cluster = None` (an orphan).
//! - **Multi-parent / diamond containment** (two containers both contain a node):
//!   exactly one is chosen as the canonical `parent` by the deterministic tie
//!   break above (deepest level, then fewest hops, then smallest id); the node
//!   appears under only that one container's `children`.
//! - **Rejected / archived containers**: resolution is purely structural and
//!   does **not** consult issue state. A rejected or archived container still
//!   contains its DAG descendants; consumers that want to hide such subtrees
//!   filter by state themselves.
//!
//! ## Deliberate divergence from the web UI's historical clustering
//!
//! The TypeScript UI, for display, picks the epic level (level 2) as its top
//! render cluster and treats milestones as visible nodes rather than containers.
//! The canonical core instead treats **every** non-leaf level as a container, so
//! a task under an epic under a milestone clusters to the *milestone* (the
//! strategic root). The DAG-authoritative rule wins; the UI's level choice is a
//! presentation concern layered on top.
//!
//! # Examples
//!
//! ```
//! use jit::domain::Issue;
//! use jit::graph::hierarchy::resolve_hierarchy;
//! use jit::type_hierarchy::HierarchyConfig;
//!
//! // milestone → epic → task (each container depends on what it contains)
//! let mut milestone = Issue::new("Release".into(), String::new());
//! milestone.labels = vec!["type:milestone".into()];
//! let mut epic = Issue::new("Auth".into(), String::new());
//! epic.labels = vec!["type:epic".into()];
//! let mut task = Issue::new("Login".into(), String::new());
//! task.labels = vec!["type:task".into()];
//!
//! milestone.dependencies = vec![epic.id.clone()];
//! epic.dependencies = vec![task.id.clone()];
//!
//! let config = HierarchyConfig::default();
//! let nodes = [&milestone, &epic, &task];
//! let resolution = resolve_hierarchy(&nodes, &config);
//!
//! // The task's nearest container is the epic, not the milestone.
//! assert_eq!(resolution.parent(&task.id), Some(epic.id.as_str()));
//! assert_eq!(resolution.parent(&epic.id), Some(milestone.id.as_str()));
//! assert_eq!(resolution.parent(&milestone.id), None);
//!
//! // The whole chain clusters to the strategic root (the milestone).
//! assert_eq!(resolution.cluster(&task.id), Some(milestone.id.as_str()));
//! ```

use crate::type_hierarchy::HierarchyConfig;
use std::collections::{HashMap, HashSet, VecDeque};

/// A node that can participate in hierarchy resolution.
///
/// Extends the dependency-graph view (`id` + `dependencies`) with the node's
/// *type name* — the value of its `type:` label — so the resolver can map each
/// node to a level via the [`HierarchyConfig`]. Implemented for
/// [`Issue`](crate::domain::Issue); a test double need only return its type
/// value.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::graph::hierarchy::HierarchyNode;
///
/// let mut epic = Issue::new("Auth".into(), String::new());
/// epic.labels = vec!["type:epic".into()];
/// let task = Issue::new("Login".into(), String::new());
/// epic.dependencies = vec![task.id.clone()];
///
/// assert_eq!(epic.id(), epic.id.as_str());
/// assert_eq!(epic.type_name(), Some("epic"));
/// assert_eq!(epic.dependencies(), &[task.id.clone()]);
/// assert_eq!(task.type_name(), None); // no type label
/// ```
pub trait HierarchyNode {
    /// Unique identifier for this node.
    fn id(&self) -> &str;

    /// IDs of the nodes this node depends on (its outgoing DAG edges).
    fn dependencies(&self) -> &[String];

    /// The value of this node's `type:` label (e.g. `"epic"`), or `None` when it
    /// carries no type label. Mapped to a level via the config.
    fn type_name(&self) -> Option<&str>;
}

impl HierarchyNode for crate::domain::Issue {
    fn id(&self) -> &str {
        &self.id
    }

    fn dependencies(&self) -> &[String] {
        &self.dependencies
    }

    fn type_name(&self) -> Option<&str> {
        crate::labels::type_label_value(&self.labels)
    }
}

/// The resolved hierarchy facts for one node.
///
/// See the [module docs](self) for how each field is derived from the DAG.
///
/// # Examples
///
/// ```
/// use jit::graph::hierarchy::NodeHierarchy;
///
/// let facts = NodeHierarchy {
///     parent: Some("epic-1".into()),
///     children: vec!["task-a".into(), "task-b".into()],
///     cluster: Some("milestone-1".into()),
///     rank: 2,
/// };
/// assert_eq!(facts.parent.as_deref(), Some("epic-1"));
/// assert_eq!(facts.children.len(), 2);
/// assert_eq!(facts.rank, 2);
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NodeHierarchy {
    /// The nearest dominating container's id, or `None` for a root node.
    pub parent: Option<String>,
    /// Ids of nodes whose resolved `parent` is this node, sorted ascending.
    pub children: Vec<String>,
    /// The strategic root container's id, or `None` for an orphan leaf.
    pub cluster: Option<String>,
    /// Longest dependency-path length from this node to an in-set sink.
    pub rank: u32,
}

/// The resolved hierarchy for a whole set of nodes.
///
/// Every input node has an entry (roots and orphans included). Look facts up by
/// id through the accessors; the map is keyed by node id.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::graph::hierarchy::resolve_hierarchy;
/// use jit::type_hierarchy::HierarchyConfig;
///
/// let mut epic = Issue::new("Epic".into(), String::new());
/// epic.labels = vec!["type:epic".into()];
/// let mut task = Issue::new("Task".into(), String::new());
/// task.labels = vec!["type:task".into()];
/// epic.dependencies = vec![task.id.clone()];
///
/// let resolution = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
/// assert_eq!(resolution.len(), 2);
/// assert_eq!(resolution.children(&epic.id), [task.id.clone()]);
/// assert_eq!(resolution.rank(&epic.id), Some(1));
/// assert_eq!(resolution.rank(&task.id), Some(0));
/// ```
#[derive(Debug, Clone, Default)]
pub struct HierarchyResolution {
    nodes: HashMap<String, NodeHierarchy>,
}

impl HierarchyResolution {
    /// The full [`NodeHierarchy`] for `id`, if the node was in the input.
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let mut epic = Issue::new("Epic".into(), String::new());
    /// epic.labels = vec!["type:epic".into()];
    /// let task = Issue::new("Task".into(), String::new());
    /// epic.dependencies = vec![task.id.clone()];
    ///
    /// let r = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
    /// assert_eq!(r.get(&task.id).unwrap().parent.as_deref(), Some(epic.id.as_str()));
    /// assert!(r.get("no-such-id").is_none());
    /// ```
    pub fn get(&self, id: &str) -> Option<&NodeHierarchy> {
        self.nodes.get(id)
    }

    /// The nearest dominating container's id for `id`, or `None` when the node
    /// is a root (or absent).
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let mut epic = Issue::new("Epic".into(), String::new());
    /// epic.labels = vec!["type:epic".into()];
    /// let task = Issue::new("Task".into(), String::new());
    /// epic.dependencies = vec![task.id.clone()];
    ///
    /// let r = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
    /// assert_eq!(r.parent(&task.id), Some(epic.id.as_str()));
    /// assert_eq!(r.parent(&epic.id), None); // a root
    /// ```
    pub fn parent(&self, id: &str) -> Option<&str> {
        self.nodes.get(id).and_then(|n| n.parent.as_deref())
    }

    /// The ids of `id`'s resolved children (empty when it has none, or is absent).
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let mut epic = Issue::new("Epic".into(), String::new());
    /// epic.labels = vec!["type:epic".into()];
    /// let task = Issue::new("Task".into(), String::new());
    /// epic.dependencies = vec![task.id.clone()];
    ///
    /// let r = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
    /// assert_eq!(r.children(&epic.id), [task.id.clone()]);
    /// assert!(r.children(&task.id).is_empty());
    /// ```
    pub fn children(&self, id: &str) -> &[String] {
        self.nodes
            .get(id)
            .map(|n| n.children.as_slice())
            .unwrap_or(&[])
    }

    /// The strategic root container's id for `id`, or `None` for an orphan leaf
    /// (or absent node).
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let mut epic = Issue::new("Epic".into(), String::new());
    /// epic.labels = vec!["type:epic".into()];
    /// let task = Issue::new("Task".into(), String::new());
    /// epic.dependencies = vec![task.id.clone()];
    ///
    /// let r = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
    /// // The epic is the strategic root, so the task clusters to it.
    /// assert_eq!(r.cluster(&task.id), Some(epic.id.as_str()));
    /// ```
    pub fn cluster(&self, id: &str) -> Option<&str> {
        self.nodes.get(id).and_then(|n| n.cluster.as_deref())
    }

    /// The longest-path rank for `id`, or `None` when the node is absent.
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let mut epic = Issue::new("Epic".into(), String::new());
    /// epic.labels = vec!["type:epic".into()];
    /// let task = Issue::new("Task".into(), String::new());
    /// epic.dependencies = vec![task.id.clone()];
    ///
    /// let r = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
    /// assert_eq!(r.rank(&epic.id), Some(1));
    /// assert_eq!(r.rank(&task.id), Some(0));
    /// assert_eq!(r.rank("no-such-id"), None);
    /// ```
    pub fn rank(&self, id: &str) -> Option<u32> {
        self.nodes.get(id).map(|n| n.rank)
    }

    /// Iterate over every resolved node as `(id, facts)`.
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let mut epic = Issue::new("Epic".into(), String::new());
    /// epic.labels = vec!["type:epic".into()];
    /// let task = Issue::new("Task".into(), String::new());
    /// epic.dependencies = vec![task.id.clone()];
    ///
    /// let r = resolve_hierarchy(&[&epic, &task], &HierarchyConfig::default());
    /// assert_eq!(r.iter().count(), 2);
    /// ```
    pub fn iter(&self) -> impl Iterator<Item = (&String, &NodeHierarchy)> {
        self.nodes.iter()
    }

    /// Number of resolved nodes.
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let task = Issue::new("Task".into(), String::new());
    /// let r = resolve_hierarchy(&[&task], &HierarchyConfig::default());
    /// assert_eq!(r.len(), 1);
    /// ```
    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    /// Whether the resolution is empty (no input nodes).
    ///
    /// # Examples
    ///
    /// ```
    /// # use jit::domain::Issue;
    /// # use jit::graph::hierarchy::resolve_hierarchy;
    /// # use jit::type_hierarchy::HierarchyConfig;
    /// let empty = resolve_hierarchy::<Issue>(&[], &HierarchyConfig::default());
    /// assert!(empty.is_empty());
    /// ```
    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

/// Resolve the canonical hierarchy for `nodes` using the dependency DAG.
///
/// Pure and deterministic: the result depends only on the node ids, their
/// dependency edges, and their configured type levels. Membership labels are not
/// consulted. See the [module docs](self) for the resolution rules and edge-case
/// semantics.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::graph::hierarchy::resolve_hierarchy;
/// use jit::type_hierarchy::HierarchyConfig;
///
/// // A diamond: two epics both depend on the same task.
/// let mut e1 = Issue::new("E1".into(), String::new());
/// e1.id = "aaaa".into();
/// e1.labels = vec!["type:epic".into()];
/// let mut e2 = Issue::new("E2".into(), String::new());
/// e2.id = "bbbb".into();
/// e2.labels = vec!["type:epic".into()];
/// let mut task = Issue::new("T".into(), String::new());
/// task.id = "cccc".into();
/// task.labels = vec!["type:task".into()];
/// e1.dependencies = vec![task.id.clone()];
/// e2.dependencies = vec![task.id.clone()];
///
/// let r = resolve_hierarchy(&[&e1, &e2, &task], &HierarchyConfig::default());
/// // Same level and hop count → smallest container id wins the tie.
/// assert_eq!(r.parent(&task.id), Some("aaaa"));
/// assert_eq!(r.children("aaaa"), [task.id.clone()]);
/// assert!(r.children("bbbb").is_empty());
/// ```
pub fn resolve_hierarchy<T: HierarchyNode>(
    nodes: &[&T],
    config: &HierarchyConfig,
) -> HierarchyResolution {
    let by_id: HashMap<&str, &T> = nodes.iter().map(|n| (n.id(), *n)).collect();
    let leaf_level = config.types().map(|(_, l)| *l).max();

    let level_of = |id: &str| -> Option<u8> {
        by_id
            .get(id)
            .and_then(|n| n.type_name())
            .and_then(|t| config.get_level(t))
    };
    // The container level of `id`, i.e. `Some(level)` only when the node is a
    // container (its level is strictly above a leaf). Returning the level and the
    // container-ness together lets the loop below extract the level with no
    // fallible re-lookup — the "container has a level" fact is proven by the
    // `Some` rather than asserted with a panic.
    let container_level = |id: &str| -> Option<u8> {
        match (level_of(id), leaf_level) {
            (Some(l), Some(max)) if l < max => Some(l),
            _ => None,
        }
    };
    let is_container = |id: &str| -> bool { container_level(id).is_some() };

    // Nearest containing container per node: id -> (container level, distance, container id).
    // A higher level, then a shorter distance, then a smaller id wins.
    let mut best: HashMap<&str, (u8, usize, &str)> = HashMap::new();
    for &container in nodes {
        let cid = container.id();
        let Some(clevel) = container_level(cid) else {
            continue;
        };

        // BFS over the container's dependency closure; BFS order gives the
        // shortest hop distance to each reachable node.
        let mut visited: HashSet<&str> = HashSet::new();
        visited.insert(cid);
        let mut queue: VecDeque<(&str, usize)> = VecDeque::new();
        queue.push_back((cid, 0));
        while let Some((cur, dist)) = queue.pop_front() {
            let Some(node) = by_id.get(cur) else { continue };
            for dep in node.dependencies() {
                let dep = dep.as_str();
                if !by_id.contains_key(dep) || !visited.insert(dep) {
                    continue;
                }
                let ndist = dist + 1;
                let candidate = (clevel, ndist, cid);
                best.entry(dep)
                    .and_modify(|current| {
                        if is_better_parent(candidate, *current) {
                            *current = candidate;
                        }
                    })
                    .or_insert(candidate);
                queue.push_back((dep, ndist));
            }
        }
    }

    // Parent map and its inverse (children).
    let parent_of: HashMap<&str, &str> = best.iter().map(|(child, v)| (*child, v.2)).collect();
    let mut children_of: HashMap<&str, Vec<String>> = HashMap::new();
    for (child, (_, _, container)) in &best {
        children_of
            .entry(*container)
            .or_default()
            .push((*child).to_string());
    }
    for kids in children_of.values_mut() {
        kids.sort();
    }

    // Cluster: climb the parent chain to the topmost container.
    let cluster_of = |start: &str| -> Option<String> {
        let mut cur = start;
        let mut seen: HashSet<&str> = HashSet::new();
        while let Some(parent) = parent_of.get(cur) {
            if !seen.insert(cur) {
                break; // defensive: never loop on malformed data
            }
            cur = parent;
        }
        is_container(cur).then(|| cur.to_string())
    };

    // Longest-path rank, memoized across the whole set.
    let mut rank_memo: HashMap<String, u32> = HashMap::new();

    let mut resolved: HashMap<String, NodeHierarchy> = HashMap::with_capacity(nodes.len());
    for &node in nodes {
        let id = node.id();
        let mut visiting: HashSet<String> = HashSet::new();
        let rank = longest_path(id, &by_id, &mut rank_memo, &mut visiting);
        resolved.insert(
            id.to_string(),
            NodeHierarchy {
                parent: parent_of.get(id).map(|p| (*p).to_string()),
                children: children_of.remove(id).unwrap_or_default(),
                cluster: cluster_of(id),
                rank,
            },
        );
    }

    HierarchyResolution { nodes: resolved }
}

/// Whether parent candidate `a` beats `b`.
///
/// A **direct** container edge (distance 1 — the container lists the node among
/// its own dependencies) is genuine containment and outranks a container that
/// only reaches the node transitively through a cross-cutting dependency. Among
/// equally-direct candidates: deeper level, then fewer hops, then smaller id.
fn is_better_parent(a: (u8, usize, &str), b: (u8, usize, &str)) -> bool {
    use std::cmp::Reverse;
    // Key order: direct edge (distance 1) first, then deeper level, then fewer
    // hops, then smaller id.
    (a.1 == 1, a.0, Reverse(a.1), Reverse(a.2)) > (b.1 == 1, b.0, Reverse(b.1), Reverse(b.2))
}

/// Longest dependency-path length from `id` to an in-set sink (memoized).
fn longest_path<T: HierarchyNode>(
    id: &str,
    by_id: &HashMap<&str, &T>,
    memo: &mut HashMap<String, u32>,
    visiting: &mut HashSet<String>,
) -> u32 {
    if let Some(v) = memo.get(id) {
        return *v;
    }
    if !visiting.insert(id.to_string()) {
        return 0; // defensive cycle guard; the DAG invariant forbids this
    }
    let mut best = 0;
    if let Some(node) = by_id.get(id) {
        for dep in node.dependencies() {
            if by_id.contains_key(dep.as_str()) {
                best = best.max(1 + longest_path(dep, by_id, memo, visiting));
            }
        }
    }
    visiting.remove(id);
    memo.insert(id.to_string(), best);
    best
}

/// A membership label whose claim the dependency DAG does not back.
///
/// Produced by [`detect_membership_divergences`]. The issue carries a membership
/// label `namespace:value`, and a container that owns that label exists, but the
/// issue is **not** in that container's dependency closure — so the label says
/// "member" while the authoritative DAG says "not a member".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MembershipDivergence {
    /// The id of the issue carrying the unsupported membership label.
    pub issue_id: String,
    /// The full `namespace:value` label (e.g. `"epic:auth"`).
    pub label: String,
    /// The label namespace (e.g. `"epic"`).
    pub namespace: String,
    /// The label value (e.g. `"auth"`).
    pub value: String,
}

/// Report membership labels that disagree with DAG-resolved containment.
///
/// For each configured membership namespace (from
/// [`label_associations`](crate::type_hierarchy::HierarchyConfig::membership_namespaces)),
/// the *anchor* of a label `ns:val` is the container issue whose type maps to
/// `ns` and that carries `ns:val` (e.g. the `type:epic` issue labeled
/// `epic:auth`). An issue that carries `ns:val` but does not sit in any such
/// anchor's dependency closure is reported as a divergence.
///
/// Only the label-claims-membership-but-DAG-disagrees direction is flagged. The
/// reverse (a DAG descendant that does not repeat its container's label) is
/// **not** a divergence: labels are advisory, and children normally rely on the
/// DAG rather than re-declaring the label. A label with no anchor at all is left
/// to the membership-reference validation, not reported here.
///
/// The result is sorted by `(issue_id, label)` for deterministic reporting.
///
/// # Examples
///
/// ```
/// use jit::domain::Issue;
/// use jit::graph::hierarchy::detect_membership_divergences;
/// use jit::type_hierarchy::HierarchyConfig;
///
/// let config = HierarchyConfig::default();
///
/// // The epic "auth" contains `inside` via the DAG but not `outside`.
/// let mut epic = Issue::new("Auth".into(), String::new());
/// epic.labels = vec!["type:epic".into(), "epic:auth".into()];
/// let mut inside = Issue::new("Inside".into(), String::new());
/// inside.labels = vec!["type:task".into(), "epic:auth".into()];
/// let mut outside = Issue::new("Outside".into(), String::new());
/// outside.labels = vec!["type:task".into(), "epic:auth".into()];
/// epic.dependencies = vec![inside.id.clone()]; // only `inside` is a DAG member
///
/// let divergences =
///     detect_membership_divergences(&[&epic, &inside, &outside], &config);
///
/// // `outside` claims epic:auth but the epic does not depend on it.
/// assert_eq!(divergences.len(), 1);
/// assert_eq!(divergences[0].issue_id, outside.id);
/// assert_eq!(divergences[0].label, "epic:auth");
/// ```
pub fn detect_membership_divergences(
    issues: &[&crate::domain::Issue],
    config: &HierarchyConfig,
) -> Vec<MembershipDivergence> {
    use crate::labels::{parse_label, type_label_values};

    let by_id: HashMap<&str, &crate::domain::Issue> =
        issues.iter().map(|i| (i.id.as_str(), *i)).collect();

    // Membership namespaces declared by the config (e.g. {"epic", "milestone"}).
    let membership_namespaces: HashSet<&str> = config
        .membership_namespaces()
        .map(|(_, ns)| ns.as_str())
        .collect();

    // Anchor containers per label: an issue whose type maps to namespace `ns`
    // and that carries label `ns:val` anchors that membership group.
    let mut anchors: HashMap<String, Vec<&str>> = HashMap::new();
    for issue in issues.iter().copied() {
        for type_value in type_label_values(&issue.labels) {
            let Some(ns) = config.get_membership_namespace(type_value) else {
                continue;
            };
            for label in &issue.labels {
                if let Ok((label_ns, _)) = parse_label(label) {
                    if label_ns == ns {
                        anchors
                            .entry(label.clone())
                            .or_default()
                            .push(issue.id.as_str());
                    }
                }
            }
        }
    }

    // Dependency closures for anchor containers, computed on demand and cached.
    let mut closure_cache: HashMap<&str, HashSet<String>> = HashMap::new();

    let mut divergences = Vec::new();
    for issue in issues.iter().copied() {
        for label in &issue.labels {
            let Ok((namespace, value)) = parse_label(label) else {
                continue;
            };
            if !membership_namespaces.contains(namespace.as_str()) {
                continue;
            }
            let Some(anchor_ids) = anchors.get(label) else {
                continue; // no anchor container → not a divergence here
            };
            if anchor_ids.contains(&issue.id.as_str()) {
                continue; // the issue is itself the anchor
            }
            let contained = anchor_ids.iter().any(|anchor| {
                closure_cache
                    .entry(anchor)
                    .or_insert_with(|| dependency_closure(anchor, &by_id))
                    .contains(&issue.id)
            });
            if !contained {
                divergences.push(MembershipDivergence {
                    issue_id: issue.id.clone(),
                    label: label.clone(),
                    namespace,
                    value,
                });
            }
        }
    }

    divergences.sort_by(|a, b| {
        a.issue_id
            .cmp(&b.issue_id)
            .then_with(|| a.label.cmp(&b.label))
    });
    divergences
}

/// The set of ids `start` transitively depends on (its DAG descendants),
/// excluding `start` itself.
fn dependency_closure(
    start: &str,
    by_id: &HashMap<&str, &crate::domain::Issue>,
) -> HashSet<String> {
    let mut result: HashSet<String> = HashSet::new();
    let mut stack: Vec<&str> = vec![start];
    let mut visited: HashSet<&str> = HashSet::new();
    visited.insert(start);
    while let Some(cur) = stack.pop() {
        if let Some(node) = by_id.get(cur) {
            for dep in &node.dependencies {
                let dep = dep.as_str();
                if by_id.contains_key(dep) && visited.insert(dep) {
                    result.insert(dep.to_string());
                    stack.push(dep);
                }
            }
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domain::Issue;

    /// A minimal [`HierarchyNode`] double so resolution can be tested without
    /// building full issues.
    struct TestNode {
        id: String,
        deps: Vec<String>,
        type_name: Option<String>,
    }

    impl TestNode {
        fn new(id: &str, type_name: Option<&str>, deps: &[&str]) -> Self {
            Self {
                id: id.to_string(),
                deps: deps.iter().map(|s| s.to_string()).collect(),
                type_name: type_name.map(str::to_string),
            }
        }
    }

    impl HierarchyNode for TestNode {
        fn id(&self) -> &str {
            &self.id
        }
        fn dependencies(&self) -> &[String] {
            &self.deps
        }
        fn type_name(&self) -> Option<&str> {
            self.type_name.as_deref()
        }
    }

    fn default_config() -> HierarchyConfig {
        HierarchyConfig::default()
    }

    #[test]
    fn test_resolve_chain_parent_and_cluster() {
        // milestone → epic → story → task
        let m = TestNode::new("m", Some("milestone"), &["e"]);
        let e = TestNode::new("e", Some("epic"), &["s"]);
        let s = TestNode::new("s", Some("story"), &["t"]);
        let t = TestNode::new("t", Some("task"), &[]);
        let r = resolve_hierarchy(&[&m, &e, &s, &t], &default_config());

        assert_eq!(r.parent("t"), Some("s"));
        assert_eq!(r.parent("s"), Some("e"));
        assert_eq!(r.parent("e"), Some("m"));
        assert_eq!(r.parent("m"), None);

        // Every node in the chain clusters to the strategic root.
        assert_eq!(r.cluster("t"), Some("m"));
        assert_eq!(r.cluster("s"), Some("m"));
        assert_eq!(r.cluster("e"), Some("m"));
        assert_eq!(r.cluster("m"), Some("m"));
    }

    #[test]
    fn test_children_are_inverse_of_parent() {
        let e = TestNode::new("e", Some("epic"), &["t1", "t2"]);
        let t1 = TestNode::new("t1", Some("task"), &[]);
        let t2 = TestNode::new("t2", Some("task"), &[]);
        let r = resolve_hierarchy(&[&e, &t1, &t2], &default_config());

        assert_eq!(r.children("e"), ["t1".to_string(), "t2".to_string()]);
        assert!(r.children("t1").is_empty());
    }

    #[test]
    fn test_nearest_container_wins_over_strategic() {
        // Both the epic and the milestone directly depend on the task; the
        // NEAREST (deepest level) container — the epic — is the parent.
        let m = TestNode::new("m", Some("milestone"), &["e", "t"]);
        let e = TestNode::new("e", Some("epic"), &["t"]);
        let t = TestNode::new("t", Some("task"), &[]);
        let r = resolve_hierarchy(&[&m, &e, &t], &default_config());

        assert_eq!(r.parent("t"), Some("e"));
        assert_eq!(r.children("e"), ["t".to_string()]);
        assert_eq!(r.children("m"), ["e".to_string()]);
    }

    #[test]
    fn test_direct_container_wins_over_transitive_cross_edge() {
        // Epic `ex` directly contains `tx` (ex→tx). A deeper story `sy` only
        // reaches `tx` transitively through a cross-cutting dependency
        // (sy→ty→tx). The direct container wins even though the story is deeper.
        let ex = TestNode::new("ex", Some("epic"), &["tx"]);
        let sy = TestNode::new("sy", Some("story"), &["ty"]);
        let ty = TestNode::new("ty", Some("task"), &["tx"]);
        let tx = TestNode::new("tx", Some("task"), &[]);
        let r = resolve_hierarchy(&[&ex, &sy, &ty, &tx], &default_config());

        assert_eq!(
            r.parent("tx"),
            Some("ex"),
            "direct epic beats transitive story"
        );
        assert_eq!(r.parent("ty"), Some("sy"));
        assert_eq!(r.children("ex"), ["tx".to_string()]);
    }

    #[test]
    fn test_diamond_tie_broken_by_id() {
        // Two epics at the same level, same hop distance → smallest id wins.
        let a = TestNode::new("aaa", Some("epic"), &["t"]);
        let b = TestNode::new("bbb", Some("epic"), &["t"]);
        let t = TestNode::new("t", Some("task"), &[]);
        let r = resolve_hierarchy(&[&a, &b, &t], &default_config());

        assert_eq!(r.parent("t"), Some("aaa"));
        assert_eq!(r.children("aaa"), ["t".to_string()]);
        assert!(r.children("bbb").is_empty());
    }

    #[test]
    fn test_root_leaf_is_orphan() {
        // A task with no container ancestor has no parent and no cluster.
        let t = TestNode::new("t", Some("task"), &[]);
        let r = resolve_hierarchy(&[&t], &default_config());
        assert_eq!(r.parent("t"), None);
        assert_eq!(r.cluster("t"), None);
        assert_eq!(r.rank("t"), Some(0));
    }

    #[test]
    fn test_untyped_node_is_leaf() {
        // A node with no type label is treated as a leaf, never a container.
        let untyped = TestNode::new("u", None, &["t"]);
        let t = TestNode::new("t", Some("task"), &[]);
        let r = resolve_hierarchy(&[&untyped, &t], &default_config());
        assert_eq!(r.parent("t"), None, "an untyped node is not a container");
        assert_eq!(r.cluster("u"), None);
    }

    #[test]
    fn test_rank_longest_path() {
        // m → e → t and m → t (direct); the longest path from m is 2.
        let m = TestNode::new("m", Some("milestone"), &["e", "t"]);
        let e = TestNode::new("e", Some("epic"), &["t"]);
        let t = TestNode::new("t", Some("task"), &[]);
        let r = resolve_hierarchy(&[&m, &e, &t], &default_config());
        assert_eq!(r.rank("t"), Some(0));
        assert_eq!(r.rank("e"), Some(1));
        assert_eq!(r.rank("m"), Some(2));
    }

    #[test]
    fn test_rejected_container_still_resolves_structurally() {
        // State is out of scope for resolution: a rejected epic still contains
        // its task. Resolution operates on Issues, so build them here.
        let mut epic = Issue::new("Epic".into(), String::new());
        epic.labels = vec!["type:epic".into()];
        epic.state = crate::domain::State::Rejected;
        let mut task = Issue::new("Task".into(), String::new());
        task.labels = vec!["type:task".into()];
        epic.dependencies = vec![task.id.clone()];

        let r = resolve_hierarchy(&[&epic, &task], &default_config());
        assert_eq!(r.parent(&task.id), Some(epic.id.as_str()));
    }

    #[test]
    fn test_divergence_flags_label_without_dag_membership() {
        let config = default_config();
        let mut epic = Issue::new("Auth".into(), String::new());
        epic.labels = vec!["type:epic".into(), "epic:auth".into()];
        let mut inside = Issue::new("Inside".into(), String::new());
        inside.labels = vec!["type:task".into(), "epic:auth".into()];
        let mut outside = Issue::new("Outside".into(), String::new());
        outside.labels = vec!["type:task".into(), "epic:auth".into()];
        epic.dependencies = vec![inside.id.clone()];

        let divergences = detect_membership_divergences(&[&epic, &inside, &outside], &config);
        assert_eq!(divergences.len(), 1);
        assert_eq!(divergences[0].issue_id, outside.id);
        assert_eq!(divergences[0].namespace, "epic");
        assert_eq!(divergences[0].value, "auth");
    }

    #[test]
    fn test_divergence_transitive_membership_is_not_flagged() {
        // A task reachable transitively (epic → story → task) is a DAG member.
        let config = default_config();
        let mut epic = Issue::new("Auth".into(), String::new());
        epic.labels = vec!["type:epic".into(), "epic:auth".into()];
        let mut story = Issue::new("Story".into(), String::new());
        story.labels = vec!["type:story".into()];
        let mut task = Issue::new("Task".into(), String::new());
        task.labels = vec!["type:task".into(), "epic:auth".into()];
        epic.dependencies = vec![story.id.clone()];
        story.dependencies = vec![task.id.clone()];

        let divergences = detect_membership_divergences(&[&epic, &story, &task], &config);
        assert!(
            divergences.is_empty(),
            "transitive members are not divergent"
        );
    }

    #[test]
    fn test_divergence_no_anchor_is_not_flagged() {
        // A label with no anchor container is left to membership-reference
        // validation, not reported as a divergence.
        let config = default_config();
        let mut task = Issue::new("Task".into(), String::new());
        task.labels = vec!["type:task".into(), "epic:ghost".into()];
        let divergences = detect_membership_divergences(&[&task], &config);
        assert!(divergences.is_empty());
    }
}

#[cfg(test)]
mod proptests {
    //! Property-based coverage for hierarchy resolution over arbitrary DAGs,
    //! mirroring the graph proptest suite in [`crate::type_hierarchy`].
    use super::*;
    use crate::type_hierarchy::HierarchyConfig;
    use proptest::prelude::*;
    use std::collections::{HashMap, HashSet};

    /// A minimal [`HierarchyNode`] for generated graphs.
    struct PropNode {
        id: String,
        deps: Vec<String>,
        type_name: Option<String>,
    }

    impl HierarchyNode for PropNode {
        fn id(&self) -> &str {
            &self.id
        }
        fn dependencies(&self) -> &[String] {
            &self.deps
        }
        fn type_name(&self) -> Option<&str> {
            self.type_name.as_deref()
        }
    }

    /// Generate an arbitrary acyclic graph: `n` nodes where node `k` may only
    /// depend on lower-indexed nodes (guaranteeing acyclicity). Each node gets an
    /// arbitrary type from the default hierarchy, or none.
    fn arbitrary_dag() -> impl Strategy<Value = Vec<(Option<&'static str>, Vec<usize>)>> {
        (1usize..=7)
            .prop_flat_map(|n| {
                (
                    Just(n),
                    prop::collection::vec(0u8..5, n),
                    prop::collection::vec(any::<u8>(), n),
                )
            })
            .prop_map(|(n, types, masks)| {
                (0..n)
                    .map(|k| {
                        let ty = match types[k] {
                            0 => Some("milestone"),
                            1 => Some("epic"),
                            2 => Some("story"),
                            3 => Some("task"),
                            _ => None,
                        };
                        // Depend on a subset of the strictly-lower indices.
                        let deps = (0..k).filter(|&i| masks[k] & (1u8 << i) != 0).collect();
                        (ty, deps)
                    })
                    .collect()
            })
    }

    fn build(spec: &[(Option<&'static str>, Vec<usize>)]) -> Vec<PropNode> {
        spec.iter()
            .enumerate()
            .map(|(k, (ty, deps))| PropNode {
                id: format!("n{k}"),
                deps: deps.iter().map(|i| format!("n{i}")).collect(),
                type_name: ty.map(str::to_string),
            })
            .collect()
    }

    /// Independent longest-path recomputation (the resolution's `rank` oracle).
    fn longest_path(
        id: &str,
        by_id: &HashMap<&str, &PropNode>,
        memo: &mut HashMap<String, u32>,
    ) -> u32 {
        if let Some(v) = memo.get(id) {
            return *v;
        }
        let mut best = 0;
        if let Some(node) = by_id.get(id) {
            for dep in &node.deps {
                if by_id.contains_key(dep.as_str()) {
                    best = best.max(1 + longest_path(dep, by_id, memo));
                }
            }
        }
        memo.insert(id.to_string(), best);
        best
    }

    /// Ids `start` transitively depends on (excluding `start`).
    fn dependency_closure(start: &str, by_id: &HashMap<&str, &PropNode>) -> HashSet<String> {
        let mut result = HashSet::new();
        let mut stack = vec![start];
        let mut seen = HashSet::new();
        seen.insert(start);
        while let Some(cur) = stack.pop() {
            if let Some(node) = by_id.get(cur) {
                for dep in &node.deps {
                    let dep = dep.as_str();
                    if by_id.contains_key(dep) && seen.insert(dep) {
                        result.insert(dep.to_string());
                        stack.push(dep);
                    }
                }
            }
        }
        result
    }

    proptest! {
        /// Resolution never panics on arbitrary acyclic input, and each resolved
        /// fact satisfies its definition: every parent is a container that
        /// transitively depends on the node; children invert parent; and rank
        /// equals the independently-computed longest path.
        #[test]
        fn prop_resolution_invariants(spec in arbitrary_dag()) {
            let config = HierarchyConfig::default();
            let nodes = build(&spec);
            let refs: Vec<&PropNode> = nodes.iter().collect();
            let resolution = resolve_hierarchy(&refs, &config);

            // Every input node is resolved.
            prop_assert_eq!(resolution.len(), nodes.len());

            let by_id: HashMap<&str, &PropNode> = nodes.iter().map(|n| (n.id(), n)).collect();
            let leaf_level = config.types().map(|(_, l)| *l).max().unwrap();
            let is_container = |id: &str| -> bool {
                matches!(
                    by_id.get(id).and_then(|n| n.type_name()).and_then(|t| config.get_level(t)),
                    Some(l) if l < leaf_level
                )
            };

            for node in &nodes {
                let facts = resolution.get(node.id()).unwrap();

                // A resolved parent is a container whose closure includes the node.
                if let Some(parent) = &facts.parent {
                    prop_assert!(is_container(parent), "parent {} is not a container", parent);
                    prop_assert!(
                        dependency_closure(parent, &by_id).contains(node.id()),
                        "parent {} does not transitively depend on {}",
                        parent,
                        node.id()
                    );
                }

                // Rank equals the longest dependency path.
                let mut memo = HashMap::new();
                prop_assert_eq!(facts.rank, longest_path(node.id(), &by_id, &mut memo));

                // Children invert parent, and are sorted.
                for child in &facts.children {
                    prop_assert_eq!(resolution.parent(child), Some(node.id()));
                }
                let mut sorted = facts.children.clone();
                sorted.sort();
                prop_assert_eq!(&facts.children, &sorted);
            }

            // Every parented node appears under its parent's children.
            for node in &nodes {
                if let Some(parent) = resolution.parent(node.id()) {
                    prop_assert!(resolution.children(parent).iter().any(|c| c == node.id()));
                }
            }
        }

        /// Resolution is invariant under input reordering: the tie-break is by
        /// node id, not vector position, so a reversed input yields identical
        /// facts for every node.
        #[test]
        fn prop_resolution_is_order_invariant(spec in arbitrary_dag()) {
            let config = HierarchyConfig::default();
            let nodes = build(&spec);
            let refs: Vec<&PropNode> = nodes.iter().collect();
            let forward = resolve_hierarchy(&refs, &config);

            let mut reversed = refs.clone();
            reversed.reverse();
            let backward = resolve_hierarchy(&reversed, &config);

            for node in &nodes {
                prop_assert_eq!(forward.get(node.id()), backward.get(node.id()));
            }
        }
    }
}
