///////////////// Forked from hyper example START //////
/// https://github.com/hyperium/hyper/blob/master/examples/service_struct_impl.rs
#[cfg(feature = "hyper")]
use hyper::{Body, Request, Response};

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

type Counter = i32;

struct Svc {
    counter: Counter,
}

impl hyper::service::Service<Request<Body>> for Svc {
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        fn mk_response(s: String) -> Result<Response<Body>, hyper::Error> {
            Ok(Response::builder().body(Body::from(s)).unwrap())
        }

        let res = match req.uri().path() {
            "/" => mk_response(format!("home! counter = {:?}", self.counter)),
            "/posts" => mk_response(format!("posts, of course! counter = {:?}", self.counter)),
            "/authors" => mk_response(format!(
                "authors extraordinare! counter = {:?}",
                self.counter
            )),
            // Return the 404 Not Found for other routes, and don't increment counter.
            _ => return Box::pin(async { mk_response("oh no! not found".into()) }),
        };

        if req.uri().path() != "/favicon.ico" {
            self.counter += 1;
        }

        Box::pin(async { res })
    }
}

struct MakeSvc {
    counter: Counter,
}

impl<T> hyper::service::Service<T> for MakeSvc {
    type Response = Svc;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: T) -> Self::Future {
        let counter = self.counter.clone();
        let fut = async move { Ok(Svc { counter }) };
        Box::pin(fut)
    }
}
///////////////// Forked from hyper example END //////

/// Our hyper example STARTS from here
use overclock::core::*;

struct Hyper {
    addr: std::net::SocketAddr,
}

impl Hyper {
    fn new(addr: std::net::SocketAddr) -> Self {
        Self { addr }
    }
}

#[async_trait::async_trait]
impl ChannelBuilder<HyperChannel<MakeSvc>> for Hyper {
    async fn build_channel(&mut self) -> ActorResult<HyperChannel<MakeSvc>> {
        let make_svc = MakeSvc { counter: 81818 };
        let server = hyper::Server::try_bind(&self.addr)
            .map_err(|e| {
                log::error!("{}", e);
                ActorError::exit_msg(e)
            })?
            .serve(make_svc);
        Ok(HyperChannel::new(server))
    }
}
#[async_trait::async_trait]
impl<S> Actor<S> for Hyper
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = HyperChannel<MakeSvc>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!("Hyper: {}", rt.service().status());
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _deps: Self::Data) -> ActorResult<()> {
        log::info!("Hyper: {}", rt.service().status());
        if let Err(err) = rt.inbox_mut().ignite().await {
            log::error!("Hyper: {}", err);
        }
        log::info!("Hyper exited its loop");
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    #[cfg(not(feature = "console"))]
    {
        let env = env_logger::Env::new().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }
    let addr = ([127, 0, 0, 1], 3000).into();
    let hyper = Hyper::new(addr);
    let runtime = Runtime::new("hyper".to_string(), hyper)
        .await
        .expect("Runtime to run");
    runtime
        .block_on()
        .await
        .expect("Runtime to shutdown gracefully");
}
