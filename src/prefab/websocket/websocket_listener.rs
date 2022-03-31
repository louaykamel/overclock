// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

use super::WebsocketEvent;
use crate::core::*;

pub struct WebsocketListener {
    addr: std::net::SocketAddr,
    ttl: Option<u32>,
}

impl WebsocketListener {
    /// Create new WebsocketListener struct
    pub(crate) fn new(addr: std::net::SocketAddr, ttl: Option<u32>) -> Self {
        Self { addr, ttl }
    }
}

#[async_trait::async_trait]
impl ChannelBuilder<TcpListenerStream> for WebsocketListener {
    async fn build_channel(&mut self) -> ActorResult<TcpListenerStream> {
        let listener = TcpListener::bind(self.addr).await.map_err(|e| {
            log::error!("{}", e);
            ActorError::exit_msg(e)
        })?;
        if let Some(ttl) = self.ttl.as_ref() {
            listener.set_ttl(*ttl).map_err(|e| {
                log::error!("{}", e);
                ActorError::exit_msg(e)
            })?;
        }
        Ok(TcpListenerStream::new(listener))
    }
}

#[async_trait::async_trait]
impl Actor<UnboundedHandle<WebsocketEvent>> for WebsocketListener {
    type Data = ();
    type Channel = TcpListenerStream;
    async fn init(&mut self, _rt: &mut Rt<Self, UnboundedHandle<WebsocketEvent>>) -> ActorResult<Self::Data> {
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, UnboundedHandle<WebsocketEvent>>, _data: Self::Data) -> ActorResult<()> {
        while let Some(r) = rt.inbox_mut().next().await {
            if let Ok(stream) = r {
                rt.supervisor_handle().send(WebsocketEvent::TcpStream(stream)).ok();
            }
        }
        // this will link the supervisor to the listener
        rt.supervisor_handle().shutdown().await;
        Ok(())
    }
}
