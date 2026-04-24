import axios from 'axios';
import type { Issue, GraphData, StatusSummary, DocumentContent, DocumentHistory, DocumentDiff, ConfigHierarchy, ConfigNamespaces, GateDefinition, GateRunSummary, GateRunDetail } from '../types/models';

// Use the origin the page was served from so that multiple jit servers
// running on different ports each talk to their own API.
export const API_BASE = `${window.location.origin}/api`;

const api = axios.create({
  baseURL: API_BASE,
  headers: {
    'Content-Type': 'application/json',
  },
});

export const apiClient = {
  async getHealth(): Promise<{ status: string; project_name?: string }> {
    const response = await api.get('/health');
    return response.data;
  },

  async listIssues(): Promise<Issue[]> {
    const response = await api.get('/issues');
    return response.data;
  },

  async getIssue(id: string): Promise<Issue> {
    const response = await api.get(`/issues/${id}`);
    return response.data;
  },

  async getGraph(): Promise<GraphData> {
    const response = await api.get('/graph');
    return response.data;
  },

  async getStatus(): Promise<StatusSummary> {
    const response = await api.get('/status');
    return response.data;
  },

  async search(query: string, options?: { limit?: number; caseSensitive?: boolean; regex?: boolean }): Promise<import('../types/models').SearchResponse> {
    const params = new URLSearchParams();
    params.set('q', query);
    if (options?.limit) params.set('limit', options.limit.toString());
    if (options?.caseSensitive) params.set('case_sensitive', 'true');
    if (options?.regex) params.set('regex', 'true');
    
    const response = await api.get(`/search?${params.toString()}`);
    return response.data;
  },

  async getDocumentContent(
    issueId: string,
    path: string,
    commit?: string
  ): Promise<DocumentContent> {
    const params = new URLSearchParams();
    if (commit) params.set('commit', commit);
    const encodedPath = encodeURIComponent(path);
    const url = `/issues/${issueId}/documents/${encodedPath}/content${params.toString() ? '?' + params.toString() : ''}`;
    const response = await api.get(url);
    return response.data;
  },

  async getDocumentHistory(
    issueId: string,
    path: string
  ): Promise<DocumentHistory> {
    const encodedPath = encodeURIComponent(path);
    const response = await api.get(`/issues/${issueId}/documents/${encodedPath}/history`);
    return response.data;
  },

  async getDocumentDiff(
    issueId: string,
    path: string,
    from: string,
    to?: string
  ): Promise<DocumentDiff> {
    const params = new URLSearchParams();
    params.set('from', from);
    if (to) params.set('to', to);
    const encodedPath = encodeURIComponent(path);
    const response = await api.get(`/issues/${issueId}/documents/${encodedPath}/diff?${params.toString()}`);
    return response.data;
  },

  async getDocumentByPath(
    path: string,
    commit?: string
  ): Promise<DocumentContent> {
    const params = new URLSearchParams();
    params.set('path', path);
    if (commit) params.set('commit', commit);
    const response = await api.get(`/documents?${params.toString()}`);
    return response.data;
  },

  async getHierarchy(): Promise<ConfigHierarchy> {
    const response = await api.get('/config/hierarchy');
    return response.data;
  },

  async getNamespaces(): Promise<ConfigNamespaces> {
    const response = await api.get('/config/namespaces');
    return response.data;
  },

  async listGates(): Promise<GateDefinition[]> {
    const response = await api.get('/gates');
    return response.data;
  },

  async getGateDefinition(key: string): Promise<GateDefinition> {
    const response = await api.get(`/gates/${encodeURIComponent(key)}`);
    return response.data;
  },

  async listGateRuns(issueId: string, gateKey?: string): Promise<GateRunSummary[]> {
    const params = new URLSearchParams();
    if (gateKey) params.set('gate_key', gateKey);
    const qs = params.toString();
    const response = await api.get(`/issues/${issueId}/gate-runs${qs ? '?' + qs : ''}`);
    return response.data;
  },

  async getGateRun(issueId: string, runId: string): Promise<GateRunDetail> {
    const response = await api.get(`/issues/${issueId}/gate-runs/${runId}`);
    return response.data;
  },
};

/**
 * Returns the URL for the raw bytes of a document attached to an issue.
 * When `issueId` is provided the URL uses the issue-scoped endpoint:
 *   /api/issues/<id>/documents/<encoded-path>/raw[?commit=…]
 * When `issueId` is omitted it falls back to the path-only endpoint:
 *   /api/documents/raw?path=<encoded>[&commit=…]
 *
 * @example
 * // Issue-scoped, latest commit:
 * getRawDocumentUrl('abc123', 'docs/presentations/deck.html');
 * // → '/api/issues/abc123/documents/docs%2Fpresentations%2Fdeck.html/raw'
 *
 * @example
 * // Issue-scoped pinned to a commit:
 * getRawDocumentUrl('abc123', 'docs/deck.html', 'a1b2c3d');
 * // → '/api/issues/abc123/documents/docs%2Fdeck.html/raw?commit=a1b2c3d'
 *
 * @example
 * // Path-only (no issue context):
 * getRawDocumentUrl(undefined, 'README.md');
 * // → '/api/documents/raw?path=README.md'
 */
export function getRawDocumentUrl(issueId: string, path: string, commit?: string): string;
export function getRawDocumentUrl(issueId: undefined | null, path: string, commit?: string): string;
export function getRawDocumentUrl(issueId: string | undefined | null, path: string, commit?: string): string {
  if (issueId) {
    const encodedPath = encodeURIComponent(path);
    const base = `/api/issues/${issueId}/documents/${encodedPath}/raw`;
    return commit ? `${base}?commit=${encodeURIComponent(commit)}` : base;
  }
  // Path-only variant
  const params = new URLSearchParams({ path });
  if (commit) params.set('commit', commit);
  return `/api/documents/raw?${params.toString()}`;
}
