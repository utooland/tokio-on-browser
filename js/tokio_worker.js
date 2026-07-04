import { initSync } from "./wasm";

self.wasm_bindgen = (module, memory) => {
  return initSync({ module, memory });
};
