import initWasm, { run } from "./wasm";

(async () => {
  await initWasm();
  let ret = await run();
  console.log("return from tokio runtime:", ret);
})()
