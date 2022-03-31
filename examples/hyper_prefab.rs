//// Based on hyper hello example https://github.com/hyperium/hyper/blob/master/examples/hello.rs
use std::convert::Infallible;

#[cfg(feature = "hyper")]
use hyper::{
    service::{make_service_fn, service_fn},
    Body, Request, Response,
};

async fn hello(_: Request<Body>) -> Result<Response<Body>, Infallible> {
    Ok(Response::new(Body::from("Hello World!")))
}
#[cfg(feature = "hyper")]
// our prefab example starts from here
use overclock::{core::*, prefab::hyper::Hyper};

#[tokio::main]
async fn main() {
    #[cfg(not(feature = "console"))]
    {
        let env = env_logger::Env::new().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }
    let make_svc = make_service_fn(|_conn| async { Ok::<_, Infallible>(service_fn(hello)) });
    let addr = ([127, 0, 0, 1], 3000).into();
    let hyper = Hyper::new(addr, make_svc);
    let runtime = Runtime::new("hyper".to_string(), hyper)
        .await
        .expect("Runtime to run");
    runtime
        .block_on()
        .await
        .expect("Runtime to shutdown gracefully");
}
