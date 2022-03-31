// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use overclock::core::*;

////////////////// First ///////////

struct First;

#[async_trait::async_trait]
impl<S> Actor<S> for First
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = AbortableUnboundedChannel<String>;
    async fn init(&mut self, _rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        while let Some(event) = rt.inbox_mut().next().await {
            log::info!("First received: {}", event);
            if let Some(second_scope_id) = rt.sibling("second").scope_id().await {
                rt.send(second_scope_id, "Hey second".to_string()).await.ok();
            }
        }
        Ok(())
    }
}
//////////////// Second ////////////

struct Second;

#[async_trait::async_trait]
impl<S> Actor<S> for Second
where
    S: SupHandle<Self>,
{
    type Data = ScopeId;
    type Channel = AbortableUnboundedChannel<String>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        if let Some(first_scope_id) = rt.sibling("first").scope_id().await {
            return Ok(first_scope_id);
        };
        Err(ActorError::exit_msg("Unable to get first scope id"))
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, first_scope_id: Self::Data) -> ActorResult<()> {
        rt.send(first_scope_id, "Hey first".to_string()).await.ok();
        while let Some(event) = rt.inbox_mut().next().await {
            log::info!("Second received: {}", event);
        }
        Ok(())
    }
}
// The root custom actor, equivalent to a launcher;
struct Overclock;

#[supervise]
enum OverclockEvent {
    Shutdown,
    Report(ScopeId, Service),
    Exit(ScopeId, Service, OverclockChild),
}

#[children]
enum OverclockChild {
    First(First),
    Second(Second),
}

// ### Impl using #[children] proc macro ### //
impl From<First> for OverclockChild {
    fn from(f: First) -> Self {
        Self::First(f)
    }
}

impl From<Second> for OverclockChild {
    fn from(s: Second) -> Self {
        Self::Second(s)
    }
}
// ### Impl using #[children] proc macro ### //

#[async_trait::async_trait]
impl<S> Actor<S> for Overclock
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = UnboundedChannel<OverclockEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!("Overclock: {}", rt.service().status());
        // build and spawn your apps actors using the rt
        // - build First
        let first = First;
        // start first
        rt.start(Some("first".into()), first).await?;
        // - build Second
        let second = Second;
        // start second
        rt.start(Some("second".into()), second).await?;
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _deps: Self::Data) -> ActorResult<()> {
        log::info!("Overclock: {}", rt.service().status());
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                OverclockEvent::Shutdown => {
                    rt.stop().await;
                    log::info!("overclock got shutdown signal");
                    if rt.microservices_stopped() {
                        rt.inbox_mut().close();
                    }
                }
                OverclockEvent::Report(scope_id, service) => {
                    log::info!(
                        "Microservice: {}, dir: {:?}, status: {}",
                        service.actor_type_name(),
                        service.directory(),
                        service.status()
                    );
                    rt.upsert_microservice(scope_id, service);
                }
                OverclockEvent::Exit(scope_id, service, _child) => {
                    log::info!(
                        "Microservice: {}, dir: {:?}, status: {}",
                        service.actor_type_name(),
                        service.directory(),
                        service.status()
                    );
                    rt.upsert_microservice(scope_id, service);
                    if rt.microservices_stopped() {
                        rt.inbox_mut().close();
                    }
                }
            }
        }
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
    let overclock = Overclock;
    let websocket_server_addr = "127.0.0.1:9000"
        .parse::<std::net::SocketAddr>()
        .expect("parsable socket addr");
    let runtime = Runtime::new(Some("overclock".into()), overclock)
        .await
        .expect("Runtime to build")
        .websocket_server(websocket_server_addr, None)
        .await
        .expect("Websocket server to run");
    runtime.block_on().await.expect("Runtime to shutdown gracefully");
}
