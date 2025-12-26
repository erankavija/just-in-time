export type State = 'backlog' | 'ready' | 'in_progress' | 'gated' | 'done' | 'rejected' | 'archived';
export type Priority = 'low' | 'normal' | 'high' | 'critical';

export interface DocumentReference {
  path: string;
  commit?: string;
  label?: string;
  doc_type?: string;
}

export interface Gate {
  key: string;
  name: string;
  description?: string;
}

export interface GateStatus {
  gate_key: string;
  state: 'pending' | 'passed' | 'failed';
}

export interface Issue {
  id: string;
  title: string;
  description: string;
  state: State;
  priority: Priority;
  assignee?: string;
  dependencies: string[];
  labels: string[];
  documents: DocumentReference[];
  gates: string[];
  gates_status: GateStatus[];
  created_at: string;
  updated_at: string;
}

export interface GraphNode {
  id: string;
  label: string;
  state: State;
  priority: Priority;
  assignee?: string;
  labels: string[];
  blocked: boolean;
}

export interface GraphEdge {
  from: string;
  to: string;
}

export interface GraphData {
  nodes: GraphNode[];
  edges: GraphEdge[];
}

export interface StatusSummary {
  total_issues: number;
  by_state: Record<State, number>;
  by_priority: Record<Priority, number>;
  ready_count: number;
  blocked_count: number;
}

export interface SearchMatch {
  text: string;
  start: number;
  end: number;
}

export interface SearchResult {
  issue_id?: string;
  path: string;
  line_number: number;
  line_text: string;
  matches: SearchMatch[];
}

export interface SearchResponse {
  query: string;
  total: number;
  results: SearchResult[];
  duration_ms: number;
}

export interface DocumentContent {
  path: string;
  commit: string;
  content: string;
  content_type: string;
}

export interface CommitInfo {
  commit: string;
  author: string;
  date: string;
  message: string;
}

export interface DocumentHistory {
  path: string;
  commits: CommitInfo[];
}

export interface DocumentDiff {
  path: string;
  from: string;
  to: string;
  diff: string;
}

export interface NamespaceInfo {
  description: string;
  unique: boolean;
}

export interface ConfigStrategicTypes {
  strategic_types: string[];
}

export interface ConfigHierarchy {
  types: Record<string, number>;
  strategic_types: string[];
}

export interface ConfigNamespaces {
  namespaces: Record<string, NamespaceInfo>;
}
