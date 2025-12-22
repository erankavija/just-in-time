# Knowledge Management & Historical Archive Vision

**Date**: 2025-12-02  
**Status**: Planning  
**Goal**: Transform JIT into an issue and knowledge management system with historical document tracking

## Vision Statement

Build a system where **issues are living documents** that reference design docs, implementation notes, and artifacts—all versioned and validated. Users can explore the dependency graph in a web UI, clicking through to view historical markdown documents inline, creating a comprehensive project knowledge base.

## Core Capabilities

### 1. **Document References in Issues**
Issues can reference files in the repository or archive:
- Design documents (e.g., `docs/api-design.md`)
- Implementation notes (e.g., `notes/auth-implementation.md`)
- Architecture diagrams (e.g., `diagrams/system-architecture.png`)
- Test plans, meeting notes, decision records

### 2. **Version-Aware References**
References can point to:
- **Current version**: `docs/design.md` (HEAD)
- **Specific commit**: `docs/design.md@a1b2c3d` (immutable historical reference)
- **Archived files**: Files that no longer exist in HEAD but are preserved in git history

### 3. **Graph Validation**
Ensure integrity of the knowledge base:
- All referenced files exist (or existed at specified commit)
- Broken links are detected and reported
- Circular dependencies in documents are prevented
- Git commit hashes are valid

### 4. **Interactive Web UI**
Visual exploration of project knowledge:
- **Graph view**: Interactive dependency graph (D3.js, React Flow, or Mermaid)
- **Issue detail**: Click issue → see description + inline document viewer
- **Document rendering**: Markdown rendered with syntax highlighting
- **Historical view**: Browse documents at specific commits
- **Search**: Full-text search across issues and referenced documents

## Development Action Plan

### Phase 1: Document References (Foundation)

**Goal**: Extend Issue model to support document references with validation

#### 1.1 Extend Domain Model
```rust
// New types in domain.rs
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentReference {
    /// Path relative to repo root
    pub path: String,
    /// Optional git commit hash (None = HEAD)
    pub commit: Option<String>,
    /// Human-readable label
    pub label: Option<String>,
    /// Document type hint (design, implementation, notes, etc.)
    pub doc_type: Option<String>,
}

// Add to Issue struct
pub struct Issue {
    // ... existing fields
    /// References to design documents, notes, and artifacts
    pub documents: Vec<DocumentReference>,
}
```

**Actions:**
- [ ] Add `DocumentReference` type to `domain.rs`
- [ ] Add `documents: Vec<DocumentReference>` field to `Issue`
- [ ] Update Issue serialization/deserialization
- [ ] Write unit tests for document reference model
- [ ] Update migration for existing issues (empty documents vec)

**Tests**: 5-10 new tests
**Estimated**: 2-3 hours

---

#### 1.2 CLI Commands for Document Management
```bash
# Add document reference to issue
jit doc add <issue-id> <path> [--commit <hash>] [--label "Design Doc"] [--type design]

# List documents for an issue
jit doc list <issue-id>

# Remove document reference
jit doc remove <issue-id> <path>

# Show document content (cat file at commit)
jit doc show <issue-id> <path>
```

**Actions:**
- [ ] Implement `doc add` command
- [ ] Implement `doc list` command
- [ ] Implement `doc remove` command
- [ ] Implement `doc show` command (reads from git)
- [ ] Update `issue show` to display document references
- [ ] Add `--json` support for all commands

**Tests**: 10-15 integration tests
**Estimated**: 4-6 hours

---

#### 1.3 Document Validation
```bash
# Validate all document references in issues
jit validate docs

# Check specific issue's references
jit validate docs <issue-id>

# Output: Missing files, invalid commits, broken links
```

**Actions:**
- [ ] Create `validation` module
- [ ] Implement git integration (use `git2-rs` crate)
- [ ] Check if file exists at HEAD
- [ ] Check if file existed at specific commit
- [ ] Validate commit hash format and existence
- [ ] Report validation errors with suggestions
- [ ] Add to existing `jit validate` command

**Dependencies**: `git2 = "0.18"` crate
**Tests**: 15-20 tests (mocking git operations)
**Estimated**: 6-8 hours

---

### Phase 2: Web UI Foundation

**Goal**: Basic web UI with graph visualization and document rendering

#### 2.1 REST API Server
```rust
// New crate: jit-server
// Endpoints:
GET  /api/issues              - List all issues
GET  /api/issues/:id          - Get issue details
GET  /api/graph               - Get dependency graph (nodes + edges)
GET  /api/documents/:id/:path - Get document content
GET  /api/validate            - Run validation, return report
POST /api/search              - Search issues and documents
```

**Actions:**
- [ ] Create `crates/server` with `axum` framework
- [ ] Implement read-only API endpoints
- [ ] Use `CommandExecutor` from jit library
- [ ] Add CORS support for local development
- [ ] JSON serialization for graph data
- [ ] Document content served with content-type detection
- [ ] Error handling with proper HTTP status codes

**Dependencies**: 
- `axum = "0.7"` - Web framework
- `tower-http = "0.5"` - CORS and middleware
- `tokio = "1.35"` - Async runtime

**Tests**: 20-25 API integration tests
**Estimated**: 8-10 hours

---

#### 2.2 Frontend Foundation (React + TypeScript)
```
web/
├── src/
│   ├── components/
│   │   ├── Graph/           # Dependency graph view
│   │   │   ├── GraphView.tsx
│   │   │   └── NodeRenderer.tsx
│   │   ├── Issue/           # Issue detail panel
│   │   │   ├── IssueDetail.tsx
│   │   │   └── DocumentList.tsx
│   │   └── Document/        # Document viewer
│   │       ├── MarkdownViewer.tsx
│   │       └── CodeViewer.tsx
│   ├── api/                 # API client
│   │   └── client.ts
│   ├── types/               # TypeScript types
│   │   └── models.ts
│   └── App.tsx
├── package.json
└── tsconfig.json
```

**Actions:**
- [ ] Initialize React + TypeScript project (Vite)
- [ ] Install dependencies (React Flow, React Markdown, Prism)
- [ ] Create basic layout (graph on left, detail on right)
- [ ] Implement API client with fetch/axios
- [ ] TypeScript types matching Rust models
- [ ] Basic routing (React Router)

**Dependencies**:
- `react = "^18.2"`
- `react-flow-renderer = "^11.10"` - Graph visualization
- `react-markdown = "^9.0"` - Markdown rendering
- `prismjs = "^1.29"` - Syntax highlighting
- `axios = "^1.6"` - HTTP client

**Estimated**: 6-8 hours

---

#### 2.3 Interactive Graph Visualization
**Actions:**
- [ ] Render dependency graph with React Flow
- [ ] Node styling by state (open, in-progress, done)
- [ ] Click node → show issue detail in side panel
- [ ] Hover → show issue title tooltip
- [ ] Pan and zoom controls
- [ ] Layout algorithm (dagre or hierarchical)
- [ ] Highlight selected node and dependencies

**Tests**: E2E tests with Playwright
**Estimated**: 8-10 hours

---

#### 2.4 Document Viewer ✅ **COMPLETE**
**Actions:**
- [x] Markdown rendering with `react-markdown`
- [x] Code syntax highlighting with react-syntax-highlighter (100+ languages)
- [x] VS Code Dark+ theme for syntax highlighting
- [x] Mermaid diagram rendering (flowcharts, sequences, class diagrams, etc.)
- [x] GitHub Flavored Markdown support (tables, strikethrough, task lists)
- [x] LaTeX math formulas with KaTeX
- [x] Modal overlay document viewer
- [x] Document history with commit timeline
- [x] Link to view document at specific commit
- [x] Integration with issue detail panel
- [ ] Support for images (relative paths resolved) (Deferred)
- [ ] Table of contents generation (Deferred)
- [ ] Print/export to PDF (Deferred)
- [ ] "View in GitHub" link (Deferred)

**Status**: Production-ready with comprehensive markdown rendering
**Actual time**: ~12 hours (including test fixes)

---

### Phase 3: Advanced Features

**Goal**: Search, historical views, and enhanced validation

#### 3.1 Full-Text Search
**Actions:**
- [ ] Add `tantivy` full-text search engine
- [ ] Index issues (title, description, context)
- [ ] Index document content
- [ ] Search API endpoint with filters
- [ ] Search UI with autocomplete
- [ ] Highlight search results

**Dependencies**: `tantivy = "0.21"`
**Estimated**: 10-12 hours

---

#### 3.2 Historical Document Viewer ✅ **COMPLETE**
**Actions:**
- [x] "Time machine" UI: select commit to view (DocumentHistory component)
- [x] Git log integration (list commits for a file)
- [x] View document at any commit via API
- [x] "View as of date" feature (commit timeline)
- [x] Commit message display
- [x] Author and timestamp for each commit
- [ ] Side-by-side diff view (compare versions) (Deferred)
- [ ] Link to full commit in GitHub (Deferred)

**Status**: Core functionality complete
**Implementation**: CLI commands (`jit doc history`, `jit doc diff`) + Web UI with commit timeline

---

#### 3.3 Document Graph Visualization
**Actions:**
- [ ] Parse document references from markdown links
- [ ] Build document-to-document dependency graph
- [ ] Visualize document relationships
- [ ] Detect circular references in docs
- [ ] Show which issues reference a document

**Estimated**: 6-8 hours

---

#### 3.4 Archive Management
**Actions:**
- [ ] Create `archive/` directory in repo
- [ ] `jit archive create <issue-id>` - Copy all referenced docs to archive
- [ ] Archive includes: issue JSON, all documents, git metadata
- [ ] `jit archive restore <issue-id>` - Restore archived issue
- [ ] Archive validation (ensure all files present)
- [ ] Web UI: browse archived issues

**Estimated**: 8-10 hours

---

### Phase 4: Enhanced UX & Polish

**Goal**: Production-ready knowledge management system

#### 4.1 Advanced UI Features
- [ ] Dark mode toggle
- [ ] Keyboard shortcuts (j/k navigation)
- [ ] Breadcrumbs for navigation
- [ ] Recent issues sidebar
- [ ] Favorites/bookmarks
- [ ] Collapsible graph legend
- [ ] Export graph as PNG/SVG

**Estimated**: 6-8 hours

---

#### 4.2 Collaboration Features
- [ ] Comments on issues (stored in git)
- [ ] @mentions in descriptions
- [ ] Activity feed (recent changes)
- [ ] Notifications (via CLI or web)

**Estimated**: 10-12 hours

---

#### 4.3 Integration & Automation
- [ ] GitHub Actions workflow templates
- [ ] Slack/Discord webhooks
- [ ] CI validation (fail build on broken refs)
- [ ] Auto-generate documentation from issues
- [ ] Export to Notion/Confluence

**Estimated**: 8-10 hours

---

## Technology Stack

### Backend
- **Rust**: Core library (existing)
- **Axum**: Web server framework
- **Git2**: Git integration for historical access
- **Tantivy**: Full-text search
- **Tower**: HTTP middleware

### Frontend
- **React** + **TypeScript**: UI framework
- **Vite**: Build tool
- **React Flow**: Graph visualization
- **React Markdown**: Document rendering
- **Prism.js**: Syntax highlighting
- **Axios**: HTTP client
- **React Router**: Navigation
- **TailwindCSS**: Styling (optional)

### Infrastructure
- **SQLite** (optional): Caching layer for performance
- **WebSocket** (optional): Real-time updates

---

## Data Model Extensions

### Issue (Extended)
```rust
pub struct Issue {
    // Existing fields...
    pub id: String,
    pub title: String,
    pub description: String,
    pub state: State,
    pub priority: Priority,
    pub assignee: Option<String>,
    pub dependencies: Vec<String>,
    pub gates_required: Vec<String>,
    pub gates_status: HashMap<String, GateState>,
    pub context: HashMap<String, String>,
    
    // New fields
    pub documents: Vec<DocumentReference>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub created_by: Option<String>,
    pub tags: Vec<String>,
    pub archived: bool,
}
```

### DocumentReference
```rust
pub struct DocumentReference {
    pub path: String,              // "docs/api-design.md"
    pub commit: Option<String>,    // Some("a1b2c3d") or None (HEAD)
    pub label: Option<String>,     // "API Design Document"
    pub doc_type: Option<String>,  // "design" | "implementation" | "notes"
    pub added_at: DateTime<Utc>,
    pub added_by: Option<String>,
}
```

---

## File Structure

```
just-in-time/
├── crates/
│   ├── jit/              # Core library (existing)
│   ├── dispatch/         # Orchestrator (existing)
│   └── server/           # NEW: Web API server
│       ├── src/
│       │   ├── main.rs
│       │   ├── routes/
│       │   ├── handlers/
│       │   └── middleware/
│       └── Cargo.toml
├── web/                  # NEW: Frontend application
│   ├── src/
│   ├── public/
│   ├── package.json
│   └── vite.config.ts
├── archive/              # NEW: Historical issue archives
│   └── <issue-id>/
│       ├── issue.json
│       └── documents/
├── docs/
│   └── knowledge-management-vision.md  # This file
└── README.md
```

---

## Implementation Timeline

### Sprint 1 (Week 1-2): Foundation
- ✅ Generic DAG refactoring (DONE)
- Phase 1.1: Document references (2-3 hours)
- Phase 1.2: CLI commands (4-6 hours)
- Phase 1.3: Validation (6-8 hours)
- **Total**: ~12-17 hours

### Sprint 2 (Week 3-4): Web UI Foundation
- Phase 2.1: REST API (8-10 hours)
- Phase 2.2: Frontend foundation (6-8 hours)
- Phase 2.3: Graph visualization (8-10 hours)
- Phase 2.4: Document viewer (4-6 hours)
- **Total**: ~26-34 hours

### Sprint 3 (Week 5-6): Advanced Features
- Phase 3.1: Full-text search (10-12 hours)
- Phase 3.2: Historical viewer (8-10 hours)
- Phase 3.3: Document graph (6-8 hours)
- Phase 3.4: Archive management (8-10 hours)
- **Total**: ~32-40 hours

### Sprint 4 (Week 7-8): Polish
- Phase 4.1: Advanced UI (6-8 hours)
- Phase 4.2: Collaboration (10-12 hours)
- Phase 4.3: Integrations (8-10 hours)
- **Total**: ~24-30 hours

**Total Estimated Effort**: 94-121 hours (12-15 full work days)

---

## Success Criteria

### Phase 1 (Foundation)
- [ ] Issues can reference documents with git commit hashes
- [ ] CLI commands for managing document references
- [ ] Validation detects all broken references
- [ ] All existing tests pass
- [ ] 30+ new tests added

### Phase 2 (Web UI)
- [ ] Web UI displays interactive dependency graph
- [ ] Clicking nodes shows issue details
- [ ] Markdown documents render inline
- [ ] Can view documents at specific commits
- [ ] Responsive design (mobile-friendly)

### Phase 3 (Advanced)
- [ ] Full-text search across issues and documents
- [ ] Historical document viewer with git integration
- [ ] Archive system preserves project knowledge
- [ ] Document-to-document graph visualization

### Phase 4 (Production)
- [ ] Dark mode and accessibility
- [ ] CI/CD integration for validation
- [ ] Documentation for end users
- [ ] Performance: <100ms API response times

---

## Next Steps

**Immediate Actions** (Start with Phase 1.1):

1. **Create feature branch**: `git checkout -b feature/document-references`
2. **Add DocumentReference type** to `domain.rs`
3. **Write tests first** (TDD):
   - Test serialization/deserialization
   - Test adding documents to issues
   - Test validation logic
4. **Implement** domain changes
5. **Update storage** to persist new fields
6. **Verify** all existing tests still pass

**Then proceed to**:
- Phase 1.2 (CLI commands)
- Phase 1.3 (Validation)
- Review and merge

---

## Related Documentation

- `docs/design.md` - Original design document
- `docs/generic-dag-refactoring.md` - Recent DAG work
- `ROADMAP.md` - Overall project roadmap
- `TESTING.md` - Testing strategy

---

## Questions for Consideration

1. **Storage format**: Keep JSON files or migrate to SQLite for better query performance?
2. **Git integration**: Embed `git2-rs` or shell out to `git` CLI?
3. **Frontend hosting**: Separate web server or embed in Rust binary with `include_dir!`?
4. **Authentication**: Add user authentication or rely on file system permissions?
5. **Real-time updates**: Use WebSockets for live collaboration or poll periodically?
6. **Deployment**: Self-hosted only or also offer SaaS version?

---

**Ready to start implementation? Begin with Phase 1.1 - Document References!**
