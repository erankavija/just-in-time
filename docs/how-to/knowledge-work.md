# How-To: Knowledge Work

> **Diátaxis Type:** How-To Guide

Practical recipes for personal productivity, learning, writing, and event planning.

## Manage Personal Projects

### Recipe: GTD-Style Task Management

```bash
# Create a project
jit issue create --title "Home Renovation" \
  --label "type:epic" --label "project:home" \
  --priority high

# Break down into next actions
jit issue create --title "Get contractor quotes" \
  --label "type:task" --label "project:home" \
  --label "context:phone"

jit issue create --title "Choose paint colors" \
  --label "type:task" --label "project:home" \
  --label "context:home"

# Find next actions by context
jit query available --label "context:phone"   # Calls to make
jit query available --label "context:errands" # Out and about

# Mark waiting-for items
jit issue update <id> --label "waiting:contractor-response" \
  --state backlog
```

### Weekly Review

```bash
# What's in progress?
jit query all --state in_progress

# What's stuck waiting?
jit query all --label "waiting:*"

# Projects without next actions (need breakdown)
jit query all --label "type:epic" --state ready
```

## Track Learning Goals

### Recipe: Course Progress Tracking

```bash
# Learning goal
jit issue create --title "Learn Spanish" \
  --label "type:epic" --label "skill:spanish"

# Course with completion gate
jit gate define course-complete \
  --title "Course Completed" --mode manual

jit issue create --title "Duolingo Unit 1-10" \
  --label "type:task" --label "skill:spanish" \
  --gate course-complete

# Sequential courses
jit issue create --title "Intermediate Spanish Course" \
  --label "type:task" --label "skill:spanish"
jit dep add <intermediate-id> <duolingo-id>

# Track all learning
jit query all --label "skill:*"
```

### Recipe: Certification Path

```bash
# Certification with prerequisites
jit issue create --title "AWS Solutions Architect" \
  --label "type:epic" --label "cert:aws-sa"

jit gate define exam-passed --title "Exam Passed" --mode manual

jit issue create --title "Complete AWS training" \
  --label "type:task" --label "cert:aws-sa"

jit issue create --title "Pass practice exams" \
  --label "type:task" --label "cert:aws-sa"

jit issue create --title "Schedule and pass exam" \
  --label "type:task" --label "cert:aws-sa" \
  --gate exam-passed

# Chain dependencies
jit dep add <practice-id> <training-id>
jit dep add <exam-id> <practice-id>
```

## Organize Writing Projects

### Recipe: Book Chapter Workflow

```bash
# Book project
jit issue create --title "Write: Cooking Guide" \
  --label "type:epic" --label "book:cooking"

# Gates for writing workflow
jit gate define draft-done --title "First Draft Complete" --mode manual
jit gate define edited --title "Editor Approved" --mode manual

# Chapters with sequential dependencies
jit issue create --title "Chapter 1: Kitchen Basics" \
  --label "type:task" --label "book:cooking" \
  --gate draft-done --gate edited

jit issue create --title "Chapter 2: Breakfast Recipes" \
  --label "type:task" --label "book:cooking" \
  --gate draft-done --gate edited

jit dep add <ch2-id> <ch1-id>

# Link manuscript files
jit doc add <ch1-id> manuscript/chapter-01.md --doc-type design
```

### Recipe: Article Pipeline

```bash
# Article tracking
jit issue create --title "Blog: Remote Work Tips" \
  --label "type:task" --label "content:blog" \
  --gate draft-done --gate seo-reviewed --gate published

jit gate define seo-reviewed --title "SEO Check Done" --mode manual
jit gate define published --title "Published Live" --mode manual

# Content calendar view
jit query all --label "content:blog" --state ready
```

## Plan Events

### Recipe: Conference Planning

```bash
# Event epic
jit issue create --title "Annual Team Retreat 2026" \
  --label "type:epic" --label "event:retreat" \
  --priority critical

# Approval gate
jit gate define budget-approved \
  --title "Budget Approved" --mode manual

# Tasks with dependencies
jit issue create --title "Get budget approval" \
  --label "type:task" --label "event:retreat" \
  --gate budget-approved

jit issue create --title "Book venue" \
  --label "type:task" --label "event:retreat"

jit issue create --title "Send invitations" \
  --label "type:task" --label "event:retreat"

# Dependencies: budget → venue → invitations
jit dep add <venue-id> <budget-id>
jit dep add <invitations-id> <venue-id>

# Deadline labels
jit issue update <venue-id> --label "deadline:2026-03-15"
```

### Event Checklist

```bash
# What's blocking the event?
jit graph deps <event-epic-id> --transitive

# What can be done now?
jit query available --label "event:retreat"
```

## Document Workflows

### Recipe: Report Review Cycle

```bash
# Report with approval chain
jit issue create --title "Q4 Financial Report" \
  --label "type:task" --label "doc:finance" \
  --gate draft-done --gate fact-checked --gate cfo-approved

jit gate define fact-checked --title "Facts Verified" --mode manual
jit gate define cfo-approved --title "CFO Sign-off" --mode manual

# Link the document
jit doc add <report-id> reports/q4-2026.md

# Track document history
jit doc history <report-id> reports/q4-2026.md
```

## See Also

- [Dependency Management](dependency-management.md) - Complex dependency patterns
- [Custom Gates](custom-gates.md) - Define domain-specific quality checks
- [CLI Reference](../reference/cli-commands.md) - Full command documentation
