# JIT Web UI

A React, TypeScript, and Vite single-page app for visualizing a JIT issue tracker. It renders the dependency DAG, per-issue detail, linked documents, and label groupings from a JIT repository, reading everything over the HTTP API exposed by `jit-server`.

## How it fits together

The web UI is a static front end. It holds no data of its own: at runtime it calls the JIT REST API under `/api` on its own origin, and the `jit-server` binary (`crates/server/`) serves that API by embedding the core `jit` library in-process, reading the repository's `.jit/` directory directly.

```mermaid
flowchart LR
    Browser["Web UI (this package)<br/>React + Vite SPA"]
    Server["jit-server<br/>(crates/server)"]
    Lib["jit library<br/>(CommandExecutor)"]
    Data[".jit/ repository"]

    Browser -->|"HTTP /api"| Server
    Server --> Lib
    Lib --> Data
```

`jit-server` can serve the built assets itself: when the web UI is built and compiled in with the `embed-web` feature, the server serves the SPA at `/` alongside the API at `/api`. It also accepts a `--web-dir` pointing at a `dist/` directory to serve from the filesystem. A live event stream (`/api/events/stream`) drives updates as the repository changes.

## What it renders

- **Graph view** (`src/components/Graph`) lays out the dependency DAG with `reactflow` and `dagre`.
- **Issue view** (`src/components/Issue`) shows issue detail, state, gates, and dependencies.
- **Documents** (`src/components/Document`) render linked markdown with `react-markdown`, GitHub-flavored markdown, Mermaid diagrams, and KaTeX math.
- **Labels** (`src/components/Labels`) and **Search** (`src/components/Search`) navigate issues by label namespace and free text.

## Scripts

Defined in `package.json`:

```bash
npm run dev          # Start the Vite dev server with hot reload
npm run build        # Type-check (tsc -b) and produce a production build in dist/
npm run preview      # Serve the production build locally
npm run lint         # Run ESLint over the sources
npm test             # Run the Vitest unit tests once
npm run test:watch   # Run Vitest in watch mode
```

## Local development

The last step runs a `jit-server` binary, so build or install one first
([Installation Guide](../INSTALL.md)):

```bash
npm install
npm run build
cd ..
jit-server --data-dir .jit --web-dir web/dist
```

Open `http://localhost:3000`. This makes the UI and its `/api` requests same-origin.

`npm run dev` remains useful for frontend asset work, but it starts Vite on a separate origin
and this repository's Vite configuration has no `/api` proxy. It therefore does not connect to a
separately started `jit-server`; configure a reverse proxy for `/api` if you need that workflow.

## License

MIT OR Apache-2.0 (matches parent project)
