# Bug Fix: Replace `namespace` with `org_id`

## Problem

The design documents and implementation plans incorrectly use `namespace: String` in configuration types, following Temporal's model. However, the Flovyn server does not support namespaces - it uses `org_id: UUID` instead.

**Affected files:**
- `.dev/docs/plans/phase2-uniffi-bindings.md` - `WorkerConfig.namespace`, `ClientConfig.namespace`
- `.dev/docs/plans/phase3-kotlin-sdk.md` - `namespace = orgId.toString()` in examples
- `.dev/docs/research/multi-language-sdk-strategy.md` - `.namespace("default")` in examples

**Not affected (already correct):**
- `sdk/` - Uses `org_id: Uuid` throughout
- `core/` - Uses `org_id` in gRPC clients

## Solution

Replace all `namespace` references with `org_id` in design docs and plans.

## TODO List

### Step 1: Fix Phase 2 Plan (uniffi bindings)

- [x] Update `WorkerConfig` record: `namespace: String` → `org_id: String`
- [x] Update `ClientConfig` record: `namespace: String` → `org_id: String`
- [x] Verify document is consistent

(Fixed by user)

### Step 2: Fix Phase 3 Plan (Kotlin SDK)

- [x] Update `CoreBridge.create()` example: remove `namespace` parameter
- [x] Update `WorkerConfig` usage to use `org_id`
- [x] Update code examples to use `orgId` consistently
- [x] Verify document is consistent

(Fixed by user)

### Step 3: Update Research Document

- [ ] Update multi-language-sdk-strategy.md examples
- [ ] Replace `.namespace("default")` with `.orgId(orgId)`
- [ ] Verify document is consistent

### Step 4: Verify Design Document

- [ ] Check core-sdk-in-rust-for-multiple-language.md for namespace references
- [ ] Update if necessary

## Changes Summary

| Location | Before | After |
|----------|--------|-------|
| WorkerConfig | `namespace: String` | `org_id: String` |
| ClientConfig | `namespace: String` | `org_id: String` |
| Builder methods | `.namespace("default")` | `.orgId(orgId)` |
| Context | N/A (already uses orgId) | N/A |

## Notes

- `org_id` is a UUID in the actual implementation
- In FFI, we use `String` representation since UUID types may not cross FFI boundary cleanly
- The Kotlin SDK should convert `UUID` to `String` when calling FFI, and vice versa
