# Document Viewer Implementation Plan

**Date**: 2025-12-05  
**Status**: Planning  
**Goal**: Add inline document content viewer and historical document viewer support to the web UI

## Overview

Currently, the JIT system has:
- ✅ CLI commands for document management (`jit doc add/list/remove/show`)
- ✅ CLI commands for historical viewing (`jit doc history`, `jit doc diff`)
- ✅ Document references stored in issues
- ✅ Git integration for version-aware content retrieval
- ✅ REST API server with issue/graph/search endpoints
- ✅ React web UI with graph visualization and issue detail panel

**Missing:**
- ❌ API endpoints for document content retrieval
- ❌ API endpoints for document history/diff
- ❌ UI component for displaying document content inline
- ❌ UI component for browsing document history
- ❌ UI component for comparing document versions

## Architecture

### Backend (REST API)

**New API Endpoints:**

```
GET /api/issues/:id/documents/:path/content
  Query params:
    - commit (optional): Git commit hash (defaults to HEAD)
  Returns: { path, commit, content, content_type }

GET /api/issues/:id/documents/:path/history
  Returns: [{ commit, date, author, message }]

GET /api/issues/:id/documents/:path/diff
  Query params:
    - from (required): Source commit hash
    - to (optional): Target commit hash (defaults to HEAD)
  Returns: { from, to, diff }
```

**Implementation approach:**
- Reuse existing `CommandExecutor` methods
- Wrap CLI functionality in HTTP handlers
- Add proper error handling and HTTP status codes
- Support JSON responses with structured data

### Frontend (React/TypeScript)

**New Components:**

1. **DocumentViewer** - Main document display component
   - Renders markdown with syntax highlighting
   - Shows document metadata (path, commit, label)
   - Inline rendering in issue detail panel
   - Supports historical versions via commit selector

2. **DocumentHistory** - Timeline view of document changes
   - List of commits affecting the document
   - Click to view document at specific commit
   - Author, date, commit message display

3. **DocumentDiff** - Side-by-side or unified diff viewer
   - Compare two versions of a document
   - Syntax-highlighted diff output
   - Line-by-line change visualization

4. **DocumentList** (enhance existing) - Document references section
   - Show all documents for an issue
   - Click to view inline in DocumentViewer
   - Show commit badges for pinned versions
   - Link to history/diff views

**Updated Components:**
- `IssueDetail.tsx` - Add document viewer section
- `api/client.ts` - Add API client methods

### User Flow

```
User clicks issue in graph
  ↓
IssueDetail shows issue metadata + documents list
  ↓
User clicks document reference
  ↓
DocumentViewer fetches and renders document content
  ↓
User clicks "History" button
  ↓
DocumentHistory shows commit timeline
  ↓
User selects commit
  ↓
DocumentViewer updates to show historical version
  ↓
User clicks "Compare with HEAD"
  ↓
DocumentDiff shows changes side-by-side
```

## Implementation Plan

### Phase 1: Backend API (2-3 hours)

**1.1 Add Document API Routes**

File: `crates/server/src/routes.rs`

```rust
// Add to create_routes()
.route("/issues/:id/documents/:path/content", get(get_document_content))
.route("/issues/:id/documents/:path/history", get(get_document_history))
.route("/issues/:id/documents/:path/diff", get(get_document_diff))
```

**1.2 Implement Handler Functions**

```rust
#[derive(Deserialize)]
struct DocumentContentQuery {
    commit: Option<String>,
}

#[derive(Serialize)]
struct DocumentContentResponse {
    path: String,
    commit: String,
    content: String,
    content_type: String,
}

async fn get_document_content<S: IssueStore>(
    Path((id, path)): Path<(String, String)>,
    Query(query): Query<DocumentContentQuery>,
    State(executor): State<AppState<S>>,
) -> Result<Json<DocumentContentResponse>, StatusCode> {
    // Use CommandExecutor::show_document_content()
    // Extract content from git
    // Return structured JSON
}
```

Similar implementations for `get_document_history` and `get_document_diff`.

**1.3 Add Response Types**

Create structs for:
- `DocumentContentResponse`
- `DocumentHistoryResponse` (list of commits)
- `DocumentDiffResponse` (diff output)

**1.4 Extract Git Logic (if needed)**

If `CommandExecutor` methods are CLI-focused, extract reusable git operations:
- `read_file_at_commit(repo, path, commit) -> Result<String>`
- `get_file_history(repo, path) -> Result<Vec<CommitInfo>>`
- `get_file_diff(repo, path, from, to) -> Result<String>`

Consider creating `crates/jit/src/git_utils.rs` for shared git operations.

**1.5 Testing**

Add integration tests in `crates/server/tests/`:
- Test document content retrieval at HEAD
- Test document content at specific commit
- Test document history listing
- Test document diff generation
- Test error cases (missing document, invalid commit)

**Estimated:** 2-3 hours  
**Tests:** 8-12 new tests

---

### Phase 2: Frontend API Client (30 mins)

**2.1 Add TypeScript Types**

File: `web/src/types/models.ts`

```typescript
export interface DocumentContent {
  path: string;
  commit: string;
  content: string;
  content_type: string;
}

export interface CommitInfo {
  commit: string;
  date: string;
  author: string;
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
```

**2.2 Add API Client Methods**

File: `web/src/api/client.ts`

```typescript
export const apiClient = {
  // ... existing methods

  async getDocumentContent(
    issueId: string, 
    path: string, 
    commit?: string
  ): Promise<DocumentContent> {
    const params = new URLSearchParams();
    if (commit) params.set('commit', commit);
    const response = await api.get(
      `/issues/${issueId}/documents/${encodeURIComponent(path)}/content?${params}`
    );
    return response.data;
  },

  async getDocumentHistory(
    issueId: string, 
    path: string
  ): Promise<DocumentHistory> {
    const response = await api.get(
      `/issues/${issueId}/documents/${encodeURIComponent(path)}/history`
    );
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
    const response = await api.get(
      `/issues/${issueId}/documents/${encodeURIComponent(path)}/diff?${params}`
    );
    return response.data;
  },
};
```

**2.3 Testing**

Add unit tests for API client methods (mock axios).

**Estimated:** 30 minutes  
**Tests:** 3-5 new tests

---

### Phase 3: Document Viewer Components (3-4 hours)

**3.1 DocumentViewer Component**

File: `web/src/components/Document/DocumentViewer.tsx`

```typescript
interface DocumentViewerProps {
  issueId: string;
  documentRef: DocumentReference;
  onClose?: () => void;
}

export function DocumentViewer({ issueId, documentRef, onClose }: DocumentViewerProps) {
  const [content, setContent] = useState<DocumentContent | null>(null);
  const [selectedCommit, setSelectedCommit] = useState<string | undefined>(
    documentRef.commit || undefined
  );
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [showHistory, setShowHistory] = useState(false);

  useEffect(() => {
    loadContent(documentRef.path, selectedCommit);
  }, [issueId, documentRef.path, selectedCommit]);

  const loadContent = async (path: string, commit?: string) => {
    // Fetch document content
    // Handle loading states
  };

  return (
    <div className="document-viewer">
      <div className="document-header">
        <h3>{documentRef.label || documentRef.path}</h3>
        <div className="document-actions">
          <button onClick={() => setShowHistory(!showHistory)}>
            📜 History
          </button>
          <button onClick={onClose}>✕</button>
        </div>
      </div>

      {showHistory && (
        <DocumentHistory 
          issueId={issueId}
          path={documentRef.path}
          onSelectCommit={(commit) => {
            setSelectedCommit(commit);
            setShowHistory(false);
          }}
        />
      )}

      <div className="document-content markdown-content">
        {loading && <div>Loading...</div>}
        {error && <div className="error">{error}</div>}
        {content && (
          <ReactMarkdown
            remarkPlugins={[remarkMath]}
            rehypePlugins={[rehypeKatex]}
          >
            {content.content}
          </ReactMarkdown>
        )}
      </div>

      <div className="document-footer">
        <span>Commit: {content?.commit.substring(0, 8)}</span>
      </div>
    </div>
  );
}
```

**3.2 DocumentHistory Component**

File: `web/src/components/Document/DocumentHistory.tsx`

```typescript
interface DocumentHistoryProps {
  issueId: string;
  path: string;
  onSelectCommit: (commit: string) => void;
}

export function DocumentHistory({ issueId, path, onSelectCommit }: DocumentHistoryProps) {
  const [history, setHistory] = useState<DocumentHistory | null>(null);
  const [loading, setLoading] = useState(false);

  useEffect(() => {
    loadHistory();
  }, [issueId, path]);

  const loadHistory = async () => {
    // Fetch document history
  };

  return (
    <div className="document-history">
      <h4>Commit History</h4>
      <div className="commit-list">
        {history?.commits.map((commit) => (
          <div 
            key={commit.commit} 
            className="commit-item"
            onClick={() => onSelectCommit(commit.commit)}
          >
            <div className="commit-hash">{commit.commit.substring(0, 8)}</div>
            <div className="commit-message">{commit.message}</div>
            <div className="commit-meta">
              {commit.author} • {new Date(commit.date).toLocaleDateString()}
            </div>
          </div>
        ))}
      </div>
    </div>
  );
}
```

**3.3 DocumentDiff Component** (Optional - Phase 4)

File: `web/src/components/Document/DocumentDiff.tsx`

```typescript
interface DocumentDiffProps {
  issueId: string;
  path: string;
  from: string;
  to?: string;
}

export function DocumentDiff({ issueId, path, from, to }: DocumentDiffProps) {
  const [diff, setDiff] = useState<DocumentDiff | null>(null);

  // Fetch and display diff
  // Use syntax highlighting for diff output
  // Consider using react-diff-viewer library
}
```

**3.4 Update IssueDetail Component**

File: `web/src/components/Issue/IssueDetail.tsx`

Add document list section and viewer:

```typescript
const [selectedDocument, setSelectedDocument] = useState<DocumentReference | null>(null);

// After gates section, add:
{issue.documents && issue.documents.length > 0 && (
  <section style={{ marginBottom: '20px' }}>
    <h2>Documents</h2>
    <div className="documents-list">
      {issue.documents.map((doc, idx) => (
        <div 
          key={idx}
          className="document-item"
          onClick={() => setSelectedDocument(doc)}
        >
          <span className="document-icon">📄</span>
          <span className="document-label">{doc.label || doc.path}</span>
          {doc.commit && (
            <span className="document-commit">@{doc.commit.substring(0, 8)}</span>
          )}
        </div>
      ))}
    </div>
  </section>
)}

{selectedDocument && (
  <DocumentViewer
    issueId={issue.id}
    documentRef={selectedDocument}
    onClose={() => setSelectedDocument(null)}
  />
)}
```

**3.5 Styling**

File: `web/src/components/Document/Document.css`

Add terminal-style theming consistent with existing UI:
- Document viewer layout (header, content, footer)
- History timeline styling
- Commit item hover states
- Diff viewer styling (if implemented)

**3.6 Testing**

Files: `web/src/components/Document/__tests__/`

- `DocumentViewer.test.tsx` - Component rendering, loading states
- `DocumentHistory.test.tsx` - History list, click handlers
- Mock API responses for testing

**Estimated:** 3-4 hours  
**Tests:** 6-10 new tests

---

### Phase 4: Polish & Optional Features (1-2 hours)

**4.1 DocumentDiff Implementation**

Implement full diff viewer with:
- Side-by-side or unified view toggle
- Syntax highlighting
- Consider using `react-diff-viewer` library

**4.2 Enhanced Features**

- **Expandable document viewer**: Modal or slide-over panel
- **Download button**: Download document at specific commit
- **Copy content button**: Copy document content to clipboard
- **Full-screen mode**: Expand document viewer
- **Keyboard shortcuts**: 
  - `Esc` to close viewer
  - `h` to toggle history
  - `Arrow keys` to navigate history

**4.3 Loading States & Error Handling**

- Skeleton loaders for content
- Retry buttons on errors
- Toast notifications for actions
- Graceful degradation (repo not git, document deleted)

**4.4 Performance Optimization**

- Cache document content in memory
- Debounce history requests
- Lazy load history only when opened

**Estimated:** 1-2 hours

---

## Testing Strategy

### Backend Tests (crates/server/tests/)
- ✅ Document content retrieval (HEAD and commit)
- ✅ Document history listing
- ✅ Document diff generation
- ✅ Error cases (404, invalid commit, git errors)

### Frontend Tests (web/src/)
- ✅ API client methods (mocked)
- ✅ DocumentViewer component rendering
- ✅ DocumentHistory component rendering
- ✅ Document list in IssueDetail
- ✅ Click interactions
- ✅ Loading and error states

### Integration Tests
- ✅ End-to-end document viewing flow
- ✅ Historical version navigation
- ✅ Diff viewing

**Total estimated tests:** 20-30 new tests

---

## Success Criteria

**Must Have (MVP):**
- ✅ API endpoint for document content retrieval
- ✅ DocumentViewer component with markdown rendering
- ✅ Document list in IssueDetail with click-to-view
- ✅ Support for viewing documents at HEAD
- ✅ Support for viewing documents at pinned commits
- ✅ Basic error handling and loading states

**Should Have (Complete):**
- ✅ API endpoint for document history
- ✅ DocumentHistory component with commit timeline
- ✅ Click to view historical versions
- ✅ Commit metadata display (author, date, message)
- ✅ Terminal-style theming consistent with UI

**Nice to Have (Polish):**
- ✅ API endpoint for document diff
- ✅ DocumentDiff component
- ✅ Side-by-side diff view
- ✅ Keyboard shortcuts
- ✅ Full-screen mode

---

## Implementation Sequence

### Day 1: Backend Foundation (3-4 hours)
1. Add API routes for document content/history/diff
2. Implement handler functions
3. Add response types
4. Write backend integration tests
5. Verify with manual API testing (curl/Postman)

### Day 2: Frontend Components (4-5 hours)
1. Add TypeScript types
2. Add API client methods
3. Create DocumentViewer component
4. Create DocumentHistory component
5. Update IssueDetail to show documents
6. Add styling
7. Write frontend tests

### Day 3: Polish & Integration (2-3 hours)
1. Implement DocumentDiff (optional)
2. Add enhanced features (keyboard shortcuts, etc.)
3. End-to-end testing
4. Documentation updates
5. Demo video/screenshots

**Total estimated time:** 9-12 hours over 3 days

---

## Dependencies

**Backend:**
- ✅ `jit::commands::CommandExecutor` - Already has document methods
- ✅ `git2` crate - Already integrated for git operations
- ✅ Axum router - Already configured

**Frontend:**
- ✅ `react-markdown` - Already installed
- ✅ `rehype-katex`, `remark-math` - Already installed
- ✅ `axios` - Already configured
- ✅ CSS variables for theming - Already defined
- ❌ `react-diff-viewer` (optional, for Phase 4)

---

## Risk Assessment

**Low Risk:**
- Backend API implementation (reuses existing CLI logic)
- TypeScript types (straightforward)
- DocumentViewer component (similar to existing markdown rendering)

**Medium Risk:**
- Git integration edge cases (non-git repos, missing commits, deleted files)
- Performance with large documents (markdown rendering)
- URL encoding for document paths (special characters)

**Mitigation:**
- Add comprehensive error handling for git operations
- Test with various document sizes
- Use proper URL encoding (`encodeURIComponent`)
- Add loading indicators for slow operations

---

## Documentation Updates

**Files to update:**
- `web/README.md` - Add document viewer features
- `docs/web-ui-architecture.md` - Document new components
- `docs/knowledge-management-vision.md` - Mark Phase 2.4 complete
- `ROADMAP.md` - Update Phase 2.4 status

---

## Future Enhancements (Post-MVP)

- **Document graph visualization**: Show which issues reference which documents
- **Document search**: Search within document content
- **Annotation support**: Add comments/notes on specific lines
- **Version comparison**: Compare multiple versions side-by-side
- **Export**: Download documents or save as PDF
- **Real-time collaboration**: Live updates when documents change
- **Linked document navigation**: Click links within documents to navigate

---

## Questions for Consideration

1. **Modal vs. Inline**: Should DocumentViewer be a modal overlay or inline in IssueDetail?
   - **Recommendation**: Start inline, add modal mode in Phase 4

2. **Diff format**: Unified or side-by-side diff?
   - **Recommendation**: Unified diff first (simpler), side-by-side in Phase 4

3. **History pagination**: Limit number of commits shown?
   - **Recommendation**: Show last 20 commits, add "Load more" if needed

4. **Caching strategy**: Cache document content in memory?
   - **Recommendation**: Use React state for current session, no persistent cache initially

5. **Git operations**: Server-side or client-side?
   - **Recommendation**: Server-side (already implemented in backend)

---

## Conclusion

This implementation plan provides a comprehensive roadmap for adding inline document content viewing and historical document viewer support to the JIT web UI. The phased approach ensures:

1. **Backend-first**: Stable API before UI work
2. **Incremental delivery**: MVP first, enhancements later
3. **Test coverage**: Each phase includes tests
4. **Risk mitigation**: Address edge cases early
5. **Clear success criteria**: Well-defined goals

The total estimated effort is **9-12 hours** over 3 days, with the MVP achievable in **7-9 hours** (Phases 1-3).
