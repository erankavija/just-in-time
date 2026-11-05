# Web rendering bundle verification

The production entry chunk falls from 2,089.92 kB on dispatch base `84f5ff7b8`
to 489.53 kB. Every emitted JavaScript chunk remains below Vite's unchanged
500 kB warning limit. The production build emits no oversized-chunk warning.

## Rendering boundaries

Syntax highlighting uses the existing asynchronous full Prism registry, preserving
canonical languages and aliases without maintaining an application language list.
The registry's grammar modules occupy two deterministic ranges. Mermaid's unchanged
configuration and complete runtime load when a diagram is mounted. React, math, and
React Flow have explicit shared chunks; the CommonJS layout dependency stays with
the application to preserve initialization order.

## Verification evidence

- The configured web gate passes the full suite, lint, and production build.
- Focused tests cover asynchronous highlighting of canonical names and aliases,
  readable unknown-language source, and source changes before Mermaid loads.
- Headless Chromium loads the actual minified application, renders a two-node
  dependency graph, and observes that document runtimes have not loaded before
  selection. After a real SSE refresh it renders highlighted JavaScript through
  the `js` alias and the description's Mermaid diagram.
- A separate production-module smoke renders flowchart, sequence, mindmap, and
  treemap diagrams and exercises `js`, `ts`, `rust`, and `yml` highlighting.
- A byte-size assertion over every generated JavaScript file passes. The largest
  chunks are the application (489.53 kB), Mermaid runtime (454.50 kB), and
  Cytoscape (441.74 kB); the two syntax ranges are 327.47 and 260.01 kB.

## Pre-existing selection defect

Initial issue selection remains loading until another refresh. The same browser
fixture reproduces this behavior on the unchanged dispatch base and on this patch;
its successful rendering check follows a real SSE change event. Task `4858de06`
owns the separate repair.

A minimal reproduction mounts `IssueDetail` with an issue ID and an asynchronous,
successful `getIssue` response. The load effect increments its request counter,
then the later issue-change effect resets that counter. The response fails the
counter check and does not clear loading. The cheapest regression owner is an
`IssueDetail` component test; this bundle task changes only that component's
highlighter import.
