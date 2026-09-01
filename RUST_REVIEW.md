# Rust Best Practices Review - orbit-tui

This review evaluates the orbit codebase against Apollo GraphQL's Rust Best Practices Handbook.

## Summary

The codebase is well-structured with 309 passing tests and clean clippy output. However, several areas can be improved following Rust best practices.

---

## 1. Borrowing & Ownership (Chapter 1)

### Issues Found

#### Unnecessary `.clone()` on Copy types
**File:** `src/app.rs:236-239`
```rust
let keys: Vec<String> = items
    .iter()
    .map(|item| extract_json_value(item, sort_path))
    .collect();
```
- `extract_json_value` returns `String` which is `Clone` but the iteration borrows - acceptable

**File:** `src/resource/fetcher.rs:78-87` (and similar)
```rust
filter
    .values
    .iter()
    .map(|v| Value::String(v.clone()))  // Could use .cloned() on iterator
    .collect(),
```
**Fix:** Use `.cloned()` instead of `.map(|v| v.clone())` for Copy/Clone types:
```rust
filter.values.iter().cloned().map(Value::String).collect()
```

#### `.clone()` where borrow would suffice
**File:** `src/app.rs:244`
```rust
let sorted: Vec<Value> = order.iter().map(|&i| items[i].clone()).collect();
```
**Context:** Needed because `items.clone_from_slice(&sorted)` requires ownership - acceptable

**File:** `src/resource/registry.rs:68` (in `get_resource`)
```rust
let resource_def = get_resource(resource_key).ok_or_else(...)?;
```
Returns `&ResourceDef` but later code clones the whole def - review if needed

#### `Arc`/`Rc` usage patterns
**File:** `src/aws/client.rs:34` - `AwsClients` derives `Clone`
```rust
#[derive(Clone)]
pub struct AwsClients {
    pub http: AwsHttpClient,
    pub region: String,
    pub profile: String,
}
```
`AwsHttpClient` contains `reqwest::Client` which is already `Arc`-based internally - this is fine

**File:** `src/resource/registry.rs:9` - Uses `OnceLock` for static registry - good pattern

---

## 2. Error Handling (Chapter 4)

### Good Practices ✅
- Uses `anyhow::Result` consistently (binary crate - appropriate per guidelines)
- Uses `?` operator for error propagation throughout
- Custom error types via `thiserror` in some modules
- `ClientResult` enum for explicit client creation outcomes (good type-safe approach)

### Issues Found

#### `unwrap()` / `expect()` in production code
**File:** `src/resource/path_extractor.rs:62`
```rust
results.into_iter().next().unwrap()  // After checking len == 1
```
**Fix:** Use `let Some(v) = results.into_iter().next() else { unreachable!() };`

**File:** `src/ui/mod.rs:247` (and similar)
```rust
.unwrap_or(0)  // on Option from .position()
```
**Fix:** Acceptable - has clear fallback

**File:** `src/app.rs:1071` (example)
```rust
let parent = self.parent_context.as_ref().unwrap();
```
**Fix:** Should use `if let Some(parent) = ...` or handle None case

#### `anyhow::Result` in library-like code
The `src/resource/` modules feel like library code but use `anyhow`. Consider:
- `thiserror` for `resource/registry.rs`, `resource/protocol.rs`, `resource/field_mapper.rs`
- Reserve `anyhow` for `src/app.rs`, `src/main.rs`, `src/aws/`

---

## 3. Performance Mindset (Chapter 3)

### Good Practices ✅
- Uses iterators extensively (`.iter()`, `.map()`, `.filter()`)
- Lazy evaluation leveraged well
- `OnceLock` for static registry avoids repeated parsing

### Issues Found

#### Intermediate collections
**File:** `src/app.rs:236-245` (`sort_items`)
```rust
let keys: Vec<String> = items.iter().map(...).collect();  // Allocation 1
let mut order: Vec<usize> = (0..items.len()).collect();  // Allocation 2
order.sort_by(...);
let sorted: Vec<Value> = order.iter().map(|&i| items[i].clone()).collect();  // Allocation 3
items.clone_from_slice(&sorted);
```
**Issue:** 3 allocations per sort. Could use Schwartzian transform more efficiently.

**Fix:** Extract keys once with `enumerate()`:
```rust
let mut indices: Vec<_> = items.iter().enumerate().collect();
indices.sort_by(|(_, a), (_, b)| compare_cells(...));
items.sort_by_cached_key...  // or reorder in-place
```

#### String allocation in hot paths
**File:** `src/resource/handlers/query.rs:133-149`
```rust
let mapped_key = config.param_mapping.get(key).cloned().unwrap_or_else(|| key.clone());
```
**Fix:** Use `Cow<str>` to avoid allocation when no mapping:
```rust
let mapped_key = config.param_mapping.get(key).map_or(Cow::Borrowed(key), Cow::Owned);
```

#### `.collect()` where iterator would suffice
**File:** `src/resource/field_mapper.rs:46-52`
```rust
match value {
    Value::String(_) => value,
    Value::Number(n) => Value::String(n.to_string()),  // allocates
    ...
}
```
**Fix:** Acceptable - conversion is necessary

---

## 4. Linting (Chapter 2)

### Status ✅
- `cargo clippy --all-targets --all-features --locked -- -D warnings` - **PASSES**
- `cargo fmt --check` - **PASSES**

### Clippy Configuration
No workspace-level clippy config in `Cargo.toml`. Consider adding:
```toml
[workspace.lints.clippy]
all = { level = "deny", priority = 10 }
redundant_clone = { level = "deny", priority = 9 }
needless_collect = { level = "deny", priority = 8 }
```

---

## 5. Testing (Chapter 5)

### Good Practices ✅
- 309 tests passing
- Tests organized in `#[cfg(test)] mod tests` blocks
- Descriptive test names: `cloudwatch_latest_picks_the_newest_datapoint_not_the_first`
- Tests cover edge cases (empty arrays, null values, different timestamp formats)

### Issues Found

#### Multiple assertions per test
**File:** `src/app.rs:2604-2617`
```rust
#[test]
fn test_parse_aws_filters_valid() {
    let result = AwsFilters::parse("Filters: owner=amazon, architecture=arm64");
    assert!(result.is_some());
    let filters = result.unwrap();
    assert_eq!(filters.filters.len(), 2);
    assert_eq!(filters.filters[0], ("owner".to_string(), "amazon".to_string()));
    assert_eq!(filters.filters[1], ("architecture".to_string(), "arm64".to_string()));
}
```
**Fix:** Split into multiple tests or use single assertion with struct comparison

#### Test module naming inconsistency
Some use `mod tests`, others use `mod test` - standardize on plural `tests`

---

## 6. Generics & Dispatch (Chapter 6)

### Good Practices ✅
- Protocol handlers use trait objects appropriately: `dyn ProtocolHandler`
- Static dispatch used where types are known: `JsonProtocolHandler`, `QueryProtocolHandler`
- `ProtocolHandler` trait is object-safe (no generic methods, no `Self` returns)

### Issues Found

#### Boxing at API boundary but not internally
**File:** `src/resource/handlers/mod.rs:22`
```rust
pub fn get_protocol_handler(protocol: ApiProtocol) -> Box<dyn ProtocolHandler>
```
**Fix:** Consider returning `&'static dyn ProtocolHandler` since handlers are stateless singletons

---

## 7. Type State Pattern (Chapter 7)

### Opportunity Missed

The `App` struct has many `Option` fields representing different modes:
```rust
pub sso_state: Option<SsoLoginState>,
pub console_login_state: Option<ConsoleLoginState>,
pub log_tail_state: Option<LogTailState>,
pub reveal_secret: Option<SecretReveal>,
pub describe_data: Option<Value>,
```
**Suggestion:** Could use Type State Pattern to encode valid state combinations at compile time:
```rust
struct App<State> { ... }
struct NormalMode;
struct SsoLoginMode;
struct LogTailMode;
```
Would prevent invalid states (e.g., having both `sso_state` and `log_tail_state` as `Some`).

---

## 8. Comments & Documentation (Chapter 8)

### Good Practices ✅
- Good module-level docs: `//!` comments explaining purpose
- Inline comments explain AWS API quirks: "CloudWatch makes no promise about datapoint order"
- `SAFETY` comments where needed

### Issues Found

#### Missing doc comments on public APIs
**File:** `src/resource/registry.rs` - Public structs `ResourceDef`, `ColumnDef`, `ActionDef` lack `///` docs
**File:** `src/resource/protocol.rs` - Public types `ApiConfig`, `PaginationConfig` lack docs
**File:** `src/aws/http.rs` - Public `ServiceDefinition`, `get_service` lack docs

#### `missing_docs` not enforced
Add to `Cargo.toml`:
```toml
[workspace.lints.rust]
missing_docs = "warn"
```

#### TODO comments without issue links
**File:** `src/app.rs:44` - `#[allow(dead_code)]` on `default_no` field - should reference issue
**File:** `src/resource/registry.rs:173-174` - `#[allow(dead_code)]` on `id_param` and `key` fields

---

## 9. Pointers & Thread Safety (Chapter 9)

### Good Practices ✅
- `Arc` used for shared state across threads (e.g., `reqwest::Client`)
- `Mutex`/`RwLock` not heavily used - good (simpler architecture)
- `OnceLock` for static registry initialization

### Issues Found

#### `Arc<Mutex<>>` pattern could be `RwLock`
Not heavily used in codebase, but where present consider `RwLock` for read-heavy workloads

---

## 10. Code Structure & Organization

### Large `app.rs` (3219 lines)
Consider splitting:
- `app/state.rs` - App struct and core state
- `app/navigation.rs` - Navigation logic
- `app/actions.rs` - Action handling
- `app/ui_state.rs` - UI-specific state

### Dead Code
Multiple `#[allow(dead_code)]` attributes indicate fields kept for JSON compatibility but unused in Rust. Consider:
- Removing if truly unused
- Documenting why they're kept

---

## Priority Fixes

### High Priority
1. **Add workspace clippy config** - Enforce lints consistently
2. **Replace `unwrap()` in production paths** - Use proper error handling
3. **Add `missing_docs` lint** - Improve public API documentation
4. **Fix redundant `.clone()` in iterator chains** - Use `.cloned()`/`.copied()`

### Medium Priority
5. **Optimize `sort_items` allocations** - Reduce from 3 to 1-2 allocations
6. **Use `Cow<str>` for param mapping** - Avoid allocations in hot path
7. **Split `app.rs`** - Improve maintainability
8. **Use `thiserror` for resource modules** - Better error types for library code

### Low Priority
9. **Type State Pattern for App modes** - Compile-time state safety
10. **Standardize test module naming** - `mod tests` consistently

---

## Verification Commands

```bash
# Current status - all pass
cargo test --quiet
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo fmt --check

# After fixes, run:
cargo test --quiet
cargo clippy --all-targets --all-features --locked -- -D warnings
cargo fmt --check
```