// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

/// Our tonic example STARTS from here
use crate::core::*;

use ::hyper::{Body, Request, Response};
use tonic::transport::server::NamedService;
use tower::Service;

/// Tonic prefab actor struct
pub struct Tonic<T> {
    addr: std::net::SocketAddr,
    service: Option<T>,
}

impl<T> Tonic<T> {
    /// Create new tonic server
    pub fn new(addr: std::net::SocketAddr, service: T) -> Self {
        Self {
            addr,
            service: Some(service),
        }
    }
}

#[async_trait::async_trait]
impl<T> ChannelBuilder<TonicChannel<T>> for Tonic<T>
where
    T: Send + Sync + 'static + NamedService + Service<Request<Body>, Response = Response<tonic::body::BoxBody>>,
    T::Future: Send,
    T::Error: Send,
{
    async fn build_channel(&mut self) -> ActorResult<TonicChannel<T>> {
        if let Some(service) = self.service.take() {
            let addrincoming = ::hyper::server::conn::AddrIncoming::bind(&self.addr).map_err(|e| {
                log::error!("{}", e);
                ActorError::exit_msg(e)
            })?;
            let server = tonic::transport::Server::builder();
            Ok(TonicChannel::new(server, service, addrincoming))
        } else {
            log::error!("No provided make service to serve");
            return Err(ActorError::exit_msg("No provided make service to serve"));
        }
    }
}

#[async_trait::async_trait]
impl<T, S: SupHandle<Self>> Actor<S> for Tonic<T>
where
    T: Clone
        + Send
        + Sync
        + 'static
        + NamedService
        + Service<Request<Body>, Response = Response<tonic::body::BoxBody>, Error = std::convert::Infallible>,
    T::Future: Send,
    T::Error: Send,
{
    type Data = String;
    type Channel = TonicChannel<T>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        let name: String = rt.service().directory().clone().unwrap_or_else(|| "tonic".into());
        log::info!("{}: {}", name, rt.service().status());
        Ok(name)
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, name: Self::Data) -> ActorResult<()> {
        log::info!("{}: {}", name, rt.service().status());
        if let Err(err) = rt.inbox_mut().ignite().await {
            log::error!("{}: {}", name, err);
            return Err(ActorError::exit_msg(err));
        }
        log::info!("{} gracefully shutdown", name);
        Ok(())
    }
}
