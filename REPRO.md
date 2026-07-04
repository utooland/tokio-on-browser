# wasm-bindgen thread static data overlap repro

This branch reproduces a wasm-bindgen threads memory layout problem with newer
Rust std builds.

## Environment

- Rust toolchain: `nightly-2026-05-15`
- wasm-bindgen CLI used by the default scripts: `0.2.108`
- Latest checked wasm-bindgen CLI support source: `0.2.126`
- Target: `wasm32-unknown-unknown` with shared memory and atomics

## Regression trigger

This repo worked before upgrading the Rust toolchain from
`nightly-2025-06-04` to `nightly-2026-05-15`. The upgrade changed the Rust std
wasm allocator dependency from `dlmalloc 0.2.8` to `dlmalloc 0.2.13`.

The old `dlmalloc 0.2.8` wasm allocator only allocated through `memory.grow`.
The new `dlmalloc 0.2.13` wasm allocator first tries to donate the
linker-provided preexisting heap range bounded by `__heap_base` and
`__heap_end`, then falls back to `memory.grow`.

The same thread transform is still present in `wasm-bindgen-cli-support`
`0.2.126`: it allocates the wasm-bindgen thread counter and temporary stack page
starting at the original exported `__heap_base`, then mutates only the
`__heap_base` global and the initial memory page count.

## Reproduce the raw wasm-bindgen output

```sh
npm install
npm run install-toolchain
npm run dev:raw
npm run start
```

Open `http://localhost:9091`.

With the raw wasm-bindgen output, worker initialization does not reach normal
Tokio task scheduling. The failure appears before the Tokio scheduler itself is
the deciding factor: wasm-bindgen's injected thread static data overlaps memory
that Rust std can now donate to the allocator.

## Local workaround

```sh
npm run dev
npm run start
```

The default `dev` script runs wasm-bindgen and then applies
`scripts/fix-wasm-bindgen-thread-page.mjs`. The script moves wasm-bindgen's
thread static data to the next 64 KiB page boundary while preserving the memory
page count that wasm-bindgen already added.

## Concrete layout observed in this repo

Before wasm-bindgen:

- Imported shared memory has 19 initial pages.
- Exported `__heap_base` is `1196624`.

Raw wasm-bindgen output:

- Imported shared memory is bumped to 20 initial pages.
- `__wbindgen_start` uses `1196624` as the thread counter address.
- `__wbindgen_start` uses `1196628` as the temporary stack lock address.
- `__wbindgen_start` uses `1262160` as the temporary stack top.

Patched output:

- `__wbindgen_start` uses `1245184` as the thread counter address.
- `__wbindgen_start` uses `1245188` as the temporary stack lock address.
- `__wbindgen_start` uses `1310720` as the temporary stack top.

## Root cause

Newer Rust std pulls in newer `dlmalloc` for wasm. That allocator can now donate
the linker-provided preexisting heap range, bounded by `__heap_base` and
`__heap_end`, before falling back to `memory.grow`.

wasm-bindgen's threads transform still assumes it can reserve its thread static
data at the original exported `__heap_base` by mutating the wasm global and
increasing the initial memory size. That is no longer enough because Rust std
allocator code can still use the original linker heap range. The thread counter,
temporary stack lock, and temporary stack page can then be overwritten by
allocator traffic.

Using `mimalloc` as the global allocator does not fully remove the issue in this
repo. Some reachable Rust std paths still explicitly use `std::alloc::System`,
including thread-local destructor registration and `std::thread::current`
initialization paths used during worker/runtime startup.
