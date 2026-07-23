# CI concurrency design

## Goal

Cancel obsolete CI runs when a pull request or non-main branch receives a
newer commit, while preserving every CI run triggered from `main`.

## Change

Add a workflow-level GitHub Actions `concurrency` block to
`.github/workflows/ci.yml`.

- The group key combines the workflow name and Git ref, so unrelated workflows
  and refs never cancel one another.
- `cancel-in-progress` is true for every ref except `refs/heads/main`.
- Pull-request refs are therefore cancellable, as approved.

## Boundaries

This changes only the CI workflow's scheduling behavior. It does not alter
triggers, jobs, permissions, release workflows, or test commands.

## Validation

Verify the workflow parses as YAML and assert that its concurrency group is
ref-scoped and its cancellation condition excludes only `main`.
