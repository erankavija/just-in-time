export type State = 'backlog' | 'ready' | 'in_progress' | 'gated' | 'done' | 'archived';
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
