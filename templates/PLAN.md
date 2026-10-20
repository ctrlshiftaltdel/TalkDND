# Implementation Plan

> This plan is the source for task registration. Each Task block below must translate field by field into one `task add` command; split any block that fails the granularity check instead of registering it.

## Plan header

- **Plan name:**
- **Source charter / design doc:**
- **Overall goal:**

## Task blocks

<!-- Repeat the block below for every task in the plan. Keep blocks in dependency order. -->

### Task: <Task ID>

- **Task ID:**
- **Goal:**
<!-- Exactly one independently acceptable goal. If the block holds more than one acceptable goal, split it before registration. -->
- **Allowed paths:**
<!-- Precise down to concrete files or directories; must match the --paths lock exactly. -->
- **Validation:**
<!-- Verification commands with their expected output. -->
  - Command:
  - Expected output:
- **Acceptance criteria:**
<!-- Every criterion must be individually checkable. -->
  - [ ]
- **Depends on:**
<!-- Task IDs that must be VERIFIED before this task can start. Write "none" if there are no dependencies. -->
