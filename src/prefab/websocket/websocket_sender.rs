use super::*;
use crate::core::*;
use futures::{
    sink::Sink,
    SinkExt,
};
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
}
impl ShutdownEvent for WebsocketSenderEvent {
    fn shutdown_event() -> Self {
        Self::Shutdown
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
            }
        }
        Ok(())
    }
}
