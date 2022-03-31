// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use overclock::core::*;
#[cfg(feature = "websocket_server")]
use overclock::prefab::websocket::{GenericResponder, JsonMessage, Responder};

struct Echo;

enum EchoEvent {
    ClientMsg(String, Responder),
}

#[async_trait::async_trait]
impl<S> Actor<S> for Echo
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = AbortableUnboundedChannel<EchoEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        rt.add_route::<(JsonMessage, Responder)>().await.ok();
        log::info!("Echo: {}", rt.service().status());
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        log::info!("Echo: {}", rt.service().status());
        while let Some(EchoEvent::ClientMsg(msg, responder)) = rt.inbox_mut().next().await {
            log::info!("EchoEvent: Received {}", msg);
            responder.inner_reply(msg).await.ok();
        }
        rt.stop().await;
        log::info!("Echo: {}", rt.service().status());
        Ok(())
    }
}

impl std::convert::TryFrom<(JsonMessage, Responder)> for EchoEvent {
    type Error = std::convert::Infallible;
    fn try_from(value: (JsonMessage, Responder)) -> Result<Self, Self::Error> {
        Ok(EchoEvent::ClientMsg(value.0 .0, value.1))
    }
}

#[tokio::main]
async fn main() {
    #[cfg(not(feature = "console"))]
    {
        let env = env_logger::Env::new().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }
    let hello_world = Echo;
    let server_addr = "127.0.0.1:9000"
        .parse::<std::net::SocketAddr>()
        .expect("parsable socket addr");
    let runtime = Runtime::new(Some("echo".into()), hello_world)
        .await
        .expect("Runtime to run")
        .websocket_server(server_addr, None)
        .await
        .expect("Websocket server to run");
    overclock::spawn_task("overclock websocket", ws_client());
    runtime.block_on().await.expect("Runtime to shutdown gracefully");
}

async fn ws_client() {
    use futures::SinkExt;
    use overclock::prefab::websocket::*;
    let (mut stream, _) = tokio_tungstenite::connect_async(url::Url::parse("ws://127.0.0.1:9000/").unwrap())
        .await
        .unwrap();
    let actor_path = ActorPath::new();
    let request = Interface::new(actor_path.clone(), Event::Call("Call message".into()));
    stream.send(request.to_message()).await.unwrap();
    while let Some(Ok(msg)) = stream.next().await {
        log::info!("Response from websocket: {}", msg);
        let request = Interface::new(ActorPath::new(), Event::shutdown());
        stream.send(request.to_message()).await.ok();
    }
}
