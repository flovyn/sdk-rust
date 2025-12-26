# UniFFI Ruby Bindings: Maintenance Considerations

## Overview

This document captures research on UniFFI's Ruby binding support, its current maintenance status, and what would be involved in taking over maintenance of the Ruby bindings.

## Current Status

UniFFI's Ruby support is officially in **maintenance/legacy mode**:

> "We also have partial legacy support for Ruby; the UniFFI team keeps the existing Ruby support working but tends to not add new features to that language."

| Aspect | Status |
|--------|--------|
| Existing features | Maintained, kept working |
| New UniFFI features | Generally not added to Ruby |
| Bug fixes | Still accepted |
| Community contributions | Welcomed |
| Future | May be split into separate crate |

**Fully supported languages**: Kotlin, Swift, Python
**Legacy/maintenance**: Ruby
**Third-party**: C#, Go, C++

## Architecture

### How UniFFI Works

```
┌─────────────────┐     ┌─────────────────┐     ┌─────────────────┐
│   Ruby Code     │────▶│  Ruby FFI Gem   │────▶│  Rust cdylib    │
│  (generated)    │     │ (loads .dylib)  │     │  (scaffolding)  │
└─────────────────┘     └─────────────────┘     └─────────────────┘
         │                                               │
         └──────── Lifting/Lowering via RustBuffer ──────┘
```

UniFFI transfers data between Rust and foreign languages through a C-style FFI layer:

- **Lowering**: Converts language-specific types into primitive FFI-compatible formats before transmission
- **Lifting**: Converts primitives back into native types on the receiving side

Simple types (integers) use direct casting. Complex types (strings, records, sequences) serialize to a `RustBuffer` byte buffer.

### Serialization Format

All complex types use big-endian serialization:

| Type | Format |
|------|--------|
| Integers | 1/2/4/8 bytes |
| Strings | 4-byte length prefix + UTF-8 bytes |
| Collections | 4-byte count + serialized items |
| Optionals | 1-byte boolean flag + conditional data |

Length fields use signed integers for JVM compatibility.

## Ruby Bindings Code Structure

Location: `uniffi_bindgen/src/bindings/ruby/`

### Directory Layout

```
uniffi_bindgen/src/bindings/ruby/
├── mod.rs              # Main module, entry point
├── gen_ruby/           # Ruby code generation logic
├── templates/          # Askama templates (.rb files)
└── test.rs             # Tests for Ruby generation
```

### Template Files

| Template | Purpose |
|----------|---------|
| `wrapper.rb` | Main wrapper module |
| `ObjectTemplate.rb` | Class wrappers for Rust objects |
| `RecordTemplate.rb` | Structs/data classes |
| `EnumTemplate.rb` | Enum definitions |
| `ErrorTemplate.rb` | Exception classes mapping Rust errors |
| `TopLevelFunctionTemplate.rb` | Free function wrappers |
| `RustBufferTemplate.rb` | Core buffer handling |
| `RustBufferBuilder.rb` | Serialization (Ruby → Rust) |
| `RustBufferStream.rb` | Deserialization (Rust → Ruby) |
| `Helpers.rb` | Utility functions |
| `macros.rb` | Reusable template macros |
| `NamespaceLibraryTemplate.rb` | Library namespace wrapper |

## Feature Support Matrix

| Feature | Kotlin/Swift/Python | Ruby |
|---------|---------------------|------|
| Basic types (integers, floats, bool) | ✅ | ✅ |
| Strings | ✅ | ✅ |
| Records/Structs | ✅ | ✅ |
| Enums (flat and with data) | ✅ | ✅ |
| Objects (classes with methods) | ✅ | ✅ |
| Errors/Exceptions | ✅ | ✅ |
| Timestamp/Duration | ✅ | ✅ |
| **Async/Futures** | ✅ | ❌ |
| Callbacks | ✅ | ⚠️ Partial |
| Foreign traits | ✅ | ❓ Unknown |

### Async Gap

Async is a significant missing feature. UniFFI's async implementation:

1. Wraps Rust `Future`s into `RustFuture` structs
2. Generates scaffolding functions:
   - Function returning `RustFuture` handle
   - `rust_future_poll` to check progress
   - `rust_future_complete` to retrieve results
   - `rust_future_free` for cleanup
3. Foreign code drives completion by polling

The async documentation covers Kotlin, Swift, Python, and WASM - **Ruby is not mentioned**, indicating no async support.

## Dependencies

Ruby bindings require the [ffi gem](https://github.com/ffi/ffi):

```bash
gem install ffi
```

This gem works on:
- CRuby (MRI)
- JRuby
- Rubinius

## Maintenance Effort Estimation

### Low Effort
- Bug fixes in existing templates
- Keeping up with `ComponentInterface` API changes
- Minor type support additions
- Documentation improvements

### Medium Effort
- Adding new builtin type support
- Improving error messages and debugging
- Template refactoring for better code generation
- Performance optimizations

### High Effort
- **Async support** - Would require:
  - Understanding Ruby's async models (Fibers, Ractors, async gem)
  - Implementing `RustFuture` polling from Ruby
  - Handling thread safety with Ruby's GVL (Global VM Lock)
  - Testing across Ruby implementations
- Full callback interface support
- Complex generic type support
- Maintaining parity with primary languages

## Testing Requirements

### Local Testing

```bash
# Install Ruby FFI gem
gem install ffi

# Run Ruby-specific tests
cargo test --features uniffi/fixtures-ruby

# Or use Docker for full test suite
./docker/cargo-docker.sh test
```

### CI Requirements

Full test suite requires:
- Kotlin: `kotlinc` compiler + `ktlint`
- Swift: Swift CLI tools + Foundation
- Python: `python3` interpreter
- Ruby: Ruby interpreter + FFI gem

## Alternatives to Maintaining UniFFI Ruby

### Option 1: Fork and Extend

Fork `uniffi-rs` and add async support specifically for Ruby. Maintain as separate project.

**Pros**: Full control, can add needed features
**Cons**: Maintenance burden, divergence from upstream

### Option 2: Blocking Wrappers

Keep using UniFFI for sync APIs. Expose synchronous wrappers that internally block on async operations.

```rust
// Internal async
async fn execute_workflow_async(&self) -> Result<Output> { ... }

// Exposed to Ruby via UniFFI
fn execute_workflow(&self) -> Result<Output> {
    runtime.block_on(self.execute_workflow_async())
}
```

**Pros**: Works with current UniFFI Ruby support
**Cons**: Loses async benefits, potential deadlocks

### Option 3: Hybrid Approach

Use UniFFI for core sync functionality. Add manual Ruby extension layer for async using:
- [Magnus](https://github.com/matsadler/magnus) - Native Ruby extensions in Rust
- [Rutie](https://github.com/danielpclark/rutie) - Alternative Rust-Ruby bridge

**Pros**: Best of both worlds
**Cons**: More complex build, two FFI layers

### Option 4: Pure C FFI

Skip UniFFI for Ruby entirely. Export C-compatible interface, use Ruby's FFI gem directly.

**Pros**: Full control, no UniFFI dependency for Ruby
**Cons**: Manual work, no code generation

## Recommendations for Flovyn SDK

Given our SDK's async-heavy nature (workflows, tasks, streaming):

1. **Primary targets** (Kotlin, Swift, Python): Continue with UniFFI - excellent support
2. **Ruby**: Consider one of:
   - **Defer** until community adds async support
   - **Blocking wrappers** if sync API is acceptable for Ruby users
   - **Manual extension** using Magnus if Ruby is high priority

If taking over maintenance:
- Start by understanding the template system
- Run full test suite to establish baseline
- Focus on async as the highest-impact feature
- Engage with Mozilla team about potential separation into own crate

## References

- [UniFFI User Guide](https://mozilla.github.io/uniffi-rs/latest/)
- [UniFFI GitHub Repository](https://github.com/mozilla/uniffi-rs)
- [Contributing Guide](https://github.com/mozilla/uniffi-rs/blob/main/docs/contributing.md)
- [Lifting & Lowering Internals](https://mozilla.github.io/uniffi-rs/latest/internals/lifting_and_lowering.html)
- [Async Overview](https://mozilla.github.io/uniffi-rs/latest/internals/async-overview.html)
- [Ruby FFI Gem](https://github.com/ffi/ffi)
- [Magnus (Rust Ruby Extensions)](https://github.com/matsadler/magnus)
