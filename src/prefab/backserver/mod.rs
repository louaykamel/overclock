/// hyper server as make service;
mod server;

use super::websocket::{
    WebsocketReceiver,
    WebsocketSender,
};
use crate::core::*;
use hyper_tungstenite::HyperWebsocket;

/// Backserver supervisor, enables websocket & http servers
/// and manages all the active websocket connections
pub struct Backserver {
    addr: std::net::SocketAddr,
    root_scope_id: ScopeId,
    link_to: Option<Box<dyn Shutdown>>,
}

impl Backserver {
    /// Create new Websocket struct
    pub fn new(addr: std::net::SocketAddr, root_scope_id: ScopeId) -> Self {
        Self {
            addr,
            link_to: None,
            root_scope_id,
        }
    }
    /// Link handle to the websocket
    pub fn link_to(mut self, handle: Box<dyn Shutdown>) -> Self {
        self.link_to.replace(handle);
        self
    }
}

/// Backserver Event type
pub enum BackserverEvent {
    /// Shutdown signal
    Shutdown,
    /// Microservices reports and eol
    Microservice(ScopeId, Service, Option<ActorResult<()>>),
    /// New HyperWebsocket from listener
    HyperWebsocket(HyperWebsocket),
}

impl<T> ServiceEvent<T> for BackserverEvent {
    fn eol_event(scope_id: ScopeId, service: Service, _actor: T, r: ActorResult<()>) -> Self {
        Self::Microservice(scope_id, service, Some(r))
    }

    fn report_event(scope_id: ScopeId, service: Service) -> Self {
        Self::Microservice(scope_id, service, None)
    }
}


impl ShutdownEvent for BackserverEvent {
    fn shutdown_event() -> Self {
        Self::Shutdown
    }
}

#[async_trait::async_trait]
impl<S: SupHandle<Self>> Actor<S> for Backserver {
    type Data = ();
    type Channel = UnboundedChannel<BackserverEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        // spawn backserver listener using prefab hyper
        let my_handle = rt.handle().clone();
        let make_svc = server::MakeListenerSvc::new(self.root_scope_id, my_handle);
        let addr = self.addr;
        let hyper = super::hyper::Hyper::new(addr, make_svc);
        rt.start("server".to_string(), hyper).await?;
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()>
    where
        S: SupHandle<Self>,
    {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                BackserverEvent::Shutdown => {
                    rt.stop().await;
                    if rt.microservices_stopped() {
                        break;
                    }
                }
                BackserverEvent::HyperWebsocket(hyper_ws) => {
                    if let Ok(ws_stream) = hyper_ws.await {
                        let (split_sink, split_stream) = ws_stream.split();
                        let sender = WebsocketSender::new(split_sink);
                        if let Ok((sender_handle, _)) = rt.spawn(None, sender).await {
                            let receiver = WebsocketReceiver::new(split_stream, sender_handle);
                            rt.spawn(None, receiver).await.ok();
                        }
                    }
                }
                BackserverEvent::Microservice(scope_id, service, _r_opt) => {
                    if service.is_stopped() {
                        rt.remove_microservice(scope_id);
                    } else {
                        rt.upsert_microservice(scope_id, service);
                    }
                    if rt.microservices_stopped() {
                        break;
                    }
                }
            }
        }
        if let Some(root_handle) = self.link_to.take() {
            root_handle.shutdown().await
        }
        Ok(())
    }
}
