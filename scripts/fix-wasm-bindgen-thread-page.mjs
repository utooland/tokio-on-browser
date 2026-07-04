#!/usr/bin/env node

import { readFileSync, writeFileSync } from "node:fs";

const PAGE_SIZE = 64 * 1024;

const file = process.argv[2];
if (!file) {
  throw new Error("Usage: node scripts/fix-wasm-bindgen-thread-page.mjs <wasm>");
}

const wasm = readFileSync(file);
if (
  wasm.length < 8 ||
  wasm[0] !== 0x00 ||
  wasm[1] !== 0x61 ||
  wasm[2] !== 0x73 ||
  wasm[3] !== 0x6d
) {
  throw new Error(`${file} is not a WebAssembly binary`);
}

const moduleInfo = parseModule(wasm);
const startRange = getExportedFunctionBody(moduleInfo, "__wbindgen_start");
const destroyRange = getExportedFunctionBody(moduleInfo, "__wbindgen_thread_destroy");
const oldBase = findFirstLargeI32Const(wasm, startRange);
const newBase = alignUp(oldBase, PAGE_SIZE);

if (oldBase === newBase) {
  console.log(`wasm-bindgen thread page already aligned at ${oldBase}`);
  process.exit(0);
}

const requiredPages = Math.ceil((newBase + PAGE_SIZE) / PAGE_SIZE);
if (moduleInfo.minMemoryPages < requiredPages) {
  throw new Error(
    `wasm memory has ${moduleInfo.minMemoryPages} pages, but moving the wasm-bindgen thread page to ${newBase} requires ${requiredPages}`,
  );
}

const replacements = [
  [oldBase + PAGE_SIZE, newBase + PAGE_SIZE, "temporary stack top"],
  [oldBase + 4, newBase + 4, "temporary stack lock"],
  [oldBase, newBase, "thread counter"],
];

const counts = new Map();
for (const [from, to, name] of replacements) {
  counts.set(`start:${name}`, replaceI32Const(wasm, startRange, from, to));
  counts.set(`destroy:${name}`, replaceI32Const(wasm, destroyRange, from, to));
}

assertCount(counts, "start:thread counter", 1);
assertAtLeast(counts, "start:temporary stack lock", 2);
assertCount(counts, "start:temporary stack top", 1);
assertCount(counts, "destroy:thread counter", 0);
assertAtLeast(counts, "destroy:temporary stack lock", 2);
assertCount(counts, "destroy:temporary stack top", 1);

writeFileSync(file, wasm);
console.log(
  `moved wasm-bindgen thread page from ${oldBase} to ${newBase} (${moduleInfo.minMemoryPages} memory pages)`,
);

function parseModule(bytes) {
  const info = {
    functionImports: 0,
    exports: new Map(),
    codeBodies: [],
    minMemoryPages: 0,
  };

  let offset = 8;
  while (offset < bytes.length) {
    const id = bytes[offset++];
    const sectionSize = readU32(bytes, offset);
    offset = sectionSize.next;
    const sectionStart = offset;
    const sectionEnd = sectionStart + sectionSize.value;

    switch (id) {
      case 2:
        parseImportSection(bytes, sectionStart, info);
        break;
      case 5:
        parseMemorySection(bytes, sectionStart, info);
        break;
      case 7:
        parseExportSection(bytes, sectionStart, info);
        break;
      case 10:
        parseCodeSection(bytes, sectionStart, info);
        break;
      default:
        break;
    }

    offset = sectionEnd;
  }

  return info;
}

function parseImportSection(bytes, offset, info) {
  const count = readU32(bytes, offset);
  offset = count.next;

  for (let i = 0; i < count.value; i++) {
    offset = readName(bytes, offset).next;
    offset = readName(bytes, offset).next;
    const kind = bytes[offset++];

    if (kind === 0x00) {
      info.functionImports += 1;
      offset = readU32(bytes, offset).next;
    } else if (kind === 0x01) {
      offset += 1;
      offset = readLimits(bytes, offset).next;
    } else if (kind === 0x02) {
      const limits = readLimits(bytes, offset);
      info.minMemoryPages = Math.max(info.minMemoryPages, limits.min);
      offset = limits.next;
    } else if (kind === 0x03) {
      offset += 2;
    } else if (kind === 0x04) {
      offset = readU32(bytes, offset).next;
    } else {
      throw new Error(`unsupported import kind ${kind}`);
    }
  }
}

function parseMemorySection(bytes, offset, info) {
  const count = readU32(bytes, offset);
  offset = count.next;

  for (let i = 0; i < count.value; i++) {
    const limits = readLimits(bytes, offset);
    info.minMemoryPages = Math.max(info.minMemoryPages, limits.min);
    offset = limits.next;
  }
}

function parseExportSection(bytes, offset, info) {
  const count = readU32(bytes, offset);
  offset = count.next;

  for (let i = 0; i < count.value; i++) {
    const name = readName(bytes, offset);
    offset = name.next;
    const kind = bytes[offset++];
    const index = readU32(bytes, offset);
    offset = index.next;
    info.exports.set(name.value, { kind, index: index.value });
  }
}

function parseCodeSection(bytes, offset, info) {
  const count = readU32(bytes, offset);
  offset = count.next;

  for (let i = 0; i < count.value; i++) {
    const bodySize = readU32(bytes, offset);
    const start = bodySize.next;
    const end = start + bodySize.value;
    info.codeBodies.push({ start, end });
    offset = end;
  }
}

function getExportedFunctionBody(info, name) {
  const exported = info.exports.get(name);
  if (!exported || exported.kind !== 0x00) {
    throw new Error(`missing wasm function export ${name}`);
  }

  const bodyIndex = exported.index - info.functionImports;
  const body = info.codeBodies[bodyIndex];
  if (bodyIndex < 0 || !body) {
    throw new Error(`export ${name} does not point to a defined function body`);
  }

  return body;
}

function findFirstLargeI32Const(bytes, range) {
  for (let offset = range.start; offset < range.end; offset++) {
    if (bytes[offset] !== 0x41) {
      continue;
    }

    const immediate = readS32(bytes, offset + 1);
    if (immediate.value >= PAGE_SIZE && immediate.value % 4 === 0) {
      return immediate.value;
    }
  }

  throw new Error("could not find wasm-bindgen thread page base");
}

function replaceI32Const(bytes, range, from, to) {
  const fromImmediate = encodeS32(from);
  const toImmediate = encodeS32(to);
  if (fromImmediate.length !== toImmediate.length) {
    throw new Error(
      `cannot patch i32.const ${from} -> ${to}: LEB128 size changed from ${fromImmediate.length} to ${toImmediate.length}`,
    );
  }

  const pattern = Buffer.concat([Buffer.from([0x41]), fromImmediate]);
  const replacement = Buffer.concat([Buffer.from([0x41]), toImmediate]);
  let count = 0;

  for (let offset = range.start; offset <= range.end - pattern.length; offset++) {
    let matched = true;
    for (let i = 0; i < pattern.length; i++) {
      if (bytes[offset + i] !== pattern[i]) {
        matched = false;
        break;
      }
    }

    if (!matched) {
      continue;
    }

    replacement.copy(bytes, offset);
    count += 1;
    offset += pattern.length - 1;
  }

  return count;
}

function readName(bytes, offset) {
  const length = readU32(bytes, offset);
  const start = length.next;
  const end = start + length.value;
  return {
    value: Buffer.from(bytes.subarray(start, end)).toString("utf8"),
    next: end,
  };
}

function readLimits(bytes, offset) {
  const flags = bytes[offset++];
  const min = readU32(bytes, offset);
  offset = min.next;

  if ((flags & 0x01) !== 0) {
    offset = readU32(bytes, offset).next;
  }

  return { min: min.value, next: offset };
}

function readU32(bytes, offset) {
  let result = 0;
  let shift = 0;

  for (let i = 0; i < 5; i++) {
    const byte = bytes[offset++];
    result |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) {
      return { value: result >>> 0, next: offset };
    }
    shift += 7;
  }

  throw new Error("invalid u32 LEB128");
}

function readS32(bytes, offset) {
  let result = 0;
  let shift = 0;
  let byte = 0;

  for (let i = 0; i < 5; i++) {
    byte = bytes[offset++];
    result |= (byte & 0x7f) << shift;
    shift += 7;
    if ((byte & 0x80) === 0) {
      if (shift < 32 && (byte & 0x40) !== 0) {
        result |= ~0 << shift;
      }
      return { value: result, next: offset };
    }
  }

  throw new Error("invalid i32 LEB128");
}

function encodeS32(value) {
  const bytes = [];
  let remaining = value | 0;

  while (true) {
    let byte = remaining & 0x7f;
    remaining >>= 7;

    const done =
      (remaining === 0 && (byte & 0x40) === 0) ||
      (remaining === -1 && (byte & 0x40) !== 0);

    if (!done) {
      byte |= 0x80;
    }

    bytes.push(byte);

    if (done) {
      return Buffer.from(bytes);
    }
  }
}

function alignUp(value, alignment) {
  return Math.ceil(value / alignment) * alignment;
}

function assertCount(counts, key, expected) {
  const actual = counts.get(key) ?? 0;
  if (actual !== expected) {
    throw new Error(`expected ${expected} replacements for ${key}, got ${actual}`);
  }
}

function assertAtLeast(counts, key, expected) {
  const actual = counts.get(key) ?? 0;
  if (actual < expected) {
    throw new Error(`expected at least ${expected} replacements for ${key}, got ${actual}`);
  }
}
