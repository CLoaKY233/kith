# Engineering rules

These hold for every crate. CI enforces most of them; review enforces the rest.

## Correctness

- **No panics in library code.**
  - `unwrap`, `panic!`, `todo!`, `unimplemented!` and slice indexing are lint errors.
  - Use `?`, `get()`, `checked_*` and `split_at_checked`.
  - Tests may use `unwrap`.
- **`expect`** is allowed only for invariants that are truly unreachable, and the message must state the invariant.
  - Use `#[expect(lint, reason = "...")]`, never `#[allow]`, and keep the scope as narrow as possible.
- **No `unsafe`** outside `kith-sys`. Every crate root has `#![forbid(unsafe_code)]`.
- **Invalid states are unrepresentable.**
  - Validate in constructors.
  - Use `#[serde(try_from = ...)]`, or a hand-written `Deserialize`, so deserialization re-validates.
  - Keep fields private unless the type is plain data with no invariants.
- **Signed bytes are kept, never re-encoded.** Verify signatures over the exact bytes received.
- **Determinism.** Anything replicated must produce identical results on every device, regardless of insertion order.
  - Use `BTreeMap`/`BTreeSet` and sorted `Vec`s, never `HashMap` iteration order.
  - Read no wall clock inside cores: time is an input.
- **Bounded inputs.** Every collection or string that arrives from the network has a documented maximum, which is checked before allocating.

## Structure

- **Sans-IO cores.** Core crates are `#![no_std]` + `alloc`: no sockets, no threads, no clock, no async.
  - Drivers wrap them.
  - CI builds every core crate for `thumbv7em-none-eabihf`.
- **Minimal public surface.**
  - Expose only what callers need.
  - Modules are private; the crate root re-exports the few items that make up the API.
  - Prefer `pub(crate)`. Fields stay private unless the type is plain data.
- **Split only where it pays.** A crate exists for a real boundary: a plane, sans-IO core vs driver, a feature-gated transport, or the one `unsafe` crate. Otherwise, use a module.
- **One crate, one job.** Depend only on lower layers. External crates are declared once in the root `Cargo.toml` with default features off.
- **Cryptography only through `kith-crypto`.** Every signature and hash takes a `Context`. Each new context string is unique, follows the convention, and is listed in the crate that owns it.
- **Errors:**
  - Library crates use `thiserror`, with one error enum per concern.
  - Mark error enums `#[non_exhaustive]` when they may grow.
  - `anyhow` is allowed only in `kith-cli`.

## Commits

- **Messages read like a person wrote them:** a short, plain subject in the imperative ("Add STUN codec and prober"), and if needed a few sentences of body saying what and why. No bullet walls, no buzzwords.
- **No attribution trailers.** No `Co-Authored-By` lines.

## Style

- **Comments are minimal.** A clean codebase is worth more than narration.
  - Public items get one short, factual doc line (`missing_docs` requires it), plus any invariant.
  - Module docs stay brief, and only explain design decisions.
  - Inline comments only for a non-obvious *why*; never restate the code. When a spec says it, cite the section.
- **Names are full words**, so the code reads without comments.
- **Secrets:** no `Clone`, `Debug` prints `<redacted>`, and memory is zeroized on drop.

## Testing

- **Fewer tests, each high quality.** A test exists only if it would catch a real bug that no other test catches.
  - No trivial, redundant or count-padding tests.
  - Prefer one strong property test over many examples.
- **Each test's name** states the behaviour it protects.
- Every parser has:
  - a golden-bytes test, because changing those bytes is a protocol break;
  - a "never panics on arbitrary input" property test.
- **Replicated logic has convergence property tests:** the same operations inserted in any order produce identical state.
- **Commands that must pass:**
  - `cargo clippy -p <crate> --all-targets --all-features -- -D warnings`
  - `cargo nextest run -p <crate> --all-features`
  - `cargo build -p <crate> --target thumbv7em-none-eabihf` (core crates)
  - `cargo xtask ci` (everything)
