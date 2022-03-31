use overclock::core::*;

////////////////// Incrementer ///////////
use std::sync::{
    atomic::{AtomicIsize, Ordering},
    Arc,
};

struct Incrementer;

#[async_trait::async_trait]
impl<S> Actor<S> for Incrementer
where
    S: SupHandle<Self>,
{
    type Data = Arc<AtomicIsize>;
    type Channel = IntervalChannel<1000>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!(
            "scope_id: {}, {} is {}",
            rt.scope_id(),
            rt.service().actor_type_name(),
            rt.service().status(),
        );
        // create atomic resource, and publish it
        let counter = Arc::new(AtomicIsize::new(0));
        rt.add_resource(counter.clone()).await;
        Ok(counter)
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, counter: Self::Data) -> ActorResult<()> {
        while let Some(_instant) = rt.inbox_mut().next().await {
            // increment the counter
            let old_counter = counter.fetch_add(1, Ordering::Relaxed);
            log::info!("Counter in Incrementer is {}", old_counter);
        }
        Ok(())
    }
}
//////////////// Decrementer ////////////

struct Decrementer;

#[async_trait::async_trait]
impl<S> Actor<S> for Decrementer
where
    S: SupHandle<Self>,
{
    type Data = Arc<AtomicIsize>;
    type Channel = IntervalChannel<1000>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!(
            "scope_id: {}, {} is {}",
            rt.scope_id(),
            rt.service().actor_type_name(),
            rt.service().status()
        );
        // link to the atomic resource under the following scope_id
        if let Some(resource_scope_id) = rt.highest_scope_id::<Self::Data>().await {
            let counter = rt
                .link::<Self::Data>(resource_scope_id, true)
                .await
                .map_err(|e| {
                    log::error!("{:?}", e);
                    ActorError::exit_msg(e)
                })?;
            Ok(counter)
        } else {
            Err(ActorError::exit_msg(
                "Unable to find scope for Arc<AtomicIsize> data",
            ))
        }
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, counter: Self::Data) -> ActorResult<()> {
        while let Some(_instant) = rt.inbox_mut().next().await {
            // decrement the counter
            let old_counter = counter.fetch_sub(1, Ordering::Relaxed);
            log::info!("Counter in Decrementer is {}", old_counter);
        }
        Ok(())
    }
}

// The root custome actor, equivalent to a launcher;
struct Overclock;
enum OverclockEvent {
    Shutdown,
    Microservice(ScopeId, Service),
}

///// All of these should be implemented using proc_macro or some macro. start //////
impl ShutdownEvent for OverclockEvent {
    fn shutdown_event() -> Self {
        Self::Shutdown
    }
}
impl<T> ReportEvent<T> for OverclockEvent {
    fn report_event(scope_id: ScopeId, service: Service) -> Self {
        Self::Microservice(scope_id, service)
    }
}
impl<T> EolEvent<T> for OverclockEvent {
    fn eol_event(scope_id: ScopeId, service: Service, _actor: T, _r: ActorResult<()>) -> Self {
        Self::Microservice(scope_id, service)
    }
}
///// All of these should be implemented using proc_macro or some macro end ///////

#[async_trait::async_trait]
impl<S> Actor<S> for Overclock
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = UnboundedChannel<OverclockEvent>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!("overclock: {}", rt.service().status());
        // build and spawn your apps actors using the rt
        // - build Incrementer
        let incrementer = Incrementer;
        // spawn incrementer
        let (_h, i) = rt
            .spawn(Some("incrementer".into()), incrementer)
            .await
            .map_err(|e| {
                log::error!("{:?}", e);
                ActorError::exit_msg(format!("{:?}", e))
            })?;
        // await incrementer till it gets initialized
        i.initialized().await?;
        // - build Decrementer
        let decrementer = Decrementer;
        // spawn decrementer
        rt.spawn(Some("decrementer".into()), decrementer)
            .await
            .map_err(|e| {
                log::error!("{:?}", e);
                ActorError::exit_msg(format!("{:?}", e))
            })?;
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _deps: Self::Data) -> ActorResult<()> {
        log::info!("overclock: {}", rt.service().status());
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                OverclockEvent::Shutdown => {
                    rt.stop().await;
                    log::info!("overclock got shutdown signal");
                    if rt.microservices_stopped() {
                        rt.inbox_mut().close();
                    }
                }
                OverclockEvent::Microservice(scope_id, service) => {
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
    runtime
        .block_on()
        .await
        .expect("Runtime to shutdown gracefully");
}
