// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::core::*;
use futures::{sink::Sink, SinkExt};
/// The websocket sender actor, manages the sink half of the client.
pub struct WebsocketSender<T>
where
    T: Sink<Message>,
{
    split_sink: T,
}

impl<T> WebsocketSender<T>
where
    T: Sink<Message> + Send + Sync + 'static,
{
    pub(crate) fn new(split_sink: T) -> Self {
        Self { split_sink }
    }
}
#[derive(Debug, Clone)]
pub enum WebsocketSenderEvent {
    Shutdown,
    Result(ResponseResult),
    Subscribe(ActorPath, ScopeId, JsonMessage),
    Event(Event<JsonMessage>),
}
impl ShutdownEvent for WebsocketSenderEvent {
    fn shutdown_event() -> Self {
        Self::Shutdown
    }
}

impl From<Event<JsonMessage>> for WebsocketSenderEvent {
    fn from(event: crate::core::Event<super::JsonMessage>) -> Self {
        Self::Event(event)
    }
}

#[async_trait::async_trait]
impl<S, T> Actor<S> for WebsocketSender<T>
where
    S: SupHandle<Self>,
    T: Sink<Message> + Send + 'static + Sync + Unpin,
{
    type Data = ();
    type Channel = UnboundedChannel<WebsocketSenderEvent>;
    async fn init(&mut self, _rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                WebsocketSenderEvent::Shutdown => break,
                WebsocketSenderEvent::Event(event) => {
                    let json_event: JsonEvent = event.into();
                    let json = serde_json::to_string(&json_event).expect("Serializable json event");
                    let message = Message::from(json);
                    self.split_sink.send(message).await.ok();
                }
                WebsocketSenderEvent::Result(r) => match r {
                    Ok(response) => {
                        let json = serde_json::to_string(&response).expect("Serializable response");
                        let message = Message::from(json);
                        self.split_sink.send(message).await.ok();
                    }
                    Err(error) => {
                        let json = serde_json::to_string(&error).expect("Serializable response");
                        let message = Message::from(json);
                        self.split_sink.send(message).await.ok();
                    }
                },
                WebsocketSenderEvent::Subscribe(actor_path, resource_scope_id, resource_ref) => {
                    match rt
                        .subscribe::<JsonMessage>(resource_scope_id, resource_ref.clone().into())
                        .await
                    {
                        Err(e) => {
                            let json =
                                serde_json::to_string(&Error::Subscribe(actor_path, resource_ref, format!("{}", e)))
                                    .expect("Serializable response");
                            let message = Message::from(json);
                            self.split_sink.send(message).await.ok();
                        }
                        Ok(mut o) => {
                            let json = serde_json::to_string(&Response::Subscribed(resource_ref.clone().into()))
                                .expect("Serializable response");
                            let message = Message::from(json);
                            self.split_sink.send(message).await.ok();
                            if let Some(ring) = o.take() {
                                let json_event = JsonEvent::Published(resource_scope_id, resource_ref.0, ring);
                                let json = serde_json::to_string(&Response::JsonEvent(json_event))
                                    .expect("Serializable response");
                                let message = Message::from(json);
                                self.split_sink.send(message).await.ok();
                            }
                        }
                    }
                }
            }
        }
        Ok(())
    }
}
