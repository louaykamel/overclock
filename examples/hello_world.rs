// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use overclock::core::*;
#[cfg(feature = "websocket_server")]
use overclock::prefab::websocket::JsonMessage;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
struct HelloWorld;

#[async_trait::async_trait]
impl<S> Actor<S> for HelloWorld
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = AbortableUnboundedChannel<String>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        rt.add_route::<JsonMessage>().await.ok();
        log::info!("HelloWorld: {}", rt.service().status());
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        log::info!("HelloWorld: {}", rt.service().status());
        while let Some(event) = rt.inbox_mut().next().await {
            log::info!("HelloWorld: Received {}", event);
        }
        rt.stop().await;
        log::info!("HelloWorld: {}", rt.service().status());
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    #[cfg(not(feature = "console"))]
    {
        let env = env_logger::Env::new().filter_or("RUST_LOG", "info");
        env_logger::Builder::from_env(env).init();
    }
    let hello_world = HelloWorld;
    let server_addr = "127.0.0.1:9000"
        .parse::<std::net::SocketAddr>()
        .expect("parsable socket addr");
    let runtime = Runtime::new(Some("HelloWorld".into()), hello_world)
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
    let request = Interface::new(actor_path.clone(), Event::cast("Print this".into()));
    stream.send(request.to_message()).await.unwrap();
    while let Some(Ok(msg)) = stream.next().await {
        log::info!("Response from websocket: {}", msg);
        let request = Interface::new(ActorPath::new(), Event::shutdown());
        stream.send(request.to_message()).await.ok();
    }
}
