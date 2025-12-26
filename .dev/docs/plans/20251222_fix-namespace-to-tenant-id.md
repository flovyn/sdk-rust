# Bug Fix: Replace `namespace` with `tenant_id`

## Problem

The design documents and implementation plans incorrectly use `namespace: String` in configuration types, following Temporal's model. However, the Flovyn server does not support namespaces - it uses `tenant_id: UUID` instead.

**Affected files:**
- `.dev/docs/plans/phase2-uniffi-bindings.md` - `WorkerConfig.namespace`, `ClientConfig.namespace`
- `.dev/docs/plans/phase3-kotlin-sdk.md` - `namespace = tenantId.toString()` in examples
- `.dev/docs/research/multi-language-sdk-strategy.md` - `.namespace("default")` in examples

**Not affected (already correct):**
- `sdk/` - Uses `tenant_id: Uuid` throughout
- `core/` - Uses `tenant_id` in gRPC clients

## Solution

Replace all `namespace` references with `tenant_id` in design docs and plans.

## TODO List

### Step 1: Fix Phase 2 Plan (uniffi bindings)

- [x] Update `WorkerConfig` record: `namespace: String` → `tenant_id: String`
- [x] Update `ClientConfig` record: `namespace: String` → `tenant_id: String`
- [x] Verify document is consistent

(Fixed by user)

### Step 2: Fix Phase 3 Plan (Kotlin SDK)

- [x] Update `CoreBridge.create()` example: remove `namespace` parameter
- [x] Update `WorkerConfig` usage to use `tenant_id`
- [x] Update code examples to use `tenantId` consistently
- [x] Verify document is consistent

(Fixed by user)

### Step 3: Update Research Document

- [ ] Update multi-language-sdk-strategy.md examples
- [ ] Replace `.namespace("default")` with `.tenantId(tenantId)`
- [ ] Verify document is consistent

### Step 4: Verify Design Document

- [ ] Check core-sdk-in-rust-for-multiple-language.md for namespace references
- [ ] Update if necessary

## Changes Summary

| Location | Before | After |
|----------|--------|-------|
| WorkerConfig | `namespace: String` | `tenant_id: String` |
| ClientConfig | `namespace: String` | `tenant_id: String` |
| Builder methods | `.namespace("default")` | `.tenantId(tenantId)` |
| Context | N/A (already uses tenantId) | N/A |

## Notes

- `tenant_id` is a UUID in the actual implementation
- In FFI, we use `String` representation since UUID types may not cross FFI boundary cleanly
- The Kotlin SDK should convert `UUID` to `String` when calling FFI, and vice versa
