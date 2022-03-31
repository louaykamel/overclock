use overclock::core::*;

#[derive(Debug, Serialize, Deserialize, PartialEq, Eq, Default, Clone)]
struct Root {
    config_field: String,
}

#[async_trait::async_trait]
impl<S> Actor<S> for Root
where
    S: SupHandle<Self>,
{
    type Data = ();
    type Channel = AbortableUnboundedChannel<String>;
    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        log::info!("Root: {}", rt.service().status());
        Ok(())
    }
    async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
        log::info!("Root: {}", rt.service().status());
        self.config_field = "update1".to_string();
        rt.publish(self.clone()).await;
        while let Some(event) = rt.inbox_mut().next().await {
            log::info!("Root: Received {}", event);
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
    let runtime = Runtime::from_config::<Root>()
        .await
        .expect("Runtime to run");
    runtime
        .block_on()
        .await
        .expect("Runtime to shutdown gracefully");
}
