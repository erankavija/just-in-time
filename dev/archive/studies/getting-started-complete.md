# Complete Getting Started Guide: Real-World Test Project

**Date**: 2025-12-06  
**Audience**: New users testing JIT for the first time  
**Goal**: Set up JIT from scratch and track a small real project end-to-end

---

## Table of Contents

1. [Prerequisites](#prerequisites)
2. [Installation](#installation)
3. [Your First JIT Repository](#your-first-jit-repository)
4. [Real-World Test Project](#real-world-test-project)
5. [Using the CLI](#using-the-cli)
6. [Using the Web UI](#using-the-web-ui)
7. [Using the MCP Server (AI Integration)](#using-the-mcp-server-ai-integration)
8. [Multi-Agent Workflow](#multi-agent-workflow)
9. [Verification & Testing](#verification--testing)
10. [Next Steps](#next-steps)

---

## Prerequisites

### Required
- **Linux** (Ubuntu 20.04+, Debian 11+, or equivalent)
- **Git** (for version control and document history)
  ```bash
  sudo apt update && sudo apt install -y git
  git --version  # Should be 2.20+
  ```

### Recommended
- **ripgrep** (for full-text search)
  ```bash
  sudo apt install -y ripgrep
  rg --version
  ```

### Optional (for Web UI)
- **Node.js 20+** (for local development)
  ```bash
  curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
  sudo apt install -y nodejs
  node --version  # Should be v20+
  ```

- **Docker** (for containerized deployment)
  ```bash
  sudo apt install -y docker.io docker-compose
  sudo usermod -aG docker $USER  # Add yourself to docker group
  newgrp docker  # Refresh group membership
  docker --version
  ```

---

## Installation

### Option 1: Build from Source (Recommended for Testing Latest Features)

```bash
# 1. Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
rustc --version  # Should be 1.80+

# 2. Clone the repository
cd ~/Projects  # or wherever you keep code
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

# 3. Build the Rust binaries
cargo build --release --workspace

# 4. Add to PATH (add this to ~/.bashrc for persistence)
export PATH="$PWD/target/release:$PATH"

# 5. Verify installation
jit --version
jit-server --version
jit-dispatch --version

# 6. Run tests to ensure everything works
cargo test --workspace
# Should see: 490+ tests passing
```

### Option 2: Pre-built Binaries (Simpler, but may not be latest)

```bash
# Download latest release
cd ~/Downloads
wget https://github.com/erankavija/just-in-time/releases/latest/download/jit-linux-x64.tar.gz

# Extract and install
mkdir -p ~/.local/bin
tar -xzf jit-linux-x64.tar.gz -C ~/.local/bin

# Add to PATH (add to ~/.bashrc for persistence)
export PATH="$HOME/.local/bin:$PATH"

# Verify
jit --version
```

### Option 3: Podman/Docker (For Building Images)

```bash
# Clone repository
git clone https://github.com/erankavija/just-in-time.git
cd just-in-time

# Build images with Podman
podman build -t jit-api -f docker/Dockerfile.api .
podman build -t jit-web -f docker/Dockerfile.web .

# Or use Docker
docker build -t jit-api -f docker/Dockerfile.api .
docker build -t jit-web -f docker/Dockerfile.web .

# See "Using the Web UI" section for how to run them
```

**Note:** For development, running the API server directly on the host (without containers) avoids permission issues with volume mounts.

---

## Your First JIT Repository

Let's create a simple test repository to understand JIT basics:

```bash
# Create a test project directory
mkdir ~/test-jit-project
cd ~/test-jit-project

# Initialize git (JIT works best with git)
git init
git config user.name "Test User"
git config user.email "test@example.com"

# Initialize JIT
jit init
# Output: Initialized JIT tracker in /home/user/test-jit-project/.jit

# Verify structure
ls -la .jit/
# Should see:
# - issues/           (stores issue JSON files)
# - gates.json        (gate registry)
# - events.jsonl      (event log)
# - index.json        (issue index)

# Check status
jit status
# Output: 0 issues (0 backlog, 0 ready, 0 in progress, 0 gated, 0 done)
```

---

## Understanding Labels and Dependencies

Just-in-time uses two orthogonal structures to organize work:

### Labels: Organizational Membership

Labels group related work for filtering and reporting:
- `--label "epic:auth"` means "this work is part of the auth epic"
- `--label "milestone:v1.0"` means "this work is part of the v1.0 release"

**Query**: `jit query all --label "epic:auth"` shows all members

### Dependencies: Work Order

Dependencies control workflow and blocking:
- `jit dep add EPIC TASK` means "epic depends on task"
- Epic is blocked until task completes

**Query**: `jit query available` shows unblocked work

### They Work Together

```
Task: Login
  ├─ label "epic:auth" → part of auth epic (grouping)
  └─ dependency of Epic → required by epic (blocking)

Both flow the same direction: Task → Epic → Milestone
But serve different purposes: organization vs workflow
```

You can use:
- **Labels alone** (just grouping, no workflow control)
- **Dependencies alone** (just ordering, no organizational grouping)
- **Both together** (recommended for most cases)

See `docs/dependency-vs-labels-clarity.md` for detailed explanation.

---

## Real-World Test Project

Let's track a realistic small project: **Building a Personal Blog Engine**

### Step 1: Create Project Structure

```bash
# Create project files
mkdir -p docs src tests
touch README.md docs/design.md docs/api-spec.md
touch src/main.rs tests/test_posts.rs

# Add initial content
cat > README.md <<'EOF'
# Personal Blog Engine

A simple static site generator for personal blogs.

## Features
- Markdown to HTML conversion
- Template system
- RSS feed generation
- Tag support
EOF

cat > docs/design.md <<'EOF'
# Blog Engine Design

## Architecture
- **Parser**: Markdown → AST
- **Renderer**: AST → HTML
- **Template Engine**: Mustache-based
- **Generator**: Static files output

## Dependencies
- `pulldown-cmark` for markdown parsing
- `handlebars` for templating
EOF

# Commit initial structure
git add .
git commit -m "Initial project structure"
```

### Step 2: Define High-Level Issues

```bash
# Create epic/parent issues
jit issue create \
  --title "Setup project infrastructure" \
  --description "Initialize Rust project, CI/CD, and testing framework" \
  --priority high \
  --state backlog

# Note the issue ID (e.g., 01ABC123XYZ)
INFRA_ID=$(jit query all --json | jq -r '.[0].id')

jit issue create \
  --title "Implement markdown parser" \
  --description "Parse markdown files using pulldown-cmark" \
  --priority high \
  --state backlog

PARSER_ID=$(jit query all --json | jq -r '.[1].id')

jit issue create \
  --title "Build template engine" \
  --description "Handlebars-based templating with custom helpers" \
  --priority normal \
  --state backlog

TEMPLATE_ID=$(jit query all --json | jq -r '.[2].id')

jit issue create \
  --title "Generate static site" \
  --description "Walk content directory, render pages, write output" \
  --priority normal \
  --state backlog \
  --depends-on "$PARSER_ID" \
  --depends-on "$TEMPLATE_ID"

GENERATOR_ID=$(jit query all --json | jq -r '.[3].id')

jit issue create \
  --title "Add RSS feed support" \
  --description "Generate RSS 2.0 feed from blog posts" \
  --priority low \
  --state backlog \
  --depends-on "$GENERATOR_ID"

RSS_ID=$(jit query all --json | jq -r '.[4].id')

# View dependency graph
jit graph show
# Output:
# Issue Dependency Graph:
# 01ABC123XYZ: Setup project infrastructure (backlog, high)
# 02DEF456ABC: Implement markdown parser (backlog, high)
# 03GHI789DEF: Build template engine (backlog, normal)
# 04JKL012GHI: Generate static site (backlog, normal)
#   └─ 02DEF456ABC: Implement markdown parser
#   └─ 03GHI789DEF: Build template engine
# 05MNO345JKL: Add RSS feed support (backlog, low)
#   └─ 04JKL012GHI: Generate static site
```

### Step 3: Break Down into Subtasks

```bash
# Break down the parser issue
jit issue breakdown "$PARSER_ID"
# This will open an editor. Enter subtasks:

# --- Subtasks (one per line) ---
# Create parser module structure
# Implement frontmatter extraction
# Add markdown AST traversal
# Write parser unit tests
# [Press Ctrl+O, Enter, Ctrl+X to save in nano]

# View the created subtasks
jit issue show "$PARSER_ID"
# Output will show 4 child issues with automatic dependency inheritance
```

### Step 4: Add Document References

```bash
# Link design docs to issues
jit doc add "$PARSER_ID" docs/design.md --label "Parser Design" --type design
jit doc add "$TEMPLATE_ID" docs/design.md --label "Template Design" --type design
jit doc add "$GENERATOR_ID" docs/api-spec.md --label "API Specification" --type spec

# View issue with documents
jit issue show "$PARSER_ID"
# Output includes:
# Documents:
#   - docs/design.md (Parser Design) [type: design]
```

### Step 5: Add Quality Gates

```bash
# Define gates in registry
jit registry add code-review --description "Code review completed and approved"
jit registry add unit-tests --description "Unit tests written and passing"
jit registry add integration-tests --description "Integration tests passing"

# Add gates to issues
jit gate add "$PARSER_ID" code-review
jit gate add "$PARSER_ID" unit-tests
jit gate add "$TEMPLATE_ID" code-review
jit gate add "$TEMPLATE_ID" unit-tests

# View gates
jit registry list
# Output:
# Gate Registry:
# - code-review: Code review completed and approved
# - unit-tests: Unit tests written and passing
# - integration-tests: Integration tests passing
```

### Step 6: Simulate Workflow

```bash
# Start working on infrastructure
jit issue claim "$INFRA_ID" agent:dev-1
jit issue update "$INFRA_ID" --state in_progress

# Complete infrastructure work
jit issue update "$INFRA_ID" --state done

# Infrastructure is done, parser is now ready
jit query available
# Output: 02DEF456ABC: Implement markdown parser (ready, high)

# Claim and work on parser
jit issue claim "$PARSER_ID" agent:dev-1
jit issue update "$PARSER_ID" --state in_progress

# Simulate completing work (but gates block completion)
jit issue update "$PARSER_ID" --state gated
# This moves to "gated" state (waiting for quality approval)

# Pass gates
jit gate pass "$PARSER_ID" unit-tests
jit gate pass "$PARSER_ID" code-review

# Now issue can complete (automatically transitions to done)
jit issue show "$PARSER_ID"
# State: done

# Check what's ready now
jit query available
# Output: 03GHI789DEF: Build template engine (ready, normal)
```

### Step 7: Search and Explore

```bash
# Search across issues and documents (requires ripgrep)
jit search "markdown"
# Output:
# Issue 02DEF456ABC: Implement markdown parser
# - docs/design.md:5: Markdown → AST

# Full-text search in documents
jit search "template" --glob "docs/*.md"

# Query by state
jit query all --state in_progress
jit query all --state done

# Query by priority
jit query all --priority high

# Check for blocked issues
jit query blocked
# Output shows issues blocked by dependencies or failed gates
```

---

## Using the CLI

### Essential Commands Cheatsheet

```bash
# Issue Management
jit issue create --title "..." --description "..." --priority high
jit query all
jit issue show <id>
jit issue update <id> --state in_progress
jit issue delete <id>
jit issue assign <id> agent:worker-1
jit issue claim <id> agent:worker-1
jit issue unassign <id>

# Dependencies
jit dep add <from-id> <to-id>  # from depends on to
jit dep rm <from-id> <to-id>

# Quality Gates
jit registry add <gate-key> --description "..."
jit gate add <id> <gate-key>
jit gate pass <id> <gate-key>
jit gate fail <id> <gate-key>

# Documents
jit doc add <id> <path> --label "..." --type design
jit doc list <id>
jit doc show <id> <path>
jit doc history <id> <path>
jit doc diff <id> <path> <commit1> <commit2>

# Graphs
jit graph show [<id>]
jit graph roots
jit graph downstream <id>
jit graph export --format dot  # or mermaid

# Queries
jit query available
jit query blocked
jit query all --state <state>
jit query all --priority <priority>
jit query all --assignee <assignee>

# Search
jit search <query> [--regex] [--glob "pattern"]

# Validation
jit validate  # Check DAG integrity, gates, references

# Status
jit status  # Overview of repository

# JSON Output (for automation)
jit query all --json | jq '.[] | select(.state == "ready")'
```

---

## Using the Web UI

### Recommended Setup: API on Host + Web UI in Container

**Why this approach?** Running the API server directly on the host avoids permission issues with containerized volume mounts, especially with Podman's rootless mode.

```bash
# Terminal 1: Start API server on host
cd ~/Projects/just-in-time
export JIT_DATA_DIR=~/test-jit-project/.jit  # Point to your .jit directory
cargo run --bin jit-server
# Server running on http://localhost:3000

# Terminal 2: Start Web UI in container (Podman/Docker)
podman run -d --name jit-web -p 8080:80 \
  --network host \
  jit-web

# Access Web UI at: http://localhost:8080
# Web UI connects to API at: http://localhost:3000
```

**Note:** With `--network host`, the web container shares your host network, so it can reach the API on localhost:3000.

### Alternative: Full Development Mode (No Containers)

```bash
# Terminal 1: Start API server
cd ~/Projects/just-in-time
export JIT_DATA_DIR=~/test-jit-project/.jit
cargo run --bin jit-server
# Server running on http://localhost:3000

# Terminal 2: Start Web UI dev server
cd ~/Projects/just-in-time/web
npm install
npm run dev
# UI running on http://localhost:5173
```

This is fastest for development with hot-reload on both frontend and backend changes.

### Option 3: Docker Compose (Production-Like)

**Note:** This approach may have permission issues with rootless Podman. Use API on host (above) for development.

```bash
# Edit docker-compose.yml to mount your project:
# api:
#   volumes:
#     - ~/test-jit-project/.jit:/data:z
#   user: "1000:1000"  # Your user ID

cd ~/Projects/just-in-time
docker-compose up -d

# Access Web UI: http://localhost:8080
# Access API: http://localhost:3000
```

### Web UI Features

Once running, open http://localhost:8080 (Docker) or http://localhost:5173 (dev):

1. **Graph View** (left panel):
   - Interactive dependency graph
   - Color-coded by state (backlog, ready, in_progress, gated, done)
   - Click nodes to select issues
   - Zoom and pan
   - Left-to-right DAG layout

2. **Issue Detail** (right panel):
   - Full issue information
   - State and priority badges
   - Dependencies list
   - Quality gates status
   - Document references (click to view)

3. **Search Bar** (top):
   - Type to search (⚡ instant results)
   - Searches ID, title, description
   - Click result to navigate

4. **Document Viewer** (modal):
   - Click document in issue detail
   - Markdown rendering with syntax highlighting
   - View history (commit timeline)
   - LaTeX math support
   - Mermaid diagram support

5. **Dark Mode Toggle** (top-right):
   - Terminal-style dark theme
   - Light mode option

---

## Using the MCP Server (AI Integration)

The MCP (Model Context Protocol) server allows AI agents and tools to interact with JIT programmatically.

### Setup MCP Server

```bash
# Build and link MCP server
cd ~/Projects/just-in-time/mcp-server
npm install
npm link  # Makes jit-mcp-server globally available

# Test it
jit-mcp-server --version
```

### Configure for AI Tools

The MCP server can be integrated with AI assistants that support the Model Context Protocol. Configuration varies by tool - check your AI assistant's documentation for MCP server setup.

Example environment configuration:
```bash
export JIT_DATA_DIR=/path/to/your/project/.jit
jit-mcp-server
```

### MCP Commands Available to AI

The AI agent can programmatically:
- Create, read, update, delete issues
- Add dependencies between issues
- Manage quality gates
- Add document references
- Query ready/blocked issues
- Search across issues and documents
- View dependency graphs
- Validate repository integrity

**Example AI interaction:**

```
User: "Create an issue for implementing authentication"
AI: [uses jit_issue_create tool]
    "Created issue 06PQR678STU: Implement authentication"

User: "Make it depend on the parser issue"
AI: [uses jit_dep_add tool]
    "Added dependency: 06PQR678STU depends on 02DEF456ABC"

User: "What's ready to work on?"
AI: [uses jit_query_ready tool]
    "Currently ready: 03GHI789DEF (Build template engine)"
```

---

## Multi-Agent Workflow

JIT is designed for coordinating multiple AI agents or human workers.

### Scenario: Two Agents Working Concurrently

```bash
# Terminal 1: Agent 1 claims next ready issue
jit issue claim-next agent:worker-1
# Output: Claimed 03GHI789DEF: Build template engine

jit issue update 03GHI789DEF --state in_progress

# Terminal 2: Agent 2 tries to claim the same issue (fails)
jit issue claim 03GHI789DEF agent:worker-2
# Error: Issue already assigned to agent:worker-1

# Terminal 2: Agent 2 gets a different issue
jit issue claim-next agent:worker-2
# Output: Claimed <next-ready-issue>

# File locking prevents data corruption
# Both agents can safely read/write concurrently
```

### Using the Dispatcher (Optional)

The dispatcher automates agent assignment:

```bash
# Create dispatch config
cat > dispatch.toml <<'EOF'
[coordinator]
poll_interval_secs = 10

[[agents]]
identifier = "agent:worker-1"
capacity = 3

[[agents]]
identifier = "agent:worker-2"
capacity = 2
EOF

# Run dispatcher (assign issues to agents automatically)
jit-dispatch start --config dispatch.toml
# Dispatcher will:
# 1. Poll for ready issues every 10 seconds
# 2. Assign to agents based on capacity and priority
# 3. Log assignment events
```

---

## Verification & Testing

### Validate Repository Integrity

```bash
# Check for issues
jit validate
# Output:
# ✓ DAG integrity: no cycles
# ✓ All dependencies exist
# ✓ All gate references valid
# ✓ Document references valid

# Check with verbose output
jit validate --verbose
```

### Test Event Log

```bash
# View recent events
jit events tail
# Output:
# 2025-12-06T20:30:00Z | issue.created | 01ABC123XYZ
# 2025-12-06T20:31:15Z | issue.claimed | 01ABC123XYZ | agent:dev-1
# 2025-12-06T20:35:20Z | issue.state_changed | 01ABC123XYZ | backlog -> in_progress
```

### Export and Visualize

```bash
# Export graph as Mermaid
jit graph export --format mermaid > graph.mmd

# View the Mermaid code
cat graph.mmd
# Output:
# graph LR
#   01ABC123XYZ["Setup project infrastructure<br/>backlog | high"]
#   02DEF456ABC["Implement markdown parser<br/>done | high"]
#   ...

# Render with online tool: https://mermaid.live
```

### Test Search (if ripgrep installed)

```bash
# Search issues
jit search "parser"
# Should find parser-related issues

# Search documents
jit search "architecture" --glob "docs/*.md"
# Should find design.md mentions

# Performance test (large repo)
time jit search "function"
# Should complete in <100ms for reasonable repos
```

---

## Next Steps

### 1. Explore Advanced Features

```bash
# Break down large issues into subtasks
jit issue breakdown <parent-id>

# Add custom context to issues
jit issue update <id> --context "complexity:high" --context "team:backend"

# Try transitive reduction (minimal edge set)
jit graph show --reduce

# Historical document viewing
jit doc show <id> <path> --at <commit-hash>
```

### 2. Integrate with CI/CD

```bash
# Add to your CI pipeline (e.g., .github/workflows/ci.yml)
# - Run: jit validate
# - Fail build if validation fails
# - Auto-pass gates on successful test runs
```

### 3. Scale to Real Project

- Initialize JIT in your actual project: `cd ~/my-real-project && jit init`
- Import existing issues from Jira/GitHub Issues (write a script)
- Set up MCP server for AI pair programming
- Use Web UI for visualization and exploration

### 4. Customize Workflow

- Define custom gates for your process (e.g., `security-review`, `performance-test`)
- Create naming conventions for agents (e.g., `human:alice`, `agent:copilot-session-1`)
- Set up webhooks (future feature) for Slack notifications

### 5. Read Documentation

- **Design**: `docs/design.md` - Comprehensive system design
- **Architecture**: `docs/web-ui-architecture.md` - Web UI structure
- **Knowledge Management**: `docs/knowledge-management-vision.md` - Long-term vision
- **Deployment**: `DEPLOYMENT.md` - Production deployment guide
- **Testing**: `TESTING.md` - Test strategy

---

## Troubleshooting

### Issue: "Command not found: jit"

**Solution**: Add to PATH
```bash
# If built from source:
export PATH="$HOME/Projects/just-in-time/target/release:$PATH"

# If binary install:
export PATH="$HOME/.local/bin:$PATH"

# Make permanent (add to ~/.bashrc):
echo 'export PATH="$HOME/.local/bin:$PATH"' >> ~/.bashrc
source ~/.bashrc
```

### Issue: "No .jit directory found"

**Solution**: Initialize repository
```bash
cd /path/to/project
jit init
```

### Issue: Web UI shows "Failed to fetch issues" or API returns 500 errors

**Solution**: Check API server and permissions
```bash
# 1. Is server running?
curl http://localhost:3000/api/health
# Should return: {"service":"jit-api","status":"ok","version":"0.1.0"}

# 2. If using containers, check logs:
podman logs jit-api  # or docker logs jit-api

# 3. Permission errors? Run API on host instead:
# Stop containerized API
podman stop jit-api

# Run directly on host (recommended for development)
cd ~/Projects/just-in-time
export JIT_DATA_DIR=~/your-project/.jit
./target/release/jit-server
# Server running on http://localhost:3000

# 4. Keep Web UI in container
podman run -d --name jit-web -p 8080:80 --network host jit-web
```

**Why run API on host?** Containerized API servers can have permission issues with volume mounts, especially with Podman's rootless mode. Running the API directly on your host eliminates these issues since it runs as your user.

### Issue: "Search not working"

**Solution**: Install ripgrep
```bash
sudo apt install ripgrep
rg --version  # Verify installation

# Test:
jit search "test"
```

### Issue: MCP server not connecting

**Solution**: Check configuration
```bash
# Verify jit-mcp-server is installed
which jit-mcp-server

# Test it runs
jit-mcp-server --version

# Verify JIT_DATA_DIR points to correct location
echo $JIT_DATA_DIR
# Should point to a directory containing .jit/

# Check your AI tool's MCP configuration
# (varies by tool - see tool documentation)
```

---

## Summary: What You've Learned

✅ Installed JIT from source  
✅ Initialized a test repository  
✅ Created issues with dependencies  
✅ Added quality gates  
✅ Linked documentation to issues  
✅ Used CLI commands for workflow  
✅ Explored Web UI visualization  
✅ Set up MCP server for AI agents  
✅ Validated repository integrity  
✅ Exported and visualized graphs  

**You're now ready to use JIT for real projects!**

---

## Feedback & Contribution

Found a bug? Have a feature request? Want to contribute?

- **Issues**: https://github.com/erankavija/just-in-time/issues
- **Pull Requests**: https://github.com/erankavija/just-in-time/pulls
- **Documentation**: `docs/` directory
- **Discussions**: GitHub Discussions (TBD)

**Happy tracking! 🚀**
