# CI Concurrency Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Cancel superseded CI runs for pull requests and non-main refs while
allowing `main` CI runs to continue.

**Architecture:** Add workflow-level GitHub Actions concurrency to the existing
CI workflow. The group is unique per workflow and Git ref, and cancellation is
enabled for every ref other than `refs/heads/main`.

**Tech Stack:** GitHub Actions YAML; Ruby standard-library YAML parser for a
local contract check.

## Global Constraints

- Modify only `.github/workflows/ci.yml`; do not change triggers, jobs,
  permissions, dependencies, or release workflows.
- Preserve every `main` CI run by excluding `refs/heads/main` from
  in-progress cancellation.
- Treat pull-request refs as cancellable.

---

### Task 1: Configure CI run concurrency

**Files:**
- Modify: `.github/workflows/ci.yml:1-10`
- Test: inline Ruby contract check (no repository test file needed for this
  configuration-only change)

**Interfaces:**
- Consumes: GitHub Actions `github.workflow` and `github.ref` contexts.
- Produces: a workflow-level `concurrency` mapping with `group` and
  `cancel-in-progress` keys.

- [ ] **Step 1: Run the failing workflow contract check**

```bash
ruby -ryaml -e 'ci = YAML.load_file(".github/workflows/ci.yml"); c = ci.fetch("concurrency"); abort "unexpected group" unless c["group"] == "${{ github.workflow }}-${{ github.ref }}"; abort "unexpected cancellation condition" unless c["cancel-in-progress"] == "${{ github.ref != '\''refs/heads/main'\'' }}"'
```

Expected: FAIL with `key not found: "concurrency"` because the workflow does
not yet define run concurrency.

- [ ] **Step 2: Add the minimal workflow-level concurrency mapping**

Insert this block after the `on` trigger block and before `env`:

```yaml
concurrency:
  group: ${{ github.workflow }}-${{ github.ref }}
  cancel-in-progress: ${{ github.ref != 'refs/heads/main' }}
```

- [ ] **Step 3: Re-run the workflow contract check**

```bash
ruby -ryaml -e 'ci = YAML.load_file(".github/workflows/ci.yml"); c = ci.fetch("concurrency"); abort "unexpected group" unless c["group"] == "${{ github.workflow }}-${{ github.ref }}"; abort "unexpected cancellation condition" unless c["cancel-in-progress"] == "${{ github.ref != '\''refs/heads/main'\'' }}"'
```

Expected: exit status 0 with no output.

- [ ] **Step 4: Check the final diff for whitespace errors**

```bash
git diff --check
```

Expected: exit status 0 with no output.

- [ ] **Step 5: Commit the workflow update**

```bash
git add .github/workflows/ci.yml
git commit -m "ci: cancel superseded branch runs"
```
