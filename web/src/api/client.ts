import axios from 'axios';
import type { Issue, GraphData, StatusSummary, DocumentContent, DocumentHistory, DocumentDiff, ConfigHierarchy, ConfigNamespaces } from '../types/models';

// Use relative URL or construct from current host to avoid CORS issues
const API_BASE = window.location.hostname === 'localhost' 
  ? 'http://localhost:3000/api'
  : `http://${window.location.hostname}:3000/api`;

const api = axios.create({
  baseURL: API_BASE,
  headers: {
    'Content-Type': 'application/json',
  },
});

export const apiClient = {
  async getHealth(): Promise<{ status: string }> {
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

  async getStrategicTypes(): Promise<string[]> {
    const response = await api.get('/config/strategic-types');
    return response.data.strategic_types;
  },

  async getHierarchy(): Promise<ConfigHierarchy> {
    const response = await api.get('/config/hierarchy');
    return response.data;
  },

  async getNamespaces(): Promise<ConfigNamespaces> {
    const response = await api.get('/config/namespaces');
    return response.data;
  },
};
