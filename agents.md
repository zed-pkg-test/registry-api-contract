# Agent instructions

## Scope and hierarchy

- These instructions apply to the whole `zed-pkg/zed-api-server.rs` repository unless a deeper lowercase `agents.md` adds narrower rules.
- Before editing, resolve the current working directory and load every readable ancestor `agents.md` from the filesystem root to the working directory. Do not search siblings. Resolve symlinks, deduplicate resolved files, and report unreadable or cyclic instruction files.
- `.claude/CLAUDE.md`, `.gemini/GEMINI.md`, and `.openai/AGENTS.md` are pointers only. Never duplicate instructions in tool-specific files.

## Repository role

This Rust service is the Zed registry API boundary. It owns authenticated package metadata and artifact operations, registry compatibility, persistence behavior, service health, and code-first API contracts consumed by the CLI and generated clients.

## Working rules

- Treat routes, status codes, error bodies, OpenAPI schemas, authentication requirements, and pagination as public APIs.
- Reuse `zed-interfaces` models; update the contract repository before consumers when a change spans repositories.
- Keep database migrations forward-safe, explicit, and tested against realistic upgrade paths. Do not hide schema changes in startup code.
- Fail closed for publish, yank, token, organization, and administrative operations; read-only diagnostics must not expose credentials or package secrets.
- Preserve checksum, size, content-type, and immutable artifact guarantees across upload and download paths.
- Keep health/readiness and metrics endpoints bounded and free of sensitive data.
- Never commit tokens, database URLs, cloud credentials, kubeconfigs, or production environment files.
- Exercise focused unit/integration tests, OpenAPI drift checks, formatting, compilation, Clippy, and container/runtime checks relevant to the change.

## Validation

The pinned `agents policy` workflow validates this hierarchy and the three tool pointers. Follow `README.md` and existing CI for service-specific validation before requesting review.

## Repository-local Git worktrees

- Create or use a Git worktree only when the human operator explicitly authorizes it for the current task. Concurrency or a dirty checkout is not permission by itself.
- Put every authorized worktree at `<repository-root>/tmp/worktrees/<name>`; from the repository root, use `./tmp/worktrees/<name>`. Never place worktrees beside repositories or organization directories.
- Keep `tmp`, `temp`, `tmp/worktrees`, and `temp/worktrees` ignored in the repository-root `.gitignore`. Do not commit files from those directories.
- Relocate or remove a worktree only when the operator explicitly requests it. Before removal, preserve and publish intended changes, verify its commit is represented on the target branch, and confirm there are no tracked, untracked, ignored-sensitive, or in-use files that must survive. Remove it with `git worktree remove <path>` without `--force`; never delete a worktree directory with `rm`.
