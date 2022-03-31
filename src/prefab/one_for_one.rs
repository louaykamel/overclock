// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use crate::core::*;
use std::collections::HashMap;
/// One for one supervisor
pub struct OneForOne<S> {
    pub(crate) children: HashMap<String, Box<dyn Child<OneForOne<S>, S>>>,
    pub(crate) start_order: Vec<String>,
}

impl<S: Supervise<OneForOne<S>> + Sync + Send> OneForOne<S> {
    /// Create new one_for_one for supervisor
    pub fn new() -> Self {
        Self {
            children: HashMap::new(),
            start_order: Vec::new(),
        }
    }
    /// Add lazy child
    pub fn add<T: Child<Self, S> + 'static>(mut self, name: String, child: T) -> Self {
        let boxed_child: Box<dyn Child<Self, S>> = Box::new(child);
        let actor = boxed_child;
        self.children.insert(name.clone(), actor);
        self.start_order.push(name);
        self
    }
}

#[async_trait::async_trait]
pub trait Child<A: Actor, S: Supervise<A>>: Send + Sync + dyn_clone::DynClone {
    async fn spawn(self: Box<Self>, rt: &mut A::Context<S>) -> Result<(), Reason>;
}

dyn_clone::clone_trait_object!(<A, S> Child<A, S>);

pub enum OneForOneEvent<S> {
    /// Shutdown everything
    Shutdown,
    /// (scope_id, service)
    Report(ScopeId, Service),
    Eol(Box<dyn HandleEol<S>>),
}
#[async_trait::async_trait]
pub trait HandleEol<S>: Send
where
    S: Supervise<OneForOne<S>>,
{
    async fn handle_eol(self: Box<Self>, state: &mut OneForOne<S>, rt: &mut Rt<OneForOne<S>, S>);
}

#[derive(Clone)]
pub struct AddChild<S, T: Actor + Clone>
where
    S: Supervise<OneForOne<S>>,
    <<OneForOne<S> as Actor>::Channel as Channel>::Handle: Supervise<T>,
{
    pub(crate) actor: T,
    pub(crate) name: String,
    pub(crate) ensure_initialized: bool,
    _marker: std::marker::PhantomData<S>,
}
#[async_trait::async_trait]
impl<S: Sync + Clone, T: Actor + Clone> Child<OneForOne<S>, S> for AddChild<S, T>
where
    S: Supervise<OneForOne<S>> + Sync + Send,
    T: Actor<Context<<<OneForOne<S> as Actor>::Channel as Channel>::Handle> = Rt<T, <<OneForOne<S> as Actor>::Channel as Channel>::Handle>>
        + ChannelBuilder<T::Channel>,
    <<OneForOne<S> as Actor>::Channel as Channel>::Handle: Supervise<T>,
{
    async fn spawn(self: Box<Self>, rt: &mut <OneForOne<S> as Actor>::Context<S>) -> Result<(), Reason> {
        let name = self.name;
        let child = self.actor;
        match rt.spawn(name, child).await {
            Ok((_handle, init_rx)) => {
                if self.ensure_initialized {
                    init_rx.initialized().await?
                }
            }
            Err(reason) => return Err(reason),
        };
        Ok(())
    }
}

impl<S> ShutdownEvent for OneForOneEvent<S> {
    fn shutdown_event() -> Self {
        Self::Shutdown
    }
}
impl<S, T: Actor> EolEvent<T> for OneForOneEvent<S>
where
    S: Supervise<OneForOne<S>>,
{
    fn eol_event(scope: ScopeId, service: Service, _actor: T, r: ActorResult) -> Self {
        let handle_eol = HandleChildEol(scope, service, r);
        Self::Eol(Box::new(handle_eol))
    }
}
struct HandleChildEol(ScopeId, Service, ActorResult);

#[async_trait::async_trait]
impl<S> HandleEol<S> for HandleChildEol
where
    S: Supervise<OneForOne<S>>,
{
    async fn handle_eol(self: Box<Self>, state: &mut OneForOne<S>, rt: &mut Rt<OneForOne<S>, S>) {
        let scope_id = self.0;
        let service = self.1;
        let name = service.name.clone();
        let r = self.2;
        // rt.handle_microservice(scope_id, service, Some(r.clone())).await;
        // handle strategy
        if let Err(reason) = r {
            match reason {
                Reason::Restart(mut opt_dur) => {
                    if let Some(dur) = opt_dur.take() {
                        todo!("support restart after duration")
                    } else {
                        if let Some(child) = state.children.get(&name) {
                            let _ = child.clone().spawn(rt).await;
                        }
                    }
                }
                Reason::Exit => {
                    log::error!("scope_id: {}, service_name: {}, exit", scope_id, name);
                }
            }
        }
    }
}
#[async_trait::async_trait]
impl<Sup: 'static + Supervise<Self>> Actor for OneForOne<Sup> {
    type Context<S: Supervise<Self>> = Rt<Self, Sup>;
    type Channel = UnboundedChannel<OneForOneEvent<Sup>>;
    async fn init<S: Supervise<Self>>(&mut self, rt: &mut Self::Context<S>) -> Result<Self::Data, Reason> {
        for name in self.start_order.iter() {
            let child = self.children.get(name).expect("Child to exist");
            child.clone().spawn(rt).await?;
        }
        Ok(())
    }
    async fn run<S: Supervise<Self>>(&mut self, rt: &mut Self::Context<S>, _data: Self::Data) -> ActorResult {
        while let Some(event) = rt.inbox_mut().next().await {
            match event {
                OneForOneEvent::Shutdown => {
                    // stop service
                    rt.stop().await;
                    if rt.microservices_stopped() {
                        break;
                    }
                }
                OneForOneEvent::Report(scope_id, service) => {
                    // rt.handle_microservice(scope_id, service, None).await;
                }
                OneForOneEvent::Eol(eol) => {
                    eol.handle_eol(self, rt).await;
                    if rt.microservices_stopped() {
                        break;
                    }
                }
            }
        }
        Ok(())
    }
}
