# Working on ProjectScone Rust

Create a feature branch, preserve unrelated changes and stage explicit paths.
The complete native source tree is in `crates/`. Root `tests/fixtures/` contains
five HTTP/prompt/profile/source/space contract fixtures used with `include_str!`;
episode fixtures remain inside `crates/scone-core/tests/fixtures/`. Keep these
paths local and intact when moving the repository.

## Native checks

From the repository root, with a stable Rust toolchain and native C/C++ compiler:

```sh
cargo fmt --all -- --check
cargo clippy --locked --workspace --all-targets --no-default-features -- -D warnings
cargo test --locked --workspace --no-default-features
cargo clippy --locked --workspace --all-targets -- -D warnings
cargo test --locked --workspace
```

These checks require neither Python nor Node nor another checkout. Add `--offline`
to Cargo commands when dependency crates are cached. ONNX runtime build scripts
may need their own cached native libraries even in Cargo offline mode. Default
temporal-operator and MCP arithmetic tests need the embedding model in `~/.scone`
and download it if missing. Use the minimal profile for model-free tests. Other
model-download and long-running probes are ignored; run those only when
intentionally provisioning the required models and measuring their results.
CI validates minimal and default features separately and builds the deterministic
external conformance probe. Compilation/Clippy supplies native type checking.

`node --test scripts/test-prompt-hook-config.cjs` additionally verifies the
optional hook installer without changing host settings. The optional browser
suite is `node --test scripts/test-rust-console.cjs`, with a separately installed
Playwright module supplied through `SCONE_PLAYWRIGHT_MODULE` or `NODE_PATH`, and
an installed browser selected with `SCONE_BROWSER_PATH`. These JavaScript checks
are not prerequisites for native builds. The optional mutation helper
`scripts/prove-test.sh` requires zsh and Python 3; normal Rust checks do not use it.

## External Python conformance

Build the test-only deterministic JSONL exchange binary here:

```sh
cargo build --locked -p scone-core --no-default-features --example episode_roundtrip
cargo test --locked -p scone-core --no-default-features --test shared_contract --test portability
```

In an independently installed Python framework test environment, run its
`packages/memory/tests/test_cross_language.py` suite with an explicit absolute
binary path (adjust the checkout locations to your machine):

```sh
cd /path/to/ProjectScone/packages/memory
SCONE_TEST_RUST_ROUNDTRIP=/path/to/ProjectScone-Rust/target/debug/examples/episode_roundtrip \
  .venv/bin/python -m pytest -q tests/test_cross_language.py
```

The Python repository owns its fixture copies and its conformance runner; Rust
CI does not install Python or infer a sibling path. Recheck fixture byte identity
when changing contracts in either repository. The probe is a limited episode
profile with identity verification, not a migration tool or proof of complete
fact/history portability. If `CARGO_TARGET_DIR` is set, use that output directory
in the explicit binary path instead.

## Updating embedded pages

`crates/scone/src/console.html` is the native Rust console;
`crates/scone/src/playground.html` is the retained shared playground snapshot.
Cargo embeds these files directly. Ordinary Rust builds and releases must never
invoke a frontend build or overwrite either file.

To propose a playground update, use an explicit ProjectScone-Webapp checkout:

1. Install its dependencies with `pnpm install --frozen-lockfile` and run its
   tests and type checks.
2. Use its explicit packaging output option to build a candidate into a temporary
   directory. Check that the candidate is the playground intended for the Rust
   HTTP routes; the Python-oriented React Memory page is not an interchangeable
   replacement.
3. Review the candidate diff, authentication handling, API requests and compatibility
   with the Rust routes. Preserve any uncommitted destination edits before an
   explicitly authorized update; do not let packaging select Rust files implicitly.
4. Only after compatibility is verified, copy the selected candidate to the one
   intended embedded asset, run the Rust HTTP tests and affected browser contracts,
   and commit the reviewed snapshot as an explicit asset update.

The candidate packaging command is explicit about the checkout and output:

```sh
cd /path/to/ProjectScone-Webapp
pnpm build
node scripts/package.mjs --output /absolute/path/to/temp/playground.html
```

This produces a review candidate only. It does not update either Rust asset.
The native console can be edited and validated locally without the Webapp.
Never replace either embedded page merely to make a build output comparison pass.

## Data and measurements

Keep credentials, databases, model caches, dependency trees, benchmark output
and private research out of commits. Use disposable local stores for tests.
Record dataset/model versions, feature flags, retrieval quality, latency,
resource use and failures; report Rust and Python measurements separately.
Keep the inherited LICENSE and CITATION.cff terms intact.
