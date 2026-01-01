#!/bin/bash
# agent-init-demo-project.sh
# 
# Demo script for AI agent to initialize a complete JIT project structure
# following best practices with milestone → epics → tasks hierarchy
#
# Usage: ./agent-init-demo-project.sh

set -e  # Exit on error

echo "🚀 JIT Agent Project Initialization"
echo "===================================="
echo ""

# Check if jit is available
if ! command -v jit &> /dev/null; then
    echo "❌ Error: 'jit' command not found"
    echo "Please build and add to PATH:"
    echo "  cargo build --release --workspace"
    echo "  export PATH=\"\$PWD/target/release:\$PATH\""
    exit 1
fi

# Check if already initialized
if [ ! -d ".jit" ]; then
    echo "⚠️  Not in a JIT repository. Run 'jit init' first."
    exit 1
fi

echo "📋 Phase 1: Setup Gate Registry"
echo "--------------------------------"
jit registry add unit-tests --title "Unit Tests" --auto 2>/dev/null || echo "  ↳ unit-tests already exists"
jit registry add integration-tests --title "Integration Tests" --auto 2>/dev/null || echo "  ↳ integration-tests already exists"
jit registry add review --title "Code Review" 2>/dev/null || echo "  ↳ review already exists"
jit registry add security-scan --title "Security Scan" --auto 2>/dev/null || echo "  ↳ security-scan already exists"
jit registry add docs --title "Documentation" 2>/dev/null || echo "  ↳ docs already exists"
echo "✅ Gate registry configured"
echo ""

echo "📦 Phase 2: Create Milestone"
echo "----------------------------"
MILESTONE=$(jit issue create \
  --title "Release v1.0 - MVP Launch" \
  --desc "First production-ready release with core features" \
  --label "type:milestone" \
  --label "milestone:v1.0" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "✅ Created milestone: $MILESTONE"
echo ""

echo "🎯 Phase 3: Create Epics"
echo "------------------------"

AUTH_EPIC=$(jit issue create \
  --title "User Authentication System" \
  --desc "JWT-based authentication with login, logout, token refresh" \
  --label "type:epic" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --priority high \
  --gate integration-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ Auth Epic: $AUTH_EPIC"

API_EPIC=$(jit issue create \
  --title "RESTful API Endpoints" \
  --desc "CRUD operations for main entities with proper error handling" \
  --label "type:epic" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --priority high \
  --gate integration-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ API Epic: $API_EPIC"

UI_EPIC=$(jit issue create \
  --title "Web User Interface" \
  --desc "React-based responsive UI with modern design" \
  --label "type:epic" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --priority high \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ UI Epic: $UI_EPIC"

INFRA_EPIC=$(jit issue create \
  --title "Database & Infrastructure Setup" \
  --desc "PostgreSQL schema, migrations, deployment configuration" \
  --label "type:epic" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --priority critical \
  --gate review --gate security-scan | grep -oP 'Created issue: \K\S+')
echo "  ✅ Infra Epic: $INFRA_EPIC"

echo ""
echo "🔗 Phase 4: Link Epics to Milestone"
echo "------------------------------------"
jit dep add $MILESTONE $AUTH_EPIC
jit dep add $MILESTONE $API_EPIC
jit dep add $MILESTONE $UI_EPIC
jit dep add $MILESTONE $INFRA_EPIC
echo "✅ Dependencies configured"
echo ""

echo "✅ Phase 5: Create Infrastructure Tasks (Foundation)"
echo "----------------------------------------------------"

INFRA_T1=$(jit issue create \
  --title "Design PostgreSQL database schema" \
  --desc "Define tables, relationships, indexes for all entities" \
  --label "type:task" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority critical \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $INFRA_T1: Database schema"

INFRA_T2=$(jit issue create \
  --title "Setup Alembic database migrations" \
  --desc "Initialize migration framework and create initial migration" \
  --label "type:task" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority critical \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $INFRA_T2: Migrations setup"

INFRA_T3=$(jit issue create \
  --title "Configure database connection pooling" \
  --desc "Setup SQLAlchemy connection pool with proper settings" \
  --label "type:task" \
  --label "epic:infra" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $INFRA_T3: Connection pooling"

jit dep add $INFRA_EPIC $INFRA_T1
jit dep add $INFRA_EPIC $INFRA_T2
jit dep add $INFRA_EPIC $INFRA_T3
jit dep add $INFRA_T2 $INFRA_T1  # Migrations need schema
jit dep add $INFRA_T3 $INFRA_T2  # Pooling needs migrations
echo ""

echo "🔐 Phase 6: Create Authentication Tasks"
echo "---------------------------------------"

AUTH_T1=$(jit issue create \
  --title "Create User model with authentication fields" \
  --desc "SQLAlchemy model: id, email, password_hash, created_at, updated_at" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $AUTH_T1: User model"

AUTH_T2=$(jit issue create \
  --title "Implement JWT token generation and validation" \
  --desc "Utility functions for creating, signing, and verifying JWT tokens" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $AUTH_T2: JWT utilities"

AUTH_T3=$(jit issue create \
  --title "Build POST /api/auth/login endpoint" \
  --desc "Accept email/password, validate credentials, return JWT access token" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $AUTH_T3: Login endpoint"

AUTH_T4=$(jit issue create \
  --title "Create authentication middleware" \
  --desc "FastAPI dependency to verify JWT and inject user into request context" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $AUTH_T4: Auth middleware"

AUTH_T5=$(jit issue create \
  --title "Write authentication integration tests" \
  --desc "E2E tests: successful login, invalid credentials, token validation, protected routes" \
  --label "type:task" \
  --label "epic:auth" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority normal \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $AUTH_T5: Integration tests"

jit dep add $AUTH_EPIC $AUTH_T1
jit dep add $AUTH_EPIC $AUTH_T2
jit dep add $AUTH_EPIC $AUTH_T3
jit dep add $AUTH_EPIC $AUTH_T4
jit dep add $AUTH_EPIC $AUTH_T5
jit dep add $AUTH_T1 $INFRA_T2  # User model needs migrations
jit dep add $AUTH_T3 $AUTH_T1   # Login needs User model
jit dep add $AUTH_T3 $AUTH_T2   # Login needs JWT utils
jit dep add $AUTH_T4 $AUTH_T2   # Middleware needs JWT utils
jit dep add $AUTH_T5 $AUTH_T1   # Tests need all components
jit dep add $AUTH_T5 $AUTH_T2
jit dep add $AUTH_T5 $AUTH_T3
jit dep add $AUTH_T5 $AUTH_T4
echo ""

echo "🌐 Phase 7: Create API Tasks"
echo "----------------------------"

API_T1=$(jit issue create \
  --title "Create FastAPI router structure" \
  --desc "Setup router modules with proper separation of concerns" \
  --label "type:task" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $API_T1: Router structure"

API_T2=$(jit issue create \
  --title "Implement CRUD endpoints for main entities" \
  --desc "GET, POST, PUT, DELETE endpoints with proper validation" \
  --label "type:task" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $API_T2: CRUD endpoints"

API_T3=$(jit issue create \
  --title "Add API error handling and validation" \
  --desc "Global exception handler, Pydantic validation, proper HTTP status codes" \
  --label "type:task" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $API_T3: Error handling"

API_T4=$(jit issue create \
  --title "Write API integration tests" \
  --desc "Test all CRUD operations, error cases, validation" \
  --label "type:task" \
  --label "epic:api" \
  --label "milestone:v1.0" \
  --label "component:backend" \
  --priority normal \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $API_T4: Integration tests"

jit dep add $API_EPIC $API_T1
jit dep add $API_EPIC $API_T2
jit dep add $API_EPIC $API_T3
jit dep add $API_EPIC $API_T4
jit dep add $API_T1 $AUTH_T4   # Router needs auth middleware
jit dep add $API_T2 $API_T1    # CRUD needs router
jit dep add $API_T3 $API_T2    # Error handling integrated with CRUD
jit dep add $API_T4 $API_T2    # Tests need all components
jit dep add $API_T4 $API_T3
echo ""

echo "💻 Phase 8: Create Frontend Tasks"
echo "---------------------------------"

UI_T1=$(jit issue create \
  --title "Setup React project with Vite" \
  --desc "Initialize React app with TypeScript, routing, state management" \
  --label "type:task" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --label "component:frontend" \
  --priority high \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $UI_T1: React setup"

UI_T2=$(jit issue create \
  --title "Create authentication UI components" \
  --desc "Login form, logout button, protected route wrapper" \
  --label "type:task" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --label "component:frontend" \
  --priority high \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $UI_T2: Auth components"

UI_T3=$(jit issue create \
  --title "Build main dashboard interface" \
  --desc "Responsive layout with navigation, data display" \
  --label "type:task" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --label "component:frontend" \
  --priority high \
  --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $UI_T3: Dashboard"

UI_T4=$(jit issue create \
  --title "Implement API client with authentication" \
  --desc "Axios/fetch wrapper with JWT token handling" \
  --label "type:task" \
  --label "epic:ui" \
  --label "milestone:v1.0" \
  --label "component:frontend" \
  --priority high \
  --gate unit-tests --gate review | grep -oP 'Created issue: \K\S+')
echo "  ✅ $UI_T4: API client"

jit dep add $UI_EPIC $UI_T1
jit dep add $UI_EPIC $UI_T2
jit dep add $UI_EPIC $UI_T3
jit dep add $UI_EPIC $UI_T4
jit dep add $UI_T2 $UI_T1   # Auth components need React setup
jit dep add $UI_T3 $UI_T1   # Dashboard needs React setup
jit dep add $UI_T4 $UI_T1   # API client needs React setup
jit dep add $UI_T2 $UI_T4   # Auth components need API client
jit dep add $UI_T3 $UI_T4   # Dashboard needs API client
echo ""

echo "📊 Phase 9: Project Summary"
echo "============================="
echo ""

# Count by type
echo "Issue Count by Type:"
jit issue list --json | jq -r '[.[] | .labels[] | select(startswith("type:"))] | group_by(.) | map("\(.[]): \(length)") | .[]' | sed 's/type:/  /'

echo ""
echo "Strategic View (Milestone + Epics):"
jit query strategic --json 2>/dev/null | jq -r '.[] | "  [\(.labels[] | select(startswith("type:")))] \(.title)"' || echo "  (Use 'jit query strategic' to view)"

echo ""
echo "Dependency Graph Visualization:"
echo "------------------------------"
jit graph show $MILESTONE

echo ""
echo "Ready Issues (foundation to start with):"
jit query ready --json 2>/dev/null | jq -r '.[] | "  \(.id): \(.title)"' || echo "  None (all blocked by dependencies)"

echo ""
echo "✅ Project Structure Complete!"
echo "=============================="
echo ""
echo "📌 Key IDs:"
echo "  Milestone: $MILESTONE"
echo "  Auth Epic: $AUTH_EPIC"
echo "  API Epic:  $API_EPIC"
echo "  UI Epic:   $UI_EPIC"
echo "  Infra Epic: $INFRA_EPIC"
echo ""
echo "🚀 Next Steps:"
echo "  1. Mark foundation task as ready:  jit issue update $INFRA_T1 --state ready"
echo "  2. Claim and start work:           jit issue claim $INFRA_T1 agent:your-name"
echo "  3. Monitor progress:               jit status"
echo "  4. View full graph:                jit graph show $MILESTONE"
echo ""
echo "📚 Resources:"
echo "  - Full guide: docs/agent-project-initialization-guide.md"
echo "  - Examples: docs/tutorials/"
echo "  - Label conventions: docs/label-conventions.md"
echo ""
