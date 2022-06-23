// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

//// Based on minimal axum hello example

use axum::{routing::get, Router};

// basic handler that responds with a static string
async fn root() -> &'static str {
    "Hello, World!"
}

// our prefab example starts from here
use overclock::{core::*, prefab::axum::Axum};

#[tokio::main]
async fn main() {
    #[cfg(not(feature = "console"))]
    {
        let env = env_logger::Env::new().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }
    let addr = ([127, 0, 0, 1], 3000).into();
    // build our application with a route
    let app = Router::new().route("/", get(root));
    let axum = Axum::new(addr, app);
    let runtime = Runtime::new("axum".to_string(), axum).await.expect("Runtime to run");
    runtime.block_on().await.expect("Runtime to shutdown gracefully");
}
