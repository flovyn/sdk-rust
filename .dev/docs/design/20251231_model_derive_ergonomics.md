# Design: Improving Model Derive Ergonomics

**Date:** 2025-12-31
**Status:** Draft
**Author:** Claude

## Problem Statement

Workflow and task input/output types require deriving multiple traits:

```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrderInput {
    pub order_id: String,
    pub amount: f64,
}
```

This is error-prone because:
1. **Easy to forget** - 5 traits is a lot to remember
2. **Inconsistent** - Different devs may forget different traits
3. **Poor error messages** - Forgetting `JsonSchema` gives cryptic trait bound errors
4. **Boilerplate** - Every model type needs the same derives

## Current State

The SDK requires:
- `Debug` - For logging and error messages
- `Clone` - Types passed through context must be cloneable
- `Serialize` - To serialize to JSON for gRPC transport
- `Deserialize` - To deserialize from JSON
- `JsonSchema` - To auto-generate JSON Schema for the server

## Proposed Solutions

### Option 1: Proc-Macro Derive (`#[derive(FlovynModel)]`)

Create a custom derive macro that expands to all required derives.

**Usage:**
```rust
use flovyn_sdk::FlovynModel;

#[derive(FlovynModel)]
pub struct OrderInput {
    pub order_id: String,
    pub amount: f64,
}
```

**Pros:**
- Single derive to remember
- Clear intent ("this is a Flovyn model")
- Can add custom attributes for schema customization
- Familiar pattern (similar to `#[derive(Component)]` in Bevy)

**Cons:**
- Requires a new proc-macro crate (`flovyn-sdk-derive`)
- Proc-macros increase compile times
- Can't combine with custom serde attributes easily
- Users lose visibility into what traits are derived

**Implementation complexity:** Medium (new crate, proc-macro code)

---

### Option 2: Attribute Macro (`#[flovyn_model]`)

Create an attribute macro that adds derives to the item.

**Usage:**
```rust
use flovyn_sdk::flovyn_model;

#[flovyn_model]
pub struct OrderInput {
    pub order_id: String,
    pub amount: f64,
}
```

**Pros:**
- Clean syntax
- Can preserve and extend existing derives
- Can add conditional derives based on attributes

**Cons:**
- Same proc-macro drawbacks as Option 1
- Less familiar pattern than derive macros
- Harder to understand what it does

**Implementation complexity:** Medium

---

### Option 3: Type Alias with Trait Bounds (Status Quo + Documentation)

Keep current approach but improve documentation and error messages.

**Usage:** (unchanged)
```rust
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrderInput { ... }
```

**Improvements:**
1. Add a `ModelBounds` trait alias for documentation:
   ```rust
   pub trait ModelBounds: Debug + Clone + Serialize + DeserializeOwned + JsonSchema + Send {}
   impl<T> ModelBounds for T where T: Debug + Clone + Serialize + DeserializeOwned + JsonSchema + Send {}
   ```

2. Better error messages via `#[diagnostic::on_unimplemented]` (Rust 1.78+):
   ```rust
   #[diagnostic::on_unimplemented(
       message = "Input/Output types must derive: Debug, Clone, Serialize, Deserialize, JsonSchema",
       label = "missing required derives",
       note = "Add: #[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]"
   )]
   pub trait ModelBounds: ... {}
   ```

3. IDE snippet / template in documentation

**Pros:**
- No new dependencies
- Explicit and transparent
- Works with all serde attributes
- Zero compile-time overhead

**Cons:**
- Still verbose
- Relies on documentation discipline

**Implementation complexity:** Low

---

### Option 4: Declarative Macro (`flovyn_model!{}`)

Provide a declarative macro wrapper.

**Usage:**
```rust
use flovyn_sdk::flovyn_model;

flovyn_model! {
    /// Order input for processing
    pub struct OrderInput {
        pub order_id: String,
        pub amount: f64,
    }
}

flovyn_model! {
    #[serde(rename_all = "camelCase")]
    pub struct CamelCaseInput {
        pub user_name: String,
    }
}
```

**Pros:**
- No proc-macro crate needed
- Single import
- Can support both structs and enums

**Cons:**
- Unfamiliar syntax for Rust developers
- IDE support may be limited (no struct completion inside macro)
- Harder to support all serde/schemars attributes
- Macro hygiene issues with complex types

**Implementation complexity:** Low-Medium

---

### Option 5: Supertrait Re-export Bundle

Re-export a trait bundle that users implement.

**Usage:**
```rust
use flovyn_sdk::prelude::*;

// User still writes derives, but SDK provides a marker trait
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct OrderInput { ... }

// SDK validates at registration time
impl FlovynInput for OrderInput {}
```

**Pros:**
- Explicit opt-in
- Clear compile-time validation
- No macro magic

**Cons:**
- Still requires all derives
- Extra boilerplate (impl line)
- Doesn't solve the core problem

**Implementation complexity:** Low

---

### Option 6: Serde-style Derive with Built-in Schema

Similar to how serde provides `Serialize`/`Deserialize`, create a unified derive.

**Usage:**
```rust
use flovyn_sdk::Model;

#[derive(Debug, Clone, Model)]
#[model(rename_all = "camelCase")]
pub struct OrderInput {
    #[model(description = "The order ID")]
    pub order_id: String,
    pub amount: f64,
}
```

**Pros:**
- Single derive for serialization + schema
- Custom attributes for schema metadata
- Clean, unified API

**Cons:**
- Major undertaking - essentially reimplementing serde
- Users lose serde ecosystem compatibility
- High maintenance burden

**Implementation complexity:** Very High (not recommended)

---

## Recommendation

**Short-term: Option 3 (Improved Documentation + Better Errors)**

1. Add `#[diagnostic::on_unimplemented]` for better error messages
2. Create a documentation page with copy-paste templates
3. Add IDE snippets to the README

**Medium-term: Option 1 (Proc-Macro Derive)**

If the community feedback indicates the boilerplate is a significant pain point:

1. Create `flovyn-sdk-derive` crate
2. Implement `#[derive(FlovynModel)]`
3. Support passthrough of serde/schemars attributes

## Detailed Design: Proc-Macro Derive (Option 1)

### Crate Structure

```
sdk-rust/
├── derive/           # New crate: flovyn-sdk-derive
│   ├── Cargo.toml
│   └── src/
│       └── lib.rs
├── sdk/
│   └── Cargo.toml    # Add: flovyn-sdk-derive = { path = "../derive" }
```

### Macro Implementation

```rust
// derive/src/lib.rs
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

#[proc_macro_derive(FlovynModel, attributes(flovyn))]
pub fn derive_flovyn_model(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as DeriveInput);
    let name = &input.ident;
    let generics = &input.generics;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();

    // The macro doesn't actually implement traits - it just validates
    // that the required derives are present and provides a marker
    let expanded = quote! {
        // Compile-time assertion that required traits are implemented
        const _: () = {
            fn assert_flovyn_model<T>()
            where
                T: ::std::fmt::Debug
                    + ::std::clone::Clone
                    + ::serde::Serialize
                    + ::serde::de::DeserializeOwned
                    + ::schemars::JsonSchema
                    + ::std::marker::Send
            {}

            fn assert_impl #impl_generics () #where_clause {
                assert_flovyn_model::<#name #ty_generics>();
            }
        };
    };

    TokenStream::from(expanded)
}
```

Wait - this approach still requires users to add all the derives. A better approach:

### True Derive Macro (generates all derives)

```rust
// derive/src/lib.rs
use proc_macro::TokenStream;
use quote::quote;
use syn::{parse_macro_input, DeriveInput};

/// Derives all traits required for Flovyn workflow/task models.
///
/// Equivalent to:
/// ```ignore
/// #[derive(Debug, Clone, serde::Serialize, serde::Deserialize, schemars::JsonSchema)]
/// ```
#[proc_macro_attribute]
pub fn flovyn_model(_attr: TokenStream, item: TokenStream) -> TokenStream {
    let input = parse_macro_input!(item as DeriveInput);

    let expanded = quote! {
        #[derive(
            ::std::fmt::Debug,
            ::std::clone::Clone,
            ::serde::Serialize,
            ::serde::Deserialize,
            ::schemars::JsonSchema
        )]
        #input
    };

    TokenStream::from(expanded)
}
```

### Usage

```rust
use flovyn_sdk::flovyn_model;

#[flovyn_model]
pub struct OrderInput {
    pub order_id: String,
    pub amount: f64,
}

// With serde attributes (passthrough)
#[flovyn_model]
#[serde(rename_all = "camelCase")]
pub struct CamelInput {
    pub user_name: String,
}
```

### Supporting Additional Derives

Users may want to add `PartialEq`, `Eq`, `Hash`, etc:

```rust
#[flovyn_model]
#[derive(PartialEq, Eq, Hash)]  // Additional derives work normally
pub struct OrderId(String);
```

## Migration Path

1. **v0.2.0**: Add `#[flovyn_model]` attribute macro, keep current approach working
2. **v0.3.0**: Deprecation warnings for raw derives (optional)
3. **v1.0.0**: Recommend `#[flovyn_model]` in all documentation

## Open Questions

1. Should `Send + Sync` be required? (Currently only `Send`)
2. Should we support `#[flovyn_model(skip_schema)]` for internal types?
3. How to handle generic types with bounds?
4. Should enums get different treatment than structs?

## Alternatives Considered

- **Blanket implementations** - Not possible due to orphan rules
- **Build script generation** - Too magical, hard to debug
- **Runtime reflection** - Rust doesn't support this well

## References

- [serde_derive](https://docs.rs/serde_derive) - Inspiration for derive approach
- [bevy_reflect](https://docs.rs/bevy_reflect) - Similar problem in game dev
- [typetag](https://docs.rs/typetag) - Combines multiple derives elegantly
- [Rust diagnostic attributes](https://doc.rust-lang.org/reference/attributes/diagnostics.html)
