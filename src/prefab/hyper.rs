/// Our hyper example STARTS from here
use crate::core::*;

/// Hyper prefab actor struct
pub struct Hyper<T> {
    addr: std::net::SocketAddr,
    make_svc: Option<T>,
}

impl<T> Hyper<T> {
    /// Create new hyper server
    pub fn new(addr: std::net::SocketAddr, make_svc: T) -> Self {
        Self {
            addr,
            make_svc: Some(make_svc),
        }
    }
}

#[async_trait::async_trait]
impl<T, E, F, R> ChannelBuilder<HyperChannel<T>> for Hyper<T>
where
    for<'a> T: Send
        + Sync
        + 'static
        + hyper::service::Service<&'a hyper::server::conn::AddrStream, Error = E, Response = R, Future = F>
        + Send,
    E: std::error::Error + Send + Sync + 'static,
    F: Send + std::future::Future<Output = Result<R, E>> + 'static,
    R: Send + hyper::service::Service<hyper::Request<hyper::Body>, Response = hyper::Response<hyper::Body>> + 'static,
    R::Error: std::error::Error + Send + Sync,
    R::Future: Send,
{
    async fn build_channel(&mut self) -> ActorResult<HyperChannel<T>> {
        if let Some(make_svc) = self.make_svc.take() {
            let server = hyper::Server::try_bind(&self.addr)
                .map_err(|e| {
                    log::error!("{}", e);
                    ActorError::exit_msg(e)
                })?
                .serve(make_svc);
            Ok(HyperChannel::new(server))
        } else {
            log::error!("No provided make svc to serve");
            return Err(ActorError::exit_msg("No provided make svc to serve"));
        }
    }
}

#[async_trait::async_trait]
impl<T, E, F, R, S> Actor<S> for Hyper<T>
where
    S: SupHandle<Self>,
    for<'a> T: Send
        + Sync
        + 'static
        + hyper::service::Service<&'a hyper::server::conn::AddrStream, Error = E, Response = R, Future = F>
        + Send,
    E: std::error::Error + Send + Sync + 'static,
    F: Send + std::future::Future<Output = Result<R, E>> + 'static,
    R: Send + hyper::service::Service<hyper::Request<hyper::Body>, Response = hyper::Response<hyper::Body>> + 'static,
    R::Error: std::error::Error + Send + Sync,
    R::Future: Send,
{
    type Data = String;
    type Channel = HyperChannel<T>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        let name: String = rt.service().directory().clone().unwrap_or_else(|| "hyper".into());
        log::info!("{}: {}",name, rt.service().status());
        Ok(name)
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, name: Self::Data) -> ActorResult<()> {
        log::info!("{}: {}", name, rt.service().status());
        if let Err(err) = rt.inbox_mut().ignite().await {
            log::error!("{}: {}",name, err);
            return Err(ActorError::exit_msg(err));
        }
        log::info!("{} gracefully shutdown", name);
        Ok(())
    }
}
