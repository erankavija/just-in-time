# Glossary

> **Status:** Draft - Story 5bad7437  
> **Diátaxis Type:** Reference

## Core Concepts

**Issue**: A unit of work tracked by JIT

**Dependency**: A relationship where one issue blocks another

**Gate**: A quality checkpoint that must pass before completion

**State**: Current lifecycle stage of an issue

**Label**: A namespace:value tag for categorization

**Assignee**: Who is responsible for an issue

## Issue States

**Backlog**: Not yet ready to work on

**Ready**: Unblocked and available to claim

**InProgress**: Actively being worked on

**Gated**: Waiting for quality gate checks

**Done**: Successfully completed

**Rejected**: Closed without implementation

**Archived**: Long-term storage

## Gate Types

**Precheck**: Gate that must pass before work begins

**Postcheck**: Gate that must pass before completion

**Manual Gate**: Requires human judgment

**Automated Gate**: Runs a checker script

## Label Types

**Strategic**: High-level categorization (milestone, epic)

**Tactical**: Work-level categorization (type, component)

## Dependency Relationships

**Blocks**: A → B means A blocks B

**Blocked By**: B is blocked by A

**Transitive**: A blocks B, B blocks C implies A transitively blocks C

**Transitive Reduction**: Minimal set of dependencies

## Other Terms

**DAG**: Directed Acyclic Graph (no cycles)

**Short Hash**: 8-character UUID prefix for referencing issues

**Assignee Type**: Format prefix (agent:, human:, ci:)

**Event Log**: Audit trail of all state changes
