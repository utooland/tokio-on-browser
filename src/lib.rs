#![cfg(all(target_family = "wasm", target_os = "unknown"))]

extern crate console_error_panic_hook;

use std::{
    future::Future,
    panic,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
    time::Duration,
};

use tokio::{runtime, sync::oneshot, task_local};
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

task_local! {
    static TT: u8;
}

#[wasm_bindgen]
pub async fn run() -> Result<String, JsValue> {
    tracing::info!("start task");

    let hello_path = "hello".to_string();

    tokio_fs_ext::write(&hello_path, "worlddddd".as_bytes()).await;

    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(10);
    let (txs, mut rxs) = tokio::sync::mpsc::channel::<String>(10);

    let path_cloned = hello_path.clone();
    let wrap = TOKIO_RUNTIME.spawn(async {
        let ret = tokio::spawn(async {
            TT.scope(1, async {
                tokio::task::spawn(async move {
                    tx.send(path_cloned.clone()).await;
                    let ret = rxs.recv().await.unwrap();
                    let thread_id = tokio::runtime::Handle::current().id();
                    tracing::info!("read_to_string {path_cloned} {ret} in {thread_id}");
                    Result::<String, String>::Ok(ret)
                })
                .await
                .unwrap()
            })
            .await
        })
        .await
        .unwrap();

        ret
    });

    let run = async {
        while let Some(task) = rx.recv().await {
            txs.send(read_to_string(task).await.unwrap()).await;
        }
    };

    let (ret, _) = futures::future::join(wrap, run).await;

    let ret = ret.map_err(|e| e.to_string())?.unwrap();

    tracing::info!("read_to_string end {hello_path} {ret}");

    Ok(ret)
}

#[wasm_bindgen(start)]
fn start() {
    panic::set_hook(Box::new(console_error_panic_hook::hook));

    let fmt_layer = fmt::layer()
        .without_time()
        .with_span_events(FmtSpan::ENTER | FmtSpan::CLOSE)
        .with_writer(MakeWebConsoleWriter::new())
        .with_filter(EnvFilter::new({
            ["tokio=trace", "tokio_fs_ext=debug", "tokio_browser=debug"].join(",")
        }));

    registry().with(fmt_layer).init();
}

pub static TOKIO_RUNTIME: LazyLock<runtime::Runtime> = LazyLock::new(|| {
    runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .thread_name_fn(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!("tokio-runtime-worker-{id}")
        })
        .wasm_bindgen_shim_url("http://localhost:9091/tokio_worker.js".to_string())
        .build()
        .unwrap()
});
