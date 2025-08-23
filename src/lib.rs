#![cfg(all(target_family = "wasm", target_os = "unknown"))]

extern crate console_error_panic_hook;

use std::{
    future::Future,
    panic,
    path::PathBuf,
    pin::Pin,
    sync::{
        atomic::{AtomicUsize, Ordering},
        LazyLock,
    },
    time::Duration,
};

use futures::FutureExt;
use tokio::{runtime, task_local, time::Instant};
use tokio_fs_ext::{create_dir_all, offload, read_to_string, write};
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
pub async fn mkdir(path: &str) -> Result<(), JsValue> {
    create_dir_all(path).await.map_err(|e| e.to_string().into())
}

#[wasm_bindgen(js_name = writeFile)]
pub async fn write_file(path: &str, content: &str) -> Result<(), JsValue> {
    write(path, content.as_bytes())
        .await
        .map_err(|e| e.to_string().into())
}

#[wasm_bindgen(js_name = readFile)]
pub async fn read_file(path: &str) -> Result<String, JsValue> {
    read_to_string(path).await.map_err(|e| e.to_string().into())
}

task_local! {
    static TT: u8;
}

const HELLO_PATH: &str = "hello";

const SOMETHING: &str = " s o m e t h i n g ";

#[wasm_bindgen]
pub async fn run() -> Result<String, String> {
    tracing::info!("start task");

    tokio_fs_ext::write(HELLO_PATH, "world d d d d".as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let (fs_server, fs_client_0) = offload::split();

    let fs_client_1 = fs_client_0.clone();
    let fs_client_2 = fs_client_0.clone();
    let fs_client_3 = fs_client_0.clone();
    let fs_client_4 = fs_client_0.clone();

    // Clone once to pass owned copies to the main Tokio task
    let tokio_task = TOKIO_RUNTIME.spawn(async move {
        let fs_client = fs_client_0;
        // Clone for the scoped task
        let scoped_task = tokio::spawn(async move {
            // Ensure tokio::task::LocalKey::scope worked
            TT.scope(1, async move {
                let ret = fs_client
                    .read(HELLO_PATH)
                    .await
                    .map(|buf| unsafe { String::from_utf8_unchecked(buf) })
                    .map_err(|e| e.to_string())?;

                let thread_id = std::thread::current().id();
                tracing::info!(
                    "read_to_string {:?} {} in tokio worker thread: {:?}",
                    HELLO_PATH,
                    ret,
                    thread_id
                );
                Result::<String, String>::Ok(ret)
            })
            .await
        });

        tokio::join!(
            write_and_read_test(fs_client_1, "/1/1"),
            write_and_read_test(fs_client_2, "/2/2"),
            write_and_read_test(fs_client_3, "/3/3")
        );

        let normal_task = tokio::task::spawn(async move {
            let ret = fs_client_4
                .read(HELLO_PATH)
                .await
                .map(|buf| unsafe { String::from_utf8_unchecked(buf) })
                .map_err(|e| e.to_string())?;

            let thread_id = std::thread::current().id();
            tracing::info!(
                "read_to_string {:?} {} in tokio worker thread: {:?}",
                HELLO_PATH,
                ret,
                thread_id
            );
            Result::<String, String>::Ok(ret)
        });

        // Ensure tokio::task::spawn blocking worked
        let _ = tokio::join!(
            tokio::task::spawn(blocking_task(40, 40)),
            tokio::task::spawn(blocking_task(20, 20)),
            tokio::task::spawn(blocking_task(10, 10)),
            tokio::task::spawn(blocking_task(30, 30)),
        );

        let (ret_1, ret_2) = tokio::join!(
            async { scoped_task.await.map_err(|e| e.to_string())? },
            async { normal_task.await.map_err(|e| e.to_string())? }
        );

        let ret_1 = ret_1.map_err(|e| e.to_string())?;
        let ret_2 = ret_2.map_err(|e| e.to_string())?;

        assert_eq!(ret_1, ret_2);

        Result::<String, String>::Ok(ret_1)
    });

    let (ret, _) = futures::future::join(
        tokio_task,
        // start the fs_actor, because calling the opfs api in tokio runtime will
        // cause hanging forever, the reason is that, calling betweean worker threads
        // on browser is scheduled in browser's event loop, but tokio runtime worker will
        // blocking or park the thread, so the browser's event loop will be blocked.
        // See: https://github.com/tokio-rs/tokio/blob/925c614c89d0a26777a334612e2ed6ad0e7935c3/tokio/src/runtime/scheduler/multi_thread/worker.rs#L524:L567
        // and https://github.com/chemicstry/wasm_thread/blob/7ec48686bb1a0d9bd42cf16e46622746e4d12ab3/README.md?plain=1#L22:L26
        fs_server.serve(offload::FsOffloadDefault),
    )
    .await;

    let ret = ret.map_err(|e| e.to_string())??;

    tracing::info!("read_to_string end {:?} {}", HELLO_PATH, ret);

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
        .worker_threads(4)
        .thread_name_fn(|| {
            static ATOMIC_ID: AtomicUsize = AtomicUsize::new(0);
            let id = ATOMIC_ID.fetch_add(1, Ordering::SeqCst);
            format!(r#"tokio-runtime-worker-{id}"#)
        })
        .wasm_bindgen_shim_url("http://localhost:9091/tokio_worker.js".to_string())
        .build()
        .expect("Failed to build Tokio runtime")
});

async fn write_and_read_test(fs_client: offload::Client, dir: &str) -> Result<String, String> {
    fs_client
        .create_dir_all(dir)
        .await
        .map_err(|e| e.to_string())?;

    let path = format!("{dir}/rw");

    fs_client
        .write(&path, SOMETHING.as_bytes())
        .await
        .map_err(|e| e.to_string())?;

    let ret = fs_client
        .read(&path)
        .await
        .map(|buf| unsafe { String::from_utf8_unchecked(buf) })
        .map_err(|e| e.to_string())?;

    assert_eq!(ret, SOMETHING);

    let thread_id = std::thread::current().id();
    tracing::info!(
        "read_to_string {:?} {} in tokio worker thread: {:?}",
        &path,
        ret,
        thread_id
    );

    Ok(ret)
}

async fn blocking_task(id: u32, n: u64) {
    let tokio_handle_id = tokio::runtime::Handle::current().id();
    tracing::info!(
        "[Task {}] Starting blocking task in tokio worker {}...",
        id,
        tokio_handle_id
    );

    let start = Instant::now();

    let result = fibonacci(n);

    let duration = start.elapsed();

    let thread_id = std::thread::current().id();

    tracing::info!(
        "[Task {}] Fibonacci({}) calculated to be {} in {:?} in tokio worker thread: {:?}",
        id,
        n,
        result,
        duration,
        thread_id
    );
}

fn fibonacci(n: u64) -> u64 {
    if n <= 1 {
        n
    } else {
        fibonacci(n - 1) + fibonacci(n - 2)
    }
}
