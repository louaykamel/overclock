// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use overclock::core::{
    AbortableUnboundedChannel, Actor, ActorResult, Rt, Runtime, ScopeId, Service, ServiceEvent, Shutdown, StreamExt,
    SupHandle,
};
use std::sync::{atomic::AtomicU32, Arc};

struct Spawner;

enum SpawnerEvent {
    Spawn,
    Microservice(ScopeId, Service),
}

impl<T> ServiceEvent<T> for SpawnerEvent {
    fn report_event(scope_id: ScopeId, service: Service) -> Self {
        Self::Microservice(scope_id, service)
    }
    fn eol_event(scope_id: ScopeId, service: Service, _actor: T, _r: ActorResult<()>) -> Self {
        Self::Microservice(scope_id, service)
    }
}

#[async_trait::async_trait]
impl<S> Actor<S> for Spawner
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = AbortableUnboundedChannel<SpawnerEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<()> {
        // stops spawning children at the 11th level
        let data: Arc<AtomicU32> = rt.lookup(0).await.unwrap();
        data.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                SpawnerEvent::Spawn => {
                    if rt.depth() < 11 {
                        rt.spawn(None, Launcher).await?;
                    } else {
                        rt.handle().shutdown().await;
                    }
                }
                SpawnerEvent::Microservice(scope_id, service) => {
                    if service.is_stopped() {
                        rt.remove_microservice(scope_id);
                        if rt.microservices_stopped() {
                            rt.inbox_mut().close();
                        }
                    } else {
                        rt.upsert_microservice(scope_id, service);
                    }
                }
            }
        }
        Ok(())
    }
}

//////// Our root runtime actor ////////
struct Launcher;

enum LauncherEvent {
    Microservice(ScopeId, Service),
}

impl<T> ServiceEvent<T> for LauncherEvent {
    fn report_event(scope_id: ScopeId, service: Service) -> Self {
        Self::Microservice(scope_id, service)
    }
    fn eol_event(scope_id: ScopeId, service: Service, _actor: T, _r: ActorResult<()>) -> Self {
        Self::Microservice(scope_id, service)
    }
}

#[async_trait::async_trait]
impl<S> Actor<S> for Launcher
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = AbortableUnboundedChannel<LauncherEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        let total_spawned_actors: Arc<AtomicU32> = rt.lookup(0).await.unwrap();
        total_spawned_actors.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        for _ in 0..10 {
            let (handle, _) = rt.spawn(None, Spawner).await?;
            handle.send(SpawnerEvent::Spawn).ok();
        }

        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                LauncherEvent::Microservice(scope_id, service) => {
                    if service.is_stopped() {
                        rt.remove_microservice(scope_id);
                    } else {
                        rt.upsert_microservice(scope_id, service);
                    }
                    // stop the runtime test if all children are offline
                    if rt.microservices_stopped() {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}

//////// Our root runtime actor ////////
struct Root;

enum RootEvent {
    Microservice(ScopeId, Service),
}

impl<T> ServiceEvent<T> for RootEvent {
    fn report_event(scope_id: ScopeId, service: Service) -> Self {
        Self::Microservice(scope_id, service)
    }

    fn eol_event(scope_id: ScopeId, service: Service, _actor: T, _r: ActorResult<()>) -> Self {
        Self::Microservice(scope_id, service)
    }
}

#[async_trait::async_trait]
impl<S> Actor<S> for Root
where
    S: SupHandle<Self>,
{
    type Data = Arc<AtomicU32>;
    type Channel = AbortableUnboundedChannel<LauncherEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        let total_spawned_actors = std::sync::Arc::new(std::sync::atomic::AtomicU32::new(0));
        rt.add_resource(total_spawned_actors.clone()).await;
        rt.spawn(None, Launcher).await?;
        Ok(total_spawned_actors)
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, total_spawned_actors: Self::Data) -> ActorResult<()> {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                LauncherEvent::Microservice(scope_id, service) => {
                    if service.is_stopped() {
                        rt.remove_microservice(scope_id);
                    } else {
                        rt.upsert_microservice(scope_id, service);
                    }
                    // stop the runtime test if all children are offline
                    if rt.microservices_stopped() {
                        break;
                    }
                }
            }
        }
        log::info!(
            "Total actors spawned: {}",
            total_spawned_actors.load(std::sync::atomic::Ordering::Relaxed)
        );
        Ok(())
    }
}

#[tokio::main]
async fn main() {
    std::env::set_var("RUST_LOG", "info");
    std::env::set_var("OVERCLOCK_PARTITIONS", "20");
    env_logger::init();
    let start = std::time::SystemTime::now();
    let runtime = Runtime::new(None, Root).await.expect("Root to run");
    runtime.block_on().await.expect("Root to gracefully shutdown");
    log::info!("Total time: {} ms", start.elapsed().unwrap().as_millis());
}
