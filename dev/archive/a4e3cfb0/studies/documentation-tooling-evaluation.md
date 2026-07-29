# Documentation Tooling Evaluation for v1.0

**Date:** 2026-01-02
**Issue:** [To be created]
**Goal:** Evaluate and decide on documentation tooling before v1.0 release

## Current State

**Documentation structure:**
- 19 markdown files in `docs/` following Diátaxis framework
- Clean hierarchy: concepts, tutorials, how-to, reference
- Currently viewed as raw markdown on GitHub/GitLab
- No search capability beyond repo search
- Navigation via index.md

**Works well for:**
- AI agents (can read markdown directly)
- Developers (comfortable with markdown)
- Current development phase (documentation still being written)

**Pain points:**
- No built-in search across documentation
- Manual navigation (no sidebar/TOC)
- Basic visual appearance
- Hard to discover related content

## Evaluation Criteria

**Must have:**
1. **Markdown preservation** - Source files stay as .md for agent readability
2. **Low maintenance** - Minimal config/overhead to update
3. **GitLab Pages compatible** - Easy deployment
4. **Search capability** - Full-text search across all docs

**Nice to have:**
5. Navigation sidebar with table of contents
6. Professional appearance for external users
7. Mobile-responsive design
8. Dark mode support

**Deal breakers:**
- Losing markdown source (agents need raw .md access)
- Complex build pipeline requiring multiple tools
- High maintenance burden (manual TOC updates for every new doc)

## Options Analysis

### Option 1: Docsify ⭐ RECOMMENDED

**Type:** Client-side JavaScript renderer (no build step)

**Setup time:** 5 minutes

**How it works:**
- Single `index.html` file loads markdown on-the-fly
- Simple `_sidebar.md` file defines navigation
- All markdown files stay unchanged
- Renders in browser using JavaScript

**Pros:**
✅ **Zero build step** - No CI/CD complexity
✅ **5-minute setup** - One HTML file, one sidebar file
✅ **Full-text search** - Built-in search plugin
✅ **Markdown preserved** - Source files unchanged
✅ **GitLab Pages ready** - Just commit and push
✅ **Modern UI** - Clean, professional appearance
✅ **Agent-compatible** - Markdown still directly readable

**Cons:**
❌ **JavaScript required** - Agents without JS see raw markdown (acceptable)
❌ **Client-side rendering** - Slightly slower initial load

**Maintenance:**
- Update `_sidebar.md` when adding new docs (simple markdown list)
- No build config, no dependencies, no CI changes

**GitLab Pages setup:**
```yaml
# .gitlab-ci.yml
pages:
  stage: deploy
  script:
    - mkdir .public
    - cp -r docs/* .public
    - mv .public public
  artifacts:
    paths:
      - public
  only:
    - main
```

**Example implementation:**
```html
<!-- docs/index.html -->
<!DOCTYPE html>
<html>
<head>
  <meta charset="UTF-8">
  <title>JIT Documentation</title>
  <link rel="stylesheet" href="//cdn.jsdelivr.net/npm/docsify/themes/vue.css">
</head>
<body>
  <div id="app">Loading...</div>
  <script>
    window.$docsify = {
      name: 'JIT Documentation',
      repo: 'https://github.com/erankavija/just-in-time',
      loadSidebar: true,
      subMaxLevel: 3,
      search: 'auto',
      auto2top: true
    }
  </script>
  <script src="//cdn.jsdelivr.net/npm/docsify/lib/docsify.min.js"></script>
  <script src="//cdn.jsdelivr.net/npm/docsify/lib/plugins/search.min.js"></script>
</body>
</html>
```

---

### Option 2: Just-the-Docs (Jekyll)

**Type:** Static site generator (Jekyll theme)

**Setup time:** 30 minutes

**How it works:**
- Jekyll builds markdown → HTML
- Navigation from frontmatter in each file
- GitLab Pages has native Jekyll support

**Pros:**
✅ **Beautiful UI** - Professional documentation theme
✅ **Search built-in** - Full-text search
✅ **GitLab native** - Pages builds Jekyll automatically
✅ **Markdown preserved** - Source files stay .md

**Cons:**
❌ **Build step required** - Jekyll processing (handled by GitLab)
❌ **Frontmatter needed** - Every file needs YAML header
❌ **More complex** - Jekyll/Ruby ecosystem

**Maintenance:**
- Add frontmatter to each new doc (title, parent, nav_order)
- More moving parts than Docsify

**Example frontmatter:**
```yaml
---
title: Core Model
parent: Concepts
nav_order: 2
---
# Core Model
(content unchanged)
```

---

### Option 3: MDBook

**Type:** Static site generator (Rust)

**Setup time:** 1-2 hours

**How it works:**
- Rust binary generates HTML from markdown
- Manual SUMMARY.md defines navigation
- Designed for book-like documentation

**Pros:**
✅ **Great UI** - Clean, modern appearance
✅ **Search built-in** - Fast full-text search
✅ **Rust ecosystem** - Fits project tech stack

**Cons:**
❌ **Build dependency** - Requires `mdbook` installed
❌ **SUMMARY.md maintenance** - Manual TOC for every doc
❌ **Linear structure** - Better for books than Diátaxis quadrants
❌ **More complex** - Build step, CI integration

**Maintenance:**
- Update SUMMARY.md for every new doc
- Build step in CI pipeline

---

### Option 4: Current State (Raw Markdown)

**Type:** No tooling

**Setup time:** 0 (current state)

**How it works:**
- GitLab renders markdown natively
- Navigation via index.md links

**Pros:**
✅ **Zero setup** - Already works
✅ **Zero maintenance** - No builds, no config
✅ **Agent-perfect** - Pure markdown

**Cons:**
❌ **No search** - Only repo-level search
❌ **Basic UI** - GitLab's markdown renderer
❌ **Manual navigation** - Click through links
❌ **Not professional** - For external users

**Maintenance:**
- None

---

### Option 5: MkDocs Material

**Type:** Static site generator (Python)

**Setup time:** 1-2 hours

**How it works:**
- Python builds markdown → HTML
- Config in mkdocs.yml

**Pros:**
✅ **Best UI** - Material Design theme is beautiful
✅ **Great search** - Fast, comprehensive
✅ **Feature-rich** - Tabs, admonitions, diagrams

**Cons:**
❌ **Python dependency** - Another ecosystem
❌ **Build complexity** - Similar to MDBook
❌ **Not simpler** - Same complexity as MDBook, different stack

**Verdict:** Not simpler than MDBook, just different. Skip.

---

## Comparison Matrix

| Tool | Setup | Build | Search | UI | Agent-Friendly | Maintenance |
|------|-------|-------|--------|-----|---------------|-------------|
| **Current (Raw MD)** | 0 min | ❌ None | ⚠️ Repo only | ⭐⭐ | ✅ Perfect | ⭐⭐⭐⭐⭐ |
| **Docsify** | 5 min | ❌ None | ✅ Full-text | ⭐⭐⭐⭐ | ✅ Yes | ⭐⭐⭐⭐ |
| **Just-the-Docs** | 30 min | ✅ Jekyll | ✅ Full-text | ⭐⭐⭐⭐ | ✅ Yes | ⭐⭐⭐⭐ |
| **MDBook** | 1-2 hrs | ✅ Rust | ✅ Full-text | ⭐⭐⭐⭐ | ✅ Yes | ⭐⭐⭐ |
| **MkDocs** | 1-2 hrs | ✅ Python | ✅ Full-text | ⭐⭐⭐⭐⭐ | ✅ Yes | ⭐⭐⭐ |

---

## Recommendation

### Decision: **Docsify**

**Rationale:**
1. **Minimal setup** - 5 minutes to add index.html and _sidebar.md
2. **Zero build complexity** - No CI changes, no build tools
3. **Search included** - Solves main pain point
4. **Professional appearance** - Good enough for v1.0
5. **Maintains markdown** - Agents can still read source files
6. **Low maintenance** - Just update _sidebar.md when adding docs

### Implementation Plan

**Phase 1: Setup (before v1.0)**
1. Create `docs/index.html` with Docsify config (5 min)
2. Create `docs/_sidebar.md` from current index.md structure (10 min)
3. Test locally with `python -m http.server`
4. Add GitLab Pages CI config if needed
5. Verify all internal links work
6. Document in README.md

**Phase 2: Polish (after v1.0 if needed)**
- Customize theme/colors
- Add custom plugins (if needed)
- Consider upgrade to MDBook if documentation grows significantly

### When to Reconsider

Upgrade from Docsify to MDBook/MkDocs if:
- Documentation grows to 50+ files (complex navigation)
- Need versioned docs (multiple releases)
- Want offline PDF generation
- Community requests it

For v1.0, Docsify hits the sweet spot of professional + simple.

---

## Acceptance Criteria

**Before v1.0 release:**
- [ ] Evaluate all options (this document)
- [ ] Make decision (Docsify recommended)
- [ ] Implement chosen solution
- [ ] Verify search works across all docs
- [ ] Verify mobile/desktop rendering
- [ ] Update README.md with link to hosted docs
- [ ] Test with agents (ensure markdown still accessible)
- [ ] Document maintenance process

**Success metrics:**
- Users can find relevant docs via search
- Navigation is intuitive (sidebar TOC)
- Documentation looks professional
- Setup took < 1 hour
- Maintenance burden is low

---

## References

- **Docsify:** https://docsify.js.org/
- **Just-the-Docs:** https://just-the-docs.github.io/just-the-docs/
- **MDBook:** https://rust-lang.github.io/mdBook/
- **MkDocs Material:** https://squidfunk.github.io/mkdocs-material/
- **Diátaxis Framework:** https://diataxis.fr/

---

## Notes

- Current documentation is ~30% complete (many drafts)
- Focus should remain on content completion, not tooling
- Docsify can be added in 15 minutes when ready
- No rush - can add after more content is written
- GitLab Pages setup is minimal regardless of tool choice
