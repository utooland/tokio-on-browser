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

use futures::channel::oneshot::{channel, Receiver, Sender};
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

type RtcTask<R> = Box<(dyn FnOnce() -> Pin<Box<dyn Future<Output = R>>> + Send)>;

struct Caller<T> {
    sender: Sender<RtcTask<T>>,
    receiver: Receiver<T>,
}

struct Callee<T> {
    sender: Sender<T>,
    receiver: Receiver<RtcTask<T>>,
}

fn rtc<T>() -> (Caller<T>, Callee<T>) {
    let (tx_caller, mut rx_caller) =
        channel::<Box<(dyn FnOnce() -> Pin<Box<dyn Future<Output = T>>> + Send)>>();
    let (tx_callee, mut rx_callee) = channel::<T>();
    (
        Caller {
            sender: tx_caller,
            receiver: rx_callee,
        },
        Callee {
            sender: tx_callee,
            receiver: rx_caller,
        },
    )
}

#[wasm_bindgen]
pub async fn run() -> Result<String, JsValue> {
    tracing::info!("start task");

    let path = "hello".to_string();

    tokio_fs_ext::write(&path, "world d d d d".as_bytes()).await;

    let (mut caller, mut callee) = rtc::<String>();

    let path_cloned = path.clone();
    let wrap = TOKIO_RUNTIME.spawn(async move {
        let ret = tokio::spawn(async move {
            TT.scope(1, async move {
                tokio::task::spawn(async move {
                    let path_cloned = path_cloned.clone();
                    let path_cloned_cloned = path_cloned.clone();
                    caller.sender.send(Box::new(|| {
                        Box::pin(async { tokio_fs_ext::read_to_string(path_cloned).await.unwrap() })
                    }));
                    let ret = caller.receiver.await.unwrap();
                    let thread_id = tokio::runtime::Handle::current().id();
                    tracing::info!("read_to_string {path_cloned_cloned} {ret} in {thread_id}");
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

    let run = async { callee.sender.send(callee.receiver.await.unwrap()().await) };

    let (ret, _) = futures::future::join(wrap, run).await;

    let ret = ret.map_err(|e| e.to_string())?.unwrap();

    tracing::info!("read_to_string end {path} {ret}");

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
            ["tokio=debug", "tokio_fs_ext=debug", "tokio_browser=debug"].join(",")
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
