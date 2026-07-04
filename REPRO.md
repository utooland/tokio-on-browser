# wasm-bindgen thread static data overlap repro

This branch reproduces a wasm-bindgen threads memory layout problem with newer
Rust std builds.

## Environment

- Rust toolchain: `nightly-2026-05-15`
- wasm-bindgen CLI used by the default scripts: `0.2.108`
- Latest checked wasm-bindgen CLI support source: `0.2.126`
- Target: `wasm32-unknown-unknown` with shared memory and atomics
- No custom global allocator. This branch intentionally uses Rust std's wasm
  allocator path so the failure is observable.

## Regression trigger

The known working baseline is the Rust toolchain used by the sibling `utoo`
repo: `nightly-2026-04-02` (`rustc 1.96.0-nightly 7e46c5f6f
2026-04-01`). The regression is observed after upgrading to
`nightly-2026-05-15`. That upgrade changed the Rust std wasm allocator
dependency from `dlmalloc 0.2.11` to `dlmalloc 0.2.13`.

The old `dlmalloc 0.2.11` wasm allocator only allocated through `memory.grow`.
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

## Address probe

```sh
npm run dev
npm run start
```

The default `dev` script runs wasm-bindgen and then applies
`scripts/fix-wasm-bindgen-thread-page.mjs`. The script moves wasm-bindgen's
thread static data to the next 64 KiB page boundary while preserving the memory
page count that wasm-bindgen already added.

In this no-mimalloc repro shape, moving only those constants is useful as a
layout probe, but it is not a complete runtime fix. The failing repro command is
`npm run dev:raw`.

Adding a custom global allocator such as the sibling repo's forked `mimalloc`
can hide this smoke-test failure because the common `__rust_alloc` path no
longer uses Rust std's wasm `dlmalloc` allocator. That allocator change is a
mitigation for this repro shape, not evidence that wasm-bindgen's injected
thread static page is placed outside memory Rust std can use.

## Concrete layout observed in this repo

Before wasm-bindgen with `nightly-2026-05-15`:

- Imported shared memory has 18 initial pages.
- Exported `__heap_base` is around `1157536` in the no-mimalloc build.

Raw wasm-bindgen output:

- Imported shared memory remains 19 initial pages for this no-mimalloc layout.
- `__wbindgen_start` uses `1157536` as the thread counter address.
- `__wbindgen_start` uses `1157540` as the temporary stack lock address.
- `__wbindgen_start` uses `1223072` as the temporary stack top.

Patched output:

- `__wbindgen_start` uses `1179648` as the thread counter address.
- `__wbindgen_start` uses `1179652` as the temporary stack lock address.
- `__wbindgen_start` uses `1245184` as the temporary stack top.

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

Using `mimalloc` as the global allocator changes this smoke test: the raw output
can complete because the dominant allocation path no longer consumes Rust std's
wasm `dlmalloc` preexisting heap. The underlying wasm-bindgen layout issue is
still the same transform assumption: it places its thread static page at the
original linker `__heap_base` and only mutates the wasm global afterwards.
