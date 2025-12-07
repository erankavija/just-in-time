# Document Graph Implementation Plan

**Phase**: 3.3 - Document Graph Visualization  
**Date**: 2025-12-06  
**Status**: Planning  
**Estimated Effort**: 12-16 hours (revised from 6-8 hours for comprehensive implementation)

## Vision Statement

Enable users to explore the **knowledge web** of their project by visualizing relationships between documents, detecting circular references, and understanding which issues depend on which documentation. This transforms the issue tracker into a comprehensive knowledge graph that reveals the structure of project documentation.

---

## Core Capabilities

### 1. Document Link Parsing
Parse markdown documents to extract references to other documents:
- Internal links: `[Design Doc](../design.md)`, `[API](./api-spec.md)`
- Absolute repo paths: `[README](/README.md)`
- External links: Ignored (https://example.com)
- Image references: `![diagram](./arch.png)` (optional)

### 2. Document-to-Document Graph
Build a directed graph showing how documents reference each other:
- Nodes: Markdown files, design docs, images
- Edges: References from one document to another
- Metadata: Reference type (link, image, code snippet)

### 3. Document-to-Issue Relationships
Track which issues reference which documents:
- Forward lookup: Issue → Documents it references
- Reverse lookup: Document → Issues that reference it
- Historical tracking: Documents referenced at specific commits

### 4. Circular Reference Detection
Identify documentation loops that could indicate:
- Circular dependencies in design
- Redundant documentation
- Opportunity for refactoring

### 5. Interactive Visualization
Web UI showing:
- Combined graph: Issues + Documents
- Document-only graph: Pure documentation structure
- Issue-centric view: Issue and its document dependencies
- Document-centric view: Document and issues that reference it

---

## Implementation Phases

### Phase 3.3.1: Backend - Document Link Parser (4-5 hours)

**Goal**: Extract markdown links and build document dependency graph

#### Data Model Extensions

```rust
// crates/jit/src/domain.rs

/// Represents a link from one document to another
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentLink {
    /// Source document path
    pub from: String,
    /// Target document path (resolved to absolute repo path)
    pub to: String,
    /// Link type (markdown_link, image_ref, code_snippet)
    pub link_type: DocumentLinkType,
    /// Line number in source where link appears (for validation)
    pub line: Option<usize>,
    /// Link text/alt text
    pub text: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum DocumentLinkType {
    MarkdownLink,    // [text](url)
    ImageReference,  // ![alt](url)
    CodeSnippet,     // ```file:path.rs```
}

/// Document graph node with metadata
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DocumentNode {
    /// Absolute path from repo root
    pub path: String,
    /// File type (md, png, jpg, pdf, etc.)
    pub file_type: String,
    /// Outgoing links to other documents
    pub links: Vec<DocumentLink>,
    /// Issues that reference this document
    pub referenced_by_issues: Vec<String>,
    /// File size in bytes (optional)
    pub size: Option<u64>,
    /// Last modified timestamp (from git or filesystem)
    pub last_modified: Option<String>,
}

/// Document dependency graph
#[derive(Debug, Clone, Default)]
pub struct DocumentGraph {
    /// All document nodes indexed by path
    pub nodes: HashMap<String, DocumentNode>,
    /// Circular reference chains (if any)
    pub cycles: Vec<Vec<String>>,
}
```

#### Markdown Parser Module

```rust
// crates/jit/src/document_parser.rs

use pulldown_cmark::{Parser, Event, Tag};
use std::path::{Path, PathBuf};

pub struct DocumentParser {
    /// Repository root for resolving relative paths
    repo_root: PathBuf,
}

impl DocumentParser {
    /// Parse markdown file and extract all document links
    pub fn parse_file(&self, path: &Path) -> Result<Vec<DocumentLink>> {
        let content = std::fs::read_to_string(path)?;
        self.parse_markdown(&content, path)
    }
    
    /// Parse markdown content and extract links
    fn parse_markdown(&self, content: &str, source_path: &Path) -> Result<Vec<DocumentLink>> {
        let mut links = Vec::new();
        let parser = Parser::new(content);
        let mut line_counter = 1;
        
        for (event, range) in parser.into_offset_iter() {
            match event {
                Event::Start(Tag::Link { dest_url, .. }) => {
                    if self.is_internal_link(&dest_url) {
                        let resolved = self.resolve_path(&dest_url, source_path)?;
                        links.push(DocumentLink {
                            from: source_path.to_string_lossy().to_string(),
                            to: resolved,
                            link_type: DocumentLinkType::MarkdownLink,
                            line: Some(line_counter),
                            text: None, // Extract from next Text event
                        });
                    }
                }
                Event::Start(Tag::Image { dest_url, .. }) => {
                    if self.is_internal_link(&dest_url) {
                        let resolved = self.resolve_path(&dest_url, source_path)?;
                        links.push(DocumentLink {
                            from: source_path.to_string_lossy().to_string(),
                            to: resolved,
                            link_type: DocumentLinkType::ImageReference,
                            line: Some(line_counter),
                            text: None,
                        });
                    }
                }
                _ => {}
            }
            
            // Count newlines in range for line tracking
            line_counter += content[range.start..range.end].matches('\n').count();
        }
        
        Ok(links)
    }
    
    /// Check if URL is internal (not http/https/mailto)
    fn is_internal_link(&self, url: &str) -> bool {
        !url.starts_with("http://") 
            && !url.starts_with("https://")
            && !url.starts_with("mailto:")
            && !url.starts_with('#') // Skip anchors
    }
    
    /// Resolve relative path to absolute repo path
    fn resolve_path(&self, link: &str, source_path: &Path) -> Result<String> {
        let link_path = Path::new(link);
        let resolved = if link.starts_with('/') {
            // Absolute from repo root
            self.repo_root.join(link.trim_start_matches('/'))
        } else {
            // Relative to source file
            source_path.parent()
                .unwrap_or(Path::new(""))
                .join(link_path)
        };
        
        // Normalize and canonicalize
        let canonical = resolved.canonicalize()
            .or_else(|_| Ok(resolved.clone()))?;
        
        // Return relative to repo root
        Ok(canonical.strip_prefix(&self.repo_root)
            .unwrap_or(&canonical)
            .to_string_lossy()
            .to_string())
    }
}
```

#### Document Graph Builder

```rust
// crates/jit/src/document_graph.rs

use crate::domain::{DocumentGraph, DocumentNode, DocumentLink};
use crate::document_parser::DocumentParser;
use std::collections::{HashMap, HashSet};

pub struct DocumentGraphBuilder {
    parser: DocumentParser,
    repo_root: PathBuf,
}

impl DocumentGraphBuilder {
    pub fn new(repo_root: PathBuf) -> Self {
        Self {
            parser: DocumentParser::new(repo_root.clone()),
            repo_root,
        }
    }
    
    /// Build document graph from all markdown files in repo
    pub fn build(&self) -> Result<DocumentGraph> {
        let mut graph = DocumentGraph::default();
        
        // Find all markdown files
        let markdown_files = self.find_markdown_files()?;
        
        // Parse each file
        for path in markdown_files {
            let links = self.parser.parse_file(&path)?;
            
            let node = DocumentNode {
                path: path.to_string_lossy().to_string(),
                file_type: self.get_file_type(&path),
                links,
                referenced_by_issues: Vec::new(), // Filled later
                size: std::fs::metadata(&path).ok().map(|m| m.len()),
                last_modified: None, // TODO: Get from git
            };
            
            graph.nodes.insert(node.path.clone(), node);
        }
        
        // Detect cycles
        graph.cycles = self.detect_cycles(&graph);
        
        Ok(graph)
    }
    
    /// Find all markdown files in repo (respects .gitignore)
    fn find_markdown_files(&self) -> Result<Vec<PathBuf>> {
        // Use walkdir or ignore crate to respect .gitignore
        let mut files = Vec::new();
        for entry in walkdir::WalkDir::new(&self.repo_root)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                if let Some(ext) = entry.path().extension() {
                    if ext == "md" || ext == "markdown" {
                        files.push(entry.path().to_path_buf());
                    }
                }
            }
        }
        Ok(files)
    }
    
    /// Detect circular references in document graph
    fn detect_cycles(&self, graph: &DocumentGraph) -> Vec<Vec<String>> {
        let mut cycles = Vec::new();
        let mut visited = HashSet::new();
        let mut rec_stack = Vec::new();
        
        for node_path in graph.nodes.keys() {
            if !visited.contains(node_path) {
                self.dfs_detect_cycle(
                    node_path,
                    graph,
                    &mut visited,
                    &mut rec_stack,
                    &mut cycles,
                );
            }
        }
        
        cycles
    }
    
    fn dfs_detect_cycle(
        &self,
        node: &str,
        graph: &DocumentGraph,
        visited: &mut HashSet<String>,
        rec_stack: &mut Vec<String>,
        cycles: &mut Vec<Vec<String>>,
    ) {
        visited.insert(node.to_string());
        rec_stack.push(node.to_string());
        
        if let Some(doc_node) = graph.nodes.get(node) {
            for link in &doc_node.links {
                if !visited.contains(&link.to) {
                    self.dfs_detect_cycle(&link.to, graph, visited, rec_stack, cycles);
                } else if rec_stack.contains(&link.to) {
                    // Found cycle
                    let cycle_start = rec_stack.iter().position(|p| p == &link.to).unwrap();
                    let cycle = rec_stack[cycle_start..].to_vec();
                    cycles.push(cycle);
                }
            }
        }
        
        rec_stack.pop();
    }
    
    fn get_file_type(&self, path: &Path) -> String {
        path.extension()
            .and_then(|e| e.to_str())
            .unwrap_or("unknown")
            .to_string()
    }
}
```

#### Integration with Issue Storage

```rust
// crates/jit/src/commands.rs

impl CommandExecutor {
    /// Get document graph with issue references
    pub fn get_document_graph(&self) -> Result<DocumentGraph> {
        let builder = DocumentGraphBuilder::new(self.repo_root.clone());
        let mut graph = builder.build()?;
        
        // Add issue references to document nodes
        let issues = self.storage.list_issues()?;
        for issue in issues {
            for doc_ref in &issue.documents {
                if let Some(node) = graph.nodes.get_mut(&doc_ref.path) {
                    node.referenced_by_issues.push(issue.id.clone());
                }
            }
        }
        
        Ok(graph)
    }
}
```

#### CLI Commands

```bash
# Show document graph
jit doc graph [--format text|json|dot|mermaid]

# Check for circular references
jit doc cycles

# Show which issues reference a document
jit doc reverse <path>

# Validate all document links
jit validate docs --check-links
```

**Actions:**
- [ ] Add `pulldown_cmark = "0.11"` dependency (markdown parser)
- [ ] Add `walkdir = "2.4"` dependency (file traversal)
- [ ] Create `document_parser.rs` module
- [ ] Create `document_graph.rs` module
- [ ] Add data model extensions to `domain.rs`
- [ ] Implement markdown link extraction
- [ ] Implement path resolution logic
- [ ] Implement cycle detection algorithm
- [ ] Add CLI commands for document graph
- [ ] Write 20-25 unit tests (parser, graph builder, cycle detection)
- [ ] Write 10-15 integration tests (CLI commands)

**Tests:**
- Parse markdown with various link formats
- Resolve relative paths correctly
- Detect simple cycles (A→B→A)
- Detect complex cycles (A→B→C→A)
- Handle missing files gracefully
- Ignore external links
- Handle edge cases (empty files, no links, broken links)

---

### Phase 3.3.2: REST API Endpoints (2-3 hours)

**Goal**: Expose document graph via REST API

#### New API Endpoints

```rust
// crates/server/src/routes.rs

// Get document graph
GET /api/documents/graph
Response: {
  "nodes": [
    {
      "path": "docs/design.md",
      "file_type": "md",
      "links": [
        {
          "from": "docs/design.md",
          "to": "docs/api-spec.md",
          "link_type": "markdown_link",
          "line": 42,
          "text": "API Specification"
        }
      ],
      "referenced_by_issues": ["01ABC", "02DEF"],
      "size": 12345,
      "last_modified": "2025-12-01T10:30:00Z"
    }
  ],
  "cycles": [
    ["docs/a.md", "docs/b.md", "docs/a.md"]
  ]
}

// Get reverse references for a document
GET /api/documents/:path/references
Response: {
  "document": "docs/design.md",
  "issues": [
    {
      "id": "01ABC",
      "title": "Implement authentication",
      "state": "in_progress",
      "reference": {
        "commit": "a1b2c3d",
        "label": "Auth Design"
      }
    }
  ],
  "documents": [
    {
      "path": "docs/implementation.md",
      "line": 15,
      "text": "See design document"
    }
  ]
}

// Get document metadata
GET /api/documents/:path/metadata
Response: {
  "path": "docs/design.md",
  "file_type": "md",
  "size": 12345,
  "line_count": 456,
  "word_count": 2345,
  "last_modified": "2025-12-01T10:30:00Z",
  "git_commits": 15,
  "authors": ["alice", "bob"]
}

// Get combined graph (issues + documents)
GET /api/graph/combined
Response: {
  "nodes": [
    { "id": "issue:01ABC", "type": "issue", "title": "...", ... },
    { "id": "doc:docs/design.md", "type": "document", "path": "..." }
  ],
  "edges": [
    { "from": "issue:01ABC", "to": "doc:docs/design.md", "type": "references" },
    { "from": "doc:docs/design.md", "to": "doc:docs/api.md", "type": "links" }
  ]
}
```

#### Handler Implementations

```rust
// crates/server/src/handlers/documents.rs

pub async fn get_document_graph(
    State(executor): State<Arc<CommandExecutor>>,
) -> Result<Json<DocumentGraph>, StatusCode> {
    let graph = executor.get_document_graph()
        .map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
    Ok(Json(graph))
}

pub async fn get_document_references(
    State(executor): State<Arc<CommandExecutor>>,
    Path(path): Path<String>,
) -> Result<Json<DocumentReferences>, StatusCode> {
    // Implementation
}

pub async fn get_combined_graph(
    State(executor): State<Arc<CommandExecutor>>,
) -> Result<Json<CombinedGraph>, StatusCode> {
    // Merge issue graph + document graph
}
```

**Actions:**
- [ ] Add new routes to `routes.rs`
- [ ] Create `handlers/documents.rs` module
- [ ] Implement handler functions
- [ ] Add response types for serialization
- [ ] Write 8-10 API integration tests
- [ ] Update OpenAPI schema (if used)

**Tests:**
- GET /api/documents/graph returns valid graph
- GET /api/documents/:path/references returns issues
- GET /api/graph/combined merges correctly
- Error handling for missing documents
- Performance with large graphs (100+ nodes)

---

### Phase 3.3.3: Frontend - Document Graph Component (4-5 hours)

**Goal**: Visualize document graph in web UI

#### New React Components

```typescript
// web/src/components/DocumentGraph/DocumentGraphView.tsx

import ReactFlow, { Node, Edge } from 'reactflow';

interface DocumentGraphViewProps {
  mode: 'documents-only' | 'issues-only' | 'combined';
  selectedNode?: string;
  onNodeClick: (nodeId: string, nodeType: 'issue' | 'document') => void;
}

export const DocumentGraphView: React.FC<DocumentGraphViewProps> = ({
  mode,
  selectedNode,
  onNodeClick,
}) => {
  const [nodes, setNodes] = useNodesState([]);
  const [edges, setEdges] = useEdgesState([]);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    loadGraph();
  }, [mode]);

  const loadGraph = async () => {
    try {
      let data;
      if (mode === 'combined') {
        data = await apiClient.getCombinedGraph();
      } else if (mode === 'documents-only') {
        data = await apiClient.getDocumentGraph();
      } else {
        data = await apiClient.getIssueGraph();
      }
      
      const flowNodes = convertToFlowNodes(data.nodes);
      const flowEdges = convertToFlowEdges(data.edges);
      
      setNodes(flowNodes);
      setEdges(flowEdges);
    } catch (error) {
      console.error('Failed to load graph:', error);
    } finally {
      setLoading(false);
    }
  };

  return (
    <div className="document-graph-view">
      <div className="graph-controls">
        <button onClick={() => setMode('combined')}>Combined</button>
        <button onClick={() => setMode('issues-only')}>Issues Only</button>
        <button onClick={() => setMode('documents-only')}>Documents Only</button>
        <button onClick={() => fitView()}>Fit View</button>
      </div>
      
      <ReactFlow
        nodes={nodes}
        edges={edges}
        onNodeClick={(_, node) => onNodeClick(node.id, node.data.type)}
        nodeTypes={nodeTypes}
        edgeTypes={edgeTypes}
      >
        <Controls />
        <Background />
      </ReactFlow>
      
      {selectedNode && <DocumentDetailPanel nodeId={selectedNode} />}
    </div>
  );
};
```

#### Custom Node Types

```typescript
// web/src/components/DocumentGraph/nodes/DocumentNode.tsx

export const DocumentNode: React.FC<NodeProps> = ({ data }) => {
  const { path, file_type, referenced_by_issues } = data;
  
  return (
    <div className="document-node">
      <div className="document-icon">📄</div>
      <div className="document-path">{path}</div>
      <div className="document-type">{file_type}</div>
      {referenced_by_issues.length > 0 && (
        <div className="issue-badge">{referenced_by_issues.length} issues</div>
      )}
    </div>
  );
};

// web/src/components/DocumentGraph/nodes/IssueNode.tsx

export const IssueNode: React.FC<NodeProps> = ({ data }) => {
  // Existing issue node rendering
  // Add indicator for document count
};
```

#### Document Detail Panel

```typescript
// web/src/components/DocumentGraph/DocumentDetailPanel.tsx

export const DocumentDetailPanel: React.FC<{ nodeId: string }> = ({ nodeId }) => {
  const [references, setReferences] = useState<DocumentReferences | null>(null);

  useEffect(() => {
    loadReferences();
  }, [nodeId]);

  const loadReferences = async () => {
    const data = await apiClient.getDocumentReferences(nodeId);
    setReferences(data);
  };

  return (
    <div className="document-detail-panel">
      <h3>📄 {nodeId}</h3>
      
      <section>
        <h4>Referenced by Issues</h4>
        <ul>
          {references?.issues.map(issue => (
            <li key={issue.id}>
              <Link to={`/issue/${issue.id}`}>{issue.title}</Link>
            </li>
          ))}
        </ul>
      </section>
      
      <section>
        <h4>Links to Documents</h4>
        <ul>
          {references?.documents.map(doc => (
            <li key={doc.path}>
              <a onClick={() => viewDocument(doc.path)}>{doc.path}</a>
              <span className="line-number">Line {doc.line}</span>
            </li>
          ))}
        </ul>
      </section>
      
      <section>
        <h4>Metadata</h4>
        <dl>
          <dt>File Type</dt><dd>{references?.file_type}</dd>
          <dt>Size</dt><dd>{formatBytes(references?.size)}</dd>
          <dt>Last Modified</dt><dd>{formatDate(references?.last_modified)}</dd>
        </dl>
      </section>
      
      <button onClick={() => openDocumentViewer(nodeId)}>View Document</button>
    </div>
  );
};
```

#### Cycle Detector UI

```typescript
// web/src/components/DocumentGraph/CycleDetector.tsx

export const CycleDetector: React.FC = () => {
  const [cycles, setCycles] = useState<string[][]>([]);

  useEffect(() => {
    detectCycles();
  }, []);

  const detectCycles = async () => {
    const graph = await apiClient.getDocumentGraph();
    setCycles(graph.cycles);
  };

  if (cycles.length === 0) {
    return <div className="success">✓ No circular references detected</div>;
  }

  return (
    <div className="cycle-detector warning">
      <h4>⚠️ Circular References Detected ({cycles.length})</h4>
      <ul>
        {cycles.map((cycle, idx) => (
          <li key={idx}>
            {cycle.join(' → ')}
            <button onClick={() => highlightCycle(cycle)}>Show in Graph</button>
          </li>
        ))}
      </ul>
    </div>
  );
};
```

**Actions:**
- [ ] Create `DocumentGraphView.tsx` component
- [ ] Create custom node types (DocumentNode, CombinedNode)
- [ ] Create `DocumentDetailPanel.tsx` component
- [ ] Create `CycleDetector.tsx` component
- [ ] Add graph mode toggle (issues, documents, combined)
- [ ] Implement node click handlers
- [ ] Add styling for document nodes (distinct from issues)
- [ ] Integrate with existing IssueDetail component
- [ ] Write 10-12 component tests
- [ ] Add E2E tests with Playwright

**Tests:**
- Document graph renders correctly
- Clicking document node shows detail panel
- Mode toggle switches between views
- Cycle detector highlights circular refs
- Combined graph shows both node types
- Navigation between issue and document views

---

### Phase 3.3.4: Integration & Polish (2-3 hours)

**Goal**: Integrate document graph with existing UI and add polish

#### Navigation Integration

```typescript
// Update existing GraphView.tsx to support document mode

// Add route for document graph view
// web/src/App.tsx
<Route path="/documents" element={<DocumentGraphView mode="documents-only" />} />
<Route path="/graph/combined" element={<DocumentGraphView mode="combined" />} />

// Add navigation links
<nav>
  <Link to="/">Issues</Link>
  <Link to="/documents">Documents</Link>
  <Link to="/graph/combined">Combined Graph</Link>
</nav>
```

#### Search Integration

```typescript
// Update search to include documents
// web/src/components/Search/SearchBar.tsx

interface SearchResult {
  type: 'issue' | 'document';
  id: string;
  title: string;
  excerpt: string;
}

// Search both issues and documents
const results = [
  ...issueResults.map(i => ({ type: 'issue', ...i })),
  ...documentResults.map(d => ({ type: 'document', ...d })),
];
```

#### Validation Integration

```typescript
// Add document link validation to validation report
// Display broken links in UI with suggestions
```

#### Performance Optimization

- [ ] Cache document graph (rebuild only on file changes)
- [ ] Lazy load document content
- [ ] Virtualize large graphs (1000+ nodes)
- [ ] Add loading skeletons

#### Documentation

- [ ] Update `docs/web-ui-architecture.md` with document graph
- [ ] Add user guide: "Exploring Document Relationships"
- [ ] Document API endpoints in OpenAPI schema
- [ ] Add README section about document graph

**Actions:**
- [ ] Integrate with existing navigation
- [ ] Update search to include documents
- [ ] Add document link validation to UI
- [ ] Performance optimization
- [ ] Documentation updates
- [ ] Final testing and bug fixes

---

## Technology Stack

### Backend Dependencies
```toml
[dependencies]
pulldown_cmark = "0.11"    # Markdown parsing
walkdir = "2.4"            # File traversal
regex = "1.10"             # Path pattern matching (optional)
```

### Frontend Dependencies
```json
{
  "dependencies": {
    "reactflow": "^11.10.0",  // Already installed
    "dagre": "^0.8.5",        // Already installed
  }
}
```

---

## Data Flow

```
1. User opens document graph view
2. Frontend requests: GET /api/documents/graph
3. Backend:
   a. Scans repo for markdown files (walkdir)
   b. Parses each file (pulldown_cmark)
   c. Extracts internal links
   d. Resolves relative paths
   e. Builds graph nodes and edges
   f. Detects cycles (DFS)
   g. Adds issue references from storage
4. Backend returns JSON graph
5. Frontend:
   a. Converts to ReactFlow nodes/edges
   b. Applies dagre layout
   c. Renders graph with custom node types
   d. Highlights cycles if present
6. User clicks document node:
   a. Request: GET /api/documents/:path/references
   b. Show detail panel with issues and links
   c. Option to open document viewer
```

---

## Testing Strategy

### Unit Tests (30-35 tests)
- Document parser: link extraction, path resolution
- Graph builder: node creation, cycle detection
- Path utilities: relative/absolute conversion
- Edge cases: empty files, no links, broken paths

### Integration Tests (15-20 tests)
- CLI commands: `jit doc graph`, `jit doc cycles`
- API endpoints: all document routes
- Graph construction: end-to-end
- Performance: large repos (1000+ files)

### Component Tests (10-12 tests)
- DocumentGraphView rendering
- Node type rendering (document vs issue)
- Mode toggle functionality
- Detail panel display
- Cycle detector UI

### E2E Tests (5-8 tests)
- Navigate to document graph
- Click document node → show details
- Toggle between graph modes
- Search documents
- Validate circular refs warning

**Total Tests**: 60-75 new tests

---

## Success Criteria

### Phase 3.3.1 Complete
- [ ] Can parse markdown files and extract links
- [ ] Document graph builds successfully
- [ ] Cycle detection works correctly
- [ ] CLI commands functional
- [ ] 35+ tests passing

### Phase 3.3.2 Complete
- [ ] REST API endpoints return valid data
- [ ] Combined graph merges issues + documents
- [ ] Performance acceptable (<500ms for 100 documents)
- [ ] 8+ API tests passing

### Phase 3.3.3 Complete
- [ ] Document graph renders in UI
- [ ] Can toggle between view modes
- [ ] Detail panel shows references
- [ ] Cycle warnings display correctly
- [ ] 12+ component tests passing

### Phase 3.3.4 Complete
- [ ] Integrated with existing navigation
- [ ] Search includes documents
- [ ] Documentation complete
- [ ] All 60+ tests passing
- [ ] Zero clippy warnings

---

## Timeline

### Sprint Breakdown
- **Day 1-2**: Phase 3.3.1 - Backend parser and graph builder (4-5 hours)
- **Day 3**: Phase 3.3.2 - REST API endpoints (2-3 hours)
- **Day 4-5**: Phase 3.3.3 - Frontend components (4-5 hours)
- **Day 6**: Phase 3.3.4 - Integration and polish (2-3 hours)

**Total Estimated**: 12-16 hours (1.5-2 full work days)

---

## Future Enhancements (Post-MVP)

- [ ] Document templates (auto-generate from issues)
- [ ] Dead document detection (no issues reference it)
- [ ] Document quality metrics (outdated, broken links)
- [ ] Export document graph as PNG/SVG
- [ ] Document dependency timeline (how refs change over time)
- [ ] Smart suggestions ("Issue X might need Document Y")
- [ ] Document clustering (related docs grouped)
- [ ] Import external docs (Notion, Confluence)

---

## Open Questions

1. **Should we parse non-markdown files?** (e.g., README.txt, code files)
   - **Recommendation**: Start with markdown only, add others later

2. **How to handle external links?** (https://example.com/doc.pdf)
   - **Recommendation**: Ignore external links for graph, but validate they're reachable

3. **Should document nodes show in main issue graph by default?**
   - **Recommendation**: No, keep separate views. Add "combined" mode as opt-in

4. **Performance limit?** (how many documents before it's too slow?)
   - **Recommendation**: Optimize for repos with <1000 markdown files. Add caching if needed

5. **Git integration depth?** (should we parse documents at every commit?)
   - **Recommendation**: Current version (HEAD) only for MVP. Historical parsing in Phase 4

---

## Related Documentation

- `docs/knowledge-management-vision.md` - Overall vision
- `docs/web-ui-architecture.md` - UI architecture
- `docs/search-implementation.md` - Search strategy
- `docs/document-viewer-implementation-plan.md` - Document viewer (completed)

---

## Next Steps

**Ready to start?**

1. Create feature branch: `git checkout -b feature/document-graph`
2. Start with Phase 3.3.1: Add `pulldown_cmark` and `walkdir` dependencies
3. Write tests for `DocumentParser::parse_markdown()`
4. Implement link extraction (TDD)
5. Build graph data structures
6. Add CLI commands
7. Proceed to API layer (Phase 3.3.2)

**Let's build the knowledge graph! 🚀**
