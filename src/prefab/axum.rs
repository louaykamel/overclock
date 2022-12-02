// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

/// Our tonic example STARTS from here
use crate::core::*;

/// Tonic prefab actor struct
pub struct Axum<T = std::net::SocketAddr> {
    addr: std::net::SocketAddr,
    router: Option<axum::Router>,
    _marker: std::marker::PhantomData<fn() -> T>,
}

impl<T> Axum<T> {
    /// Create new tonic server
    pub fn new(addr: std::net::SocketAddr, router: axum::Router) -> Self {
        Self {
            addr,
            router: Some(router),
            _marker: std::marker::PhantomData,
        }
    }
}

#[async_trait::async_trait]
impl<T: for<'a> axum::extract::connect_info::Connected<&'a ::hyper::server::conn::AddrStream>>
    ChannelBuilder<AxumChannel<T>> for Axum<T>
{
    async fn build_channel(&mut self) -> ActorResult<AxumChannel<T>> {
        if let Some(router) = self.router.take() {
            let builder = hyper::Server::try_bind(&self.addr).map_err(|e| {
                log::error!("{}", e);
                ActorError::exit_msg(e)
            })?;
            Ok(AxumChannel::new(router, builder))
        } else {
            log::error!("No provided make service to serve");
            return Err(ActorError::exit_msg("No provided make service to serve"));
        }
    }
}

#[async_trait::async_trait]
impl<T: for<'a> axum::extract::connect_info::Connected<&'a ::hyper::server::conn::AddrStream>, S: SupHandle<Self>>
    Actor<S> for Axum<T>
{
    type Data = String;
    type Channel = AxumChannel<T>;
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
