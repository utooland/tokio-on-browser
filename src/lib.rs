#![cfg(all(target_family = "wasm", target_os = "unknown"))]

extern crate console_error_panic_hook;

use std::panic;

use tokio_fs_ext::{create_dir_all, read_to_string, write};
use tracing_subscriber::{
    fmt::{self, format::FmtSpan},
    layer::SubscriberExt,
    registry,
    util::SubscriberInitExt,
    EnvFilter, Layer,
};

use tracing_web::MakeWebConsoleWriter;
use wasm_bindgen::{prelude::wasm_bindgen, JsValue};

#[wasm_bindgen]
pub async fn mkdir(path: &str) {
    create_dir_all(path).await.unwrap()
}

#[wasm_bindgen(js_name = writeFile)]
pub async fn write_file(path: &str, content: &str) {
    write(path, content.as_bytes()).await.unwrap()
}

#[wasm_bindgen(js_name = readFile)]
pub async fn read_file(path: &str) -> String {
    read_to_string(path).await.unwrap()
}

#[wasm_bindgen]
pub async fn run() -> Result<String, JsValue> {
    tokio_fs_ext::write("hello", "world".as_bytes()).await;
    let rt = tokio::runtime::Builder::new_multi_thread()
        .wasm_bindgen_shim_url("http://localhost:9091/tokio_worker.js".to_string())
        .worker_threads(4)
        .build()
        .unwrap();

    tracing::info!("start task");

    let ret = rt
        .spawn_blocking(|| async { read_to_string("hello").await })
        .await
        .map_err(|e| JsValue::from(&e.to_string()))?
        .await
        .map_err(|e| JsValue::from(&e.to_string()))?;

    tracing::info!("read_to_string hello {ret}");

    Ok(ret)
}

#[wasm_bindgen(start)]
fn init_pack() {
    panic::set_hook(Box::new(console_error_panic_hook::hook));

    let fmt_layer = fmt::layer()
        .without_time()
        .with_span_events(FmtSpan::ENTER | FmtSpan::CLOSE)
        .with_writer(MakeWebConsoleWriter::new())
        .with_filter(EnvFilter::new({
            let mut trace = vec![];
            trace.extend([
                "tokio=debug",
                "wasm_thread=debug",
                "tokio_fs_ext=debug",
                "tokio_browser=debug",
            ]);
            trace.join(",")
        }));

    registry().with(fmt_layer).init();
}
