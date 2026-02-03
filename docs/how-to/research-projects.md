# How-To: Research Projects

> **Diátaxis Type:** How-To Guide

Practical recipes for academic research, experiments, and literature management.

## Track Research Questions

### Recipe: Research Question Hierarchy

```bash
# Main research question as epic
jit issue create --title "RQ: Does X improve Y?" \
  --label "type:epic" --label "research:main-study"

# Sub-questions as stories
jit issue create --title "RQ1: Baseline measurement" \
  --label "type:story" --label "research:main-study"

jit issue create --title "RQ2: Intervention effect" \
  --label "type:story" --label "research:main-study"

# RQ2 depends on RQ1 baseline
jit dep add <rq2-id> <rq1-id>

# Tasks under each sub-question
jit issue create --title "Design baseline survey" \
  --label "type:task" --label "research:main-study"
```

## Manage Literature Review

### Recipe: Paper Reading Queue

```bash
# Literature review epic
jit issue create --title "Literature Review: Topic X" \
  --label "type:epic" --label "lit-review:topic-x"

# Papers as tasks
jit issue create --title "Read: Smith 2024 - Key Paper" \
  --label "type:task" --label "lit-review:topic-x" \
  --priority critical

jit issue create --title "Read: Jones 2023 - Background" \
  --label "type:task" --label "lit-review:topic-x"

# Reading order (Jones before Smith)
jit dep add <smith-id> <jones-id>

# Link PDFs and notes
jit doc add <smith-id> papers/smith-2024.pdf
jit doc add <smith-id> notes/smith-2024-notes.md
```

### Literature Status

```bash
# What's left to read?
jit query available --label "lit-review:*"

# What's been reviewed?
jit query closed --label "lit-review:topic-x"
```

## Coordinate Experiments

### Recipe: Experiment Validation

```bash
# Experiment with quality gates
jit gate define irb-approved --title "IRB Approval" --mode manual
jit gate define data-collected --title "Data Collection Complete" --mode manual
jit gate define analysis-done --title "Statistical Analysis Done" --mode manual

jit issue create --title "Experiment 1: User Study" \
  --label "type:task" --label "research:main-study" \
  --gate irb-approved --gate data-collected --gate analysis-done

# Must have IRB before data collection
jit issue create --title "Submit IRB application" \
  --label "type:task" --label "research:main-study"

jit dep add <experiment-id> <irb-id>
```

### Recipe: Reproducibility Checklist

```bash
# Gates for reproducible research
jit gate define code-documented --title "Code Documented" --mode manual
jit gate define data-archived --title "Data Archived" --mode manual
jit gate define methods-described --title "Methods Section Complete" --mode manual

jit issue create --title "Prepare replication package" \
  --label "type:task" --label "research:main-study" \
  --gate code-documented --gate data-archived --gate methods-described
```

## Document Findings

### Recipe: Paper Writing Workflow

```bash
# Paper sections as dependent tasks
jit issue create --title "Write: Introduction" \
  --label "type:task" --label "paper:main-paper"

jit issue create --title "Write: Methods" \
  --label "type:task" --label "paper:main-paper"

jit issue create --title "Write: Results" \
  --label "type:task" --label "paper:main-paper"

# Results depends on experiments being done
jit dep add <results-id> <experiment-id>

# Co-author review gates
jit gate define coauthor-reviewed --title "Co-author Approval" --mode manual

jit issue create --title "Final paper draft" \
  --label "type:task" --label "paper:main-paper" \
  --gate coauthor-reviewed

# Link manuscript
jit doc add <paper-id> manuscript/main-paper.tex
```

### Track Revisions

```bash
# Reviewer feedback as tasks
jit issue create --title "Address Reviewer 1 comments" \
  --label "type:task" --label "paper:main-paper" \
  --label "revision:r1"

jit issue create --title "Address Reviewer 2 comments" \
  --label "type:task" --label "paper:main-paper" \
  --label "revision:r1"

# Track by revision round
jit query all --label "revision:r1"
```

## See Also

- [Knowledge Work](knowledge-work.md) - Personal productivity patterns
- [Dependency Management](dependency-management.md) - Complex dependency graphs
- [CLI Reference](../reference/cli-commands.md) - Full command documentation
