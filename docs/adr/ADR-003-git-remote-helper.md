# ADR-003: Git Remote Helper Instead Of Git Fork

## Status

Accepted for initial implementation.

## Context

Developers must keep ordinary Git workflows. Forking Git would increase
compatibility and distribution risk.

## Decision

Implement `git-remote-gitmesh` as a standard Git remote helper that talks to local
`gitmeshd`.

## PROTOCOL REQUIREMENT

GitMesh must interoperate with standard Git clients and preserve normal Git
semantics for clone, fetch, pull, push, branches, tags, and conflicts.

## IMPLEMENTATION CHOICE

Ship a remote helper and local daemon using native Git plumbing for the first
generation.

## INITIAL DEFAULT

Generation 1 uses cached bare repositories, `upload-pack`, `receive-pack`, and
pack ingest/export.

## FUTURE OPTION

Generation 2 may implement Git smart-protocol v2 directly over GitMesh object
indexes and segment retrieval for performance.
