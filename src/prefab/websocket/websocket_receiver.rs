// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::{Event, *};
use crate::core::*;
use futures::stream::{Stream, StreamExt};
use tokio_tungstenite::tungstenite::Error as WsError;
/// The websocket receiver actor, manages the Stream from the client
pub struct WebsocketReceiver<T>
where
    T: Send + 'static + Sync + Stream<Item = Result<Message, WsError>>,
{
    sender_handle: UnboundedHandle<WebsocketSenderEvent>,
    split_stream: Option<T>,
}

impl<T> WebsocketReceiver<T>
where
    T: Send + 'static + Sync + Stream<Item = Result<Message, WsError>>,
{
    /// Create new WebsocketReceiver struct
    pub fn new(split_stream: T, sender_handle: UnboundedHandle<WebsocketSenderEvent>) -> Self {
        Self {
            sender_handle,
            split_stream: Some(split_stream),
        }
    }
}

#[async_trait::async_trait]
impl<T> ChannelBuilder<IoChannel<T>> for WebsocketReceiver<T>
where
    T: Send + 'static + Sync + Stream<Item = Result<Message, WsError>>,
{
    async fn build_channel(&mut self) -> ActorResult<IoChannel<T>> {
        if let Some(stream) = self.split_stream.take() {
            Ok(IoChannel(stream))
        } else {
            Err(ActorError::exit_msg("Unable to build websocket receiver channel"))
        }
    }
}

#[async_trait::async_trait]
impl<S, T> Actor<S> for WebsocketReceiver<T>
where
    S: SupHandle<Self>,
    T: Send + 'static + Sync + Stream<Item = Result<Message, WsError>> + Unpin,
{
    type Data = ();
    type Channel = IoChannel<T>;
    async fn init(&mut self, _rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        while let Some(Ok(message)) = rt.inbox_mut().next().await {
            // Deserialize message::text
            match message {
                Message::Text(text) => {
                    if let Ok(interface) = serde_json::from_str::<Interface>(&text) {
                        let mut targeted_scope_id_opt = interface.actor_path.destination().await;
                        match interface.event {
                            Event::Shutdown => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    if let Err(err) = rt.shutdown_scope(scope_id).await {
                                        let err_string = err.to_string();
                                        let r = Error::Shutdown(interface.actor_path, err_string);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                    } else {
                                        let r = Response::Shutdown(interface.actor_path);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Ok(r))).ok();
                                    };
                                } else {
                                    let err_string = "Unreachable ActorPath".to_string();
                                    let r = Error::Shutdown(interface.actor_path, err_string);
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                            Event::RequestServiceTree => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    if let Some(service) = rt.lookup::<Service>(scope_id).await {
                                        let r = Response::ServiceTree(service);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Ok(r))).ok();
                                    } else {
                                        let r = Error::ServiceTree("Service not available".into());
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                    };
                                } else {
                                    let r = Error::ServiceTree("Unreachable ActorPath".into());
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                            Event::Cast(message_to_route) => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    let route_message = message_to_route.clone();
                                    match rt.send(scope_id, message_to_route).await {
                                        Ok(()) => {
                                            let r = Response::Sent(route_message);
                                            self.sender_handle.send(WebsocketSenderEvent::Result(Ok(r))).ok();
                                        }
                                        Err(e) => {
                                            let err = format!("{}", e);
                                            let r = Error::Cast(interface.actor_path, route_message, err);
                                            self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                        }
                                    };
                                } else {
                                    let r = Error::Cast(
                                        interface.actor_path,
                                        message_to_route,
                                        "Unreachable ActorPath".into(),
                                    );
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                            Event::Call(message_to_route) => {
                                if let Some(scope_id) = targeted_scope_id_opt.take() {
                                    let json_message = message_to_route.clone();
                                    // create responder
                                    let ws_handle = self.sender_handle.clone();
                                    let responder = JsonResponder::trait_obj(ws_handle);
                                    if let Err(e) = rt.send(scope_id, (json_message, responder)).await {
                                        let err = format!("{}", e);
                                        let r = Error::Call(interface.actor_path, message_to_route, err);
                                        self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                    };
                                } else {
                                    let r = Error::Call(
                                        interface.actor_path,
                                        message_to_route,
                                        "Unreachable ActorPath".into(),
                                    );
                                    self.sender_handle.send(WebsocketSenderEvent::Result(Err(r))).ok();
                                }
                            }
                        }
                    } else {
                        break;
                    };
                }

                Message::Close(_) => {
                    break;
                }
                _ => {}
            }
        }
        self.sender_handle.shutdown().await;
        Ok(())
    }
}
