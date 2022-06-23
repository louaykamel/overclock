// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

/// Our tonic example STARTS from here
use crate::core::*;

/// Tonic prefab actor struct
pub struct Axum {
    addr: std::net::SocketAddr,
    router: Option<axum::Router>,
}

impl Axum {
    /// Create new tonic server
    pub fn new(addr: std::net::SocketAddr, router: axum::Router) -> Self {
        Self {
            addr,
            router: Some(router),
        }
    }
}

#[async_trait::async_trait]
impl ChannelBuilder<AxumChannel> for Axum {
    async fn build_channel(&mut self) -> ActorResult<AxumChannel> {
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
impl<S: SupHandle<Self>> Actor<S> for Axum {
    type Data = String;
    type Channel = AxumChannel;
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
