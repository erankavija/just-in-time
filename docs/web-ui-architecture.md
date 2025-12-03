# JIT Web UI Architecture & Visual Design

**Date**: 2025-12-03  
**Status**: Planning Phase  
**Goal**: Interactive web interface for issue tracking with dependency graph visualization

---

## 🎨 Visual Layout Overview

```
┌─────────────────────────────────────────────────────────────────────────┐
│                          JIT Issue Tracker                               │
│  [🏠 Home]  [📊 Graph]  [📋 Issues]  [🔍 Search]  [⚙️ Settings]        │
├──────────────────────────┬──────────────────────────────────────────────┤
│                          │                                              │
│    DEPENDENCY GRAPH      │         ISSUE DETAIL PANEL                  │
│         (Left)           │              (Right)                        │
│                          │                                              │
│  ┌────────────────────┐  │  Issue: #01ABC                              │
│  │   ┌─────┐          │  │  ━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━  │
│  │   │Issue│          │  │  Title: Implement Authentication           │
│  │   │ 01  │          │  │  State: 🟢 Ready                           │
│  │   └──┬──┘          │  │  Priority: 🔴 High                         │
│  │      │              │  │  Assignee: @alice                          │
│  │   ┌──▼──┐  ┌────┐  │  │                                            │
│  │   │Issue│  │Iss │  │  │  Description:                              │
│  │   │ 02  │  │ 03 │  │  │  ┌──────────────────────────────────────┐ │
│  │   └──┬──┘  └─┬──┘  │  │  │ Implement OAuth2 authentication...   │ │
│  │      │       │      │  │  │ - JWT tokens                         │ │
│  │   ┌──▼───────▼──┐   │  │  │ - Session management                │ │
│  │   │   Issue 04  │   │  │  └──────────────────────────────────────┘ │
│  │   │   (Blocked)  │   │  │                                            │
│  │   └─────────────┘   │  │  📄 Documents (3):                         │
│  │                      │  │  • docs/auth-design.md (Design Doc)       │
│  │  [Zoom] [Center]    │  │    [View Content] [@ HEAD]                │
│  │  [Filter: All]      │  │  • notes/oauth-flow.md (Implementation)   │
│  │                      │  │    [View Content] [@ a1b2c3d]             │
│  └────────────────────┘  │  • diagrams/auth-sequence.png             │
│                          │    [View Image] [@ HEAD]                   │
│      Graph Controls      │                                              │
│  • 🔵 Open               │  🔗 Dependencies (2):                       │
│  • 🟢 Ready              │  • Issue #00XYZ: Setup database            │
│  • 🟡 In Progress        │  • Issue #00ABC: Create user model         │
│  • ✅ Done               │                                              │
│  • 🔴 Blocked            │  ⚡ Gates (2/3 passed):                     │
│                          │  • ✅ code_review                           │
│                          │  • ✅ unit_tests                            │
│                          │  • ⏳ integration_tests (pending)           │
│                          │                                              │
│                          │  [Edit] [Close] [Claim] [Transition]       │
└──────────────────────────┴──────────────────────────────────────────────┘

                    STATUS BAR: 45 issues • 12 ready • 5 in progress
```

---

## 🏗️ Architecture: 3-Tier Stack

```
┌─────────────────────────────────────────────────────────────────────┐
│                         FRONTEND (React + TS)                        │
│  ┌──────────────┐  ┌──────────────┐  ┌──────────────┐             │
│  │ Graph View   │  │ Issue Detail │  │ Doc Viewer   │             │
│  │ (React Flow) │  │ Panel        │  │ (Markdown)   │             │
│  └──────┬───────┘  └──────┬───────┘  └──────┬───────┘             │
│         │                  │                  │                      │
│         └──────────────────┼──────────────────┘                      │
│                            │                                         │
│                    ┌───────▼────────┐                               │
│                    │  API Client    │                               │
│                    │  (axios/fetch) │                               │
│                    └───────┬────────┘                               │
└────────────────────────────┼─────────────────────────────────────────┘
                             │ HTTP/JSON
┌────────────────────────────▼─────────────────────────────────────────┐
│                      REST API SERVER (Rust)                          │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │                    Axum Web Framework                        │   │
│  └──────────────────────────────────────────────────────────────┘   │
│                                                                      │
│  Routes:                                                             │
│  • GET  /api/issues          → List all issues                     │
│  • GET  /api/issues/:id      → Get issue details                   │
│  • GET  /api/graph           → Get dependency graph                │
│  • GET  /api/documents/:id/:path → Get document content            │
│  • GET  /api/validate        → Validation report                   │
│  • POST /api/search          → Search issues/docs                  │
│                                                                      │
│  ┌──────────────────────────────────────────────────────────────┐   │
│  │              CommandExecutor (jit library)                   │   │
│  └────────────────────────┬─────────────────────────────────────┘   │
└───────────────────────────┼──────────────────────────────────────────┘
                            │
┌───────────────────────────▼──────────────────────────────────────────┐
│                   STORAGE LAYER (JSON Files)                         │
│  • .jit/index.json         → Issue index                            │
│  • .jit/issues/*.json      → Individual issues                      │
│  • .jit/gates.json         → Gate registry                          │
│  • .jit/events.jsonl       → Event log                              │
│  • Git repository          → Document content                       │
└──────────────────────────────────────────────────────────────────────┘
```

---

## 📦 Technology Stack

### Backend (Rust)
```toml
# crates/server/Cargo.toml
[dependencies]
jit = { path = "../jit" }        # Core library
axum = "0.7"                      # Web framework
tokio = "1.35"                    # Async runtime
tower-http = "0.5"                # CORS, middleware
serde = "1.0"                     # JSON serialization
serde_json = "1.0"
anyhow = "1.0"
```

### Frontend (TypeScript + React)
```json
{
  "dependencies": {
    "react": "^18.2.0",
    "react-dom": "^18.2.0",
    "react-router-dom": "^6.20.0",
    "reactflow": "^11.10.0",       // Graph visualization
    "react-markdown": "^9.0.0",    // Markdown rendering
    "axios": "^1.6.0",              // HTTP client
    "prismjs": "^1.29.0"           // Code highlighting
  },
  "devDependencies": {
    "@types/react": "^18.2.0",
    "typescript": "^5.3.0",
    "vite": "^5.0.0"               // Build tool
  }
}
```

---

## 🎯 Phase 2 Implementation Plan

### Sprint 1: REST API Server (Week 1)
**Goal**: Read-only API serving issue data

#### Day 1-2: Project Setup
- Create `crates/server` directory
- Initialize Cargo.toml with dependencies
- Basic Axum server with health check endpoint
- CORS configuration for local development

#### Day 3-4: Core Endpoints
```rust
// GET /api/issues - List all issues
async fn list_issues(State(executor): State<Arc<CommandExecutor>>) 
    -> Result<Json<Vec<Issue>>, StatusCode>

// GET /api/issues/:id - Get single issue
async fn get_issue(Path(id): Path<String>, State(executor): State<Arc<CommandExecutor>>) 
    -> Result<Json<Issue>, StatusCode>

// GET /api/graph - Get dependency graph
async fn get_graph(State(executor): State<Arc<CommandExecutor>>) 
    -> Result<Json<GraphData>, StatusCode>
```

#### Day 5: Document Endpoints
```rust
// GET /api/documents/:id/:path - Get document content
async fn get_document(
    Path((issue_id, doc_path)): Path<(String, String)>,
    State(executor): State<Arc<CommandExecutor>>
) -> Result<Response<String>, StatusCode>
```

**Deliverable**: Working API server on `localhost:3000`  
**Tests**: 20-25 API integration tests  
**Estimated**: 40 hours (1 week)

---

### Sprint 2: Frontend Foundation (Week 2)
**Goal**: Basic UI with graph and issue viewing

#### Day 1: Project Setup
```bash
npm create vite@latest web -- --template react-ts
cd web
npm install reactflow react-markdown axios prismjs
npm install @types/prismjs -D
```

#### Day 2-3: Graph Component
```typescript
// src/components/Graph/GraphView.tsx
export function GraphView() {
  const [nodes, setNodes] = useState<Node[]>([]);
  const [edges, setEdges] = useState<Edge[]>([]);
  
  // Fetch graph data from API
  useEffect(() => {
    axios.get('/api/graph').then(response => {
      const { nodes, edges } = transformGraphData(response.data);
      setNodes(nodes);
      setEdges(edges);
    });
  }, []);

  return (
    <ReactFlow 
      nodes={nodes} 
      edges={edges}
      onNodeClick={handleNodeClick}
      nodeTypes={customNodeTypes}
    />
  );
}
```

#### Day 4-5: Issue Detail Panel
```typescript
// src/components/Issue/IssueDetail.tsx
export function IssueDetail({ issueId }: { issueId: string }) {
  const [issue, setIssue] = useState<Issue | null>(null);

  return (
    <div className="issue-detail">
      <h1>{issue.title}</h1>
      <StatusBadge state={issue.state} />
      <PriorityBadge priority={issue.priority} />
      
      <section className="description">
        <ReactMarkdown>{issue.description}</ReactMarkdown>
      </section>

      <DocumentList documents={issue.documents} />
      <DependencyList dependencies={issue.dependencies} />
      <GateStatus gates={issue.gates_status} />
    </div>
  );
}
```

**Deliverable**: Working UI on `localhost:5173`  
**Estimated**: 40 hours (1 week)

---

## 🎨 Visual Design Patterns

### Node Styling by State
```typescript
const nodeColors = {
  open: '#3B82F6',      // Blue
  ready: '#10B981',     // Green
  in_progress: '#F59E0B', // Amber
  done: '#6B7280',      // Gray
  archived: '#9CA3AF'   // Light gray
};

const nodeStyles = (state: State) => ({
  border: `2px solid ${nodeColors[state]}`,
  borderRadius: '8px',
  padding: '12px',
  background: 'white',
  boxShadow: '0 2px 4px rgba(0,0,0,0.1)'
});
```

### Priority Indicators
```
🔴 Critical   (Red dot)
🟠 High       (Orange dot)
🟡 Normal     (Yellow dot)
🟢 Low        (Green dot)
```

### Gate Status Display
```
✅ Passed     (Green checkmark)
⏳ Pending    (Yellow clock)
❌ Failed     (Red X)
```

---

## 🚀 Features by Priority

### MVP (Must Have)
- ✅ Dependency graph visualization
- ✅ Issue detail panel
- ✅ Document list with view links
- ✅ Basic navigation
- ✅ State/priority indicators

### Phase 2.1 (Should Have)
- Click node → show issue details
- Hover → show title tooltip
- Zoom/pan controls
- Filter by state/priority
- Search by title

### Phase 2.2 (Nice to Have)
- Markdown document viewer
- Code syntax highlighting
- Side-by-side view (graph + doc)
- Dark mode
- Export graph as PNG

### Future (Phase 3+)
- Full-text search
- Historical document viewer (time travel)
- Real-time updates (WebSocket)
- Keyboard shortcuts
- Mobile responsive design

---

## 📊 Data Flow Example

### Loading Issue Detail
```
User clicks node "Issue #01ABC"
         │
         ▼
GraphView.onNodeClick(nodeId)
         │
         ▼
setSelectedIssueId("01ABC")
         │
         ▼
IssueDetail component renders
         │
         ▼
useEffect → axios.get('/api/issues/01ABC')
         │
         ▼
Server: CommandExecutor.show_issue("01ABC")
         │
         ▼
Storage.load_issue("01ABC") → JSON file
         │
         ▼
Returns Issue JSON
         │
         ▼
Frontend: setIssue(response.data)
         │
         ▼
UI updates with issue details
```

---

## 🧪 Testing Strategy

### Backend Tests
```rust
#[tokio::test]
async fn test_list_issues_endpoint() {
    let app = create_test_app().await;
    let response = app
        .oneshot(Request::builder()
            .uri("/api/issues")
            .body(Body::empty())
            .unwrap())
        .await
        .unwrap();
    
    assert_eq!(response.status(), StatusCode::OK);
}
```

### Frontend Tests
```typescript
describe('GraphView', () => {
  it('renders nodes from API', async () => {
    render(<GraphView />);
    await waitFor(() => {
      expect(screen.getAllByRole('button')).toHaveLength(3);
    });
  });
});
```

---

## 📁 Final Directory Structure

```
just-in-time/
├── crates/
│   ├── jit/              # Core library
│   ├── dispatch/         # Orchestrator
│   └── server/           # NEW: Web API server
│       ├── src/
│       │   ├── main.rs
│       │   ├── routes.rs
│       │   ├── handlers/
│       │   │   ├── issues.rs
│       │   │   ├── graph.rs
│       │   │   └── documents.rs
│       │   └── error.rs
│       └── Cargo.toml
├── web/                  # NEW: Frontend
│   ├── src/
│   │   ├── components/
│   │   │   ├── Graph/
│   │   │   │   ├── GraphView.tsx
│   │   │   │   └── NodeRenderer.tsx
│   │   │   ├── Issue/
│   │   │   │   ├── IssueDetail.tsx
│   │   │   │   ├── DocumentList.tsx
│   │   │   │   └── DependencyList.tsx
│   │   │   └── Document/
│   │   │       └── MarkdownViewer.tsx
│   │   ├── api/
│   │   │   └── client.ts
│   │   ├── types/
│   │   │   └── models.ts
│   │   ├── App.tsx
│   │   └── main.tsx
│   ├── index.html
│   ├── package.json
│   └── vite.config.ts
└── README.md
```

---

## 🎬 Getting Started (Future)

```bash
# Terminal 1: Start API server
cd crates/server
cargo run
# Server running on http://localhost:3000

# Terminal 2: Start frontend dev server
cd web
npm run dev
# UI running on http://localhost:5173

# Open browser to http://localhost:5173
```

---

## 📈 Success Metrics

- ✅ Graph renders all issues correctly
- ✅ Click interaction works smoothly
- ✅ API responds in <100ms
- ✅ Document content displays correctly
- ✅ UI works on modern browsers (Chrome, Firefox, Safari)
- ✅ Zero console errors
- ✅ Responsive on desktop (tablet/mobile Phase 3)

---

**Ready to start implementation? Let's begin with Phase 2.1: REST API Server!**
