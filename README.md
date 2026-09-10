# ProjectScone Rust

ProjectScone · A JudgeHuman project by Mark Sturman

[Source repository](https://github.com/ProjectScone/ProjectScone-Rust) ·
[Releases](https://github.com/ProjectScone/ProjectScone-Rust/releases)

The native Rust temporal memory engine, CLI, HTTP/MCP server, C ABI and benchmark
harness. This Cargo workspace builds independently. The Python framework lives
in the separate [ProjectScone repository](https://github.com/ProjectScone/ProjectScone);
the React frontend lives in [ProjectScone-Webapp](https://github.com/ProjectScone/ProjectScone-Webapp).
Neither checkout is required for Rust builds.

## Build and run

Install a stable Rust toolchain with support for edition 2024 and a native C/C++
compiler, then run from this checkout:

```sh
cargo build --locked -p scone-cli --no-default-features
./target/debug/scone --embedder hash --llm none add --note "hello, memory"
./target/debug/scone --embedder hash --llm none search "hello"
```

The deterministic hash embedder is useful for development and conformance; it
is not a semantic embedding model. These commands use the default local data
directory (`~/.scone`); choose `--data-dir /path/to/local/data` and `--space NAME`
to keep independent stores and scopes.

Default features add PDF text extraction and local ONNX embeddings:

```sh
cargo build --locked -p scone-cli
./target/debug/scone add --note "changed the oil on the truck"
./target/debug/scone search "vehicle maintenance"
```

The local embedder downloads a model on first use, then reuses cached files.
Default-feature compilation can also obtain ONNX runtime binaries. Use the
minimal build above when model provisioning is unwanted. PDF text extraction
is not OCR. Configure any LLM or external connector explicitly; basic ingestion
and retrieval do not require an LLM service.

## Workspace

| Package | Purpose |
| --- | --- |
| `scone-core` | SQLite-backed episodes and temporal facts, indexing and recall |
| `scone-cli` | `scone` binary, HTTP server, MCP, hooks and embedded console |
| `scone-ffi` | C ABI subset; declarations in `crates/scone-ffi/include/scone.h` |
| `scone-bench` | Rust evaluation harness |

Recall combines lexical and vector results with facts and recency. Tags and
source filters narrow evidence. Recorded facts and model extractions remain
claims, not independently verified truth. `--as-of` queries validity intervals;
it does not reconstruct the database's past state before later corrections.
Returned/stored byte reduction is not measured token reduction or answer quality.
Rust and Python have distinct implementations and storage, with only a bounded
episode-transfer conformance profile rather than complete portability.

Run `scone --help` or `scone <command> --help` for supported commands. For an
explicitly model-free local console or MCP server:

```sh
./target/debug/scone --embedder hash --llm none ui
./target/debug/scone --embedder hash --llm none --space myproject mcp
```

The console binds to loopback and uses a process-scoped credential. Its native
console and preview playground are embedded from existing HTML files at compile
time. Building Rust does not rebuild the frontend. See [CONTRIBUTING.md](CONTRIBUTING.md)
for the explicit asset-update procedure, native checks and external conformance.
The separate Webapp's Python-oriented Memory page has not been established as
a compatible replacement for the Rust native console.

## C ABI and releases

```sh
cargo build --locked --release -p scone-ffi --no-default-features
```

This produces a native shared library in `target/release` (`libscone_ffi.dylib`
on macOS, `libscone_ffi.so` on Linux). The C ABI exposes a subset of the engine;
follow its header's ownership and error rules. A model-free FFI build requires
the hash embedder selection rather than local ONNX.

The release workflow packages the CLI, C header, license, citation and artifact
notes for Apple Silicon macOS and x86-64 Linux. It does **not** package a compiled
FFI library. Manual workflow runs produce downloadable CI artifacts; version
tags publish releases. No frontend or Python build runs during packaging.


## Contributing, license and citation

See [CONTRIBUTING.md](CONTRIBUTING.md) for setup and review requirements.
The [ProjectScone Research Attribution License](LICENSE) is custom and
MIT-derived, with mandatory research and academic citation. Credit Mark
Sturman, JudgeHuman and ProjectScone. [CITING.md](CITING.md) provides MLA,
APA, Chicago and BibTeX examples; [CITATION.cff](CITATION.cff) provides metadata.
