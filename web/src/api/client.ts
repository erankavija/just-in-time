import axios from 'axios';
import type { Issue, GraphData, StatusSummary } from '../types/models';

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
};
