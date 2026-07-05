# wasm-bindgen threads stall with large initial shared memory

This branch is a minimal browser repro for a wasm-bindgen threads runtime
layout issue that becomes visible with a large linker-provided initial shared
memory.

## Environment

- Rust: `nightly-2026-05-15`
- wasm-bindgen crate and CLI: `0.2.126`
- Target: `wasm32-unknown-unknown`
- Memory flags:
  - `--shared-memory`
  - `--import-memory`
  - `--initial-memory=67108864`
  - `--max-memory=4294967296`

## Reproduce

```sh
npm install
cargo install wasm-bindgen-cli@0.2.126 --locked
rustup target add wasm32-unknown-unknown --toolchain nightly-2026-05-15
npm run dev
npm run start
```

Open `http://localhost:9091`.

Expected successful run:

- Console contains `start task in thread ...`.
- Console later contains `fs_event: ...`.
- Console later contains `read_to_string end ...`.
- Console finally contains `return from tokio runtime: ...`.

Actual behavior with the checked-in `--initial-memory=67108864` flag:

- Console reaches `start task in thread ...`.
- Tokio worker tasks do not reach the expected `fs_event`,
  `read_to_string end`, or `return from tokio runtime` messages.
- There is no ordinary Rust panic explaining the stall.

## Control

Remove this line from `.cargo/config.toml`:

```toml
"-Clink-arg=--initial-memory=67108864",
```

Then rebuild:

```sh
rm -rf js/wasm dist
npm run dev
npm run start
```

Open `http://localhost:9091` again. The same app reaches
`return from tokio runtime: ...`.

## Why this points at wasm-bindgen's thread layout

`--initial-memory=67108864` is a legal linker configuration. It only makes the
imported shared memory start at roughly 64 MiB instead of the smaller value
computed from static data, TLS, stack, and linker defaults.

The observable failure is tied to wasm-bindgen's threads bootstrap layout:
wasm-bindgen injects thread bookkeeping and a temporary stack near the original
`__heap_base`, then adjusts the memory globals/page count in the transformed
module. With a large pre-grown imported shared memory, that injected area is not
robustly reserved from the rest of the wasm runtime/allocator-visible address
space. On `nightly-2026-05-15`, this can corrupt or block worker startup enough
that the Tokio runtime no longer schedules expected tasks.

The important part of the repro is the pair of facts:

- `--initial-memory=67108864`: build succeeds, browser runtime stalls.
- without `--initial-memory=67108864`: same app and toolchain reaches the Tokio
  completion log.
