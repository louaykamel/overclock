// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::{
    AbortRegistration, Abortable, Actor, ActorError, ActorResult, Channel, ChannelBuilder, Cleanup, CleanupData, Data,
    NullSupervisor, Resource, Route, Scope, ScopeId, Service, ServiceStatus, Shutdown, Subscriber, SupHandle,
    OVERCLOCK_PARTITIONS, SCOPES, SCOPE_ID_RANGE, VISIBLE_DATA,
};
#[cfg(feature = "config")]
use crate::config::*;
use rand::{distributions::Distribution, thread_rng};
#[cfg(feature = "config")]
use serde::{de::DeserializeOwned, Deserializer, Serialize};

#[cfg(all(feature = "prefabs", feature = "tungstenite"))]
use crate::prefab::websocket::Websocket;

use prometheus::core::Collector;
use std::collections::HashMap;

/// The actor's local context
pub struct Rt<A: Actor<S>, S: Send> {
    /// The level depth of the context, it's the scope depth from the root
    pub(crate) depth: usize,
    /// The service of the actor
    pub(crate) service: Service,
    /// The PartitionedScope's position index in the SCOPES
    /// That owns the actor's scope.
    pub(crate) scopes_index: usize,
    /// ScopeId of the actor
    pub(crate) scope_id: ScopeId,
    /// the parent's scope id
    pub(crate) parent_scope_id: Option<ScopeId>,
    /// The supervisor handle
    pub(crate) sup_handle: S,
    /// The actor's handle
    pub(crate) handle: <A::Channel as Channel>::Handle,
    /// The actor's inbox
    pub(crate) inbox: <A::Channel as Channel>::Inbox,
    /// The actor's children handles
    pub(crate) children_handles: std::collections::HashMap<ScopeId, Box<dyn Shutdown>>,
    /// The actor's children joins
    pub(crate) children_joins: std::collections::HashMap<ScopeId, tokio::task::JoinHandle<ActorResult<()>>>,
    /// The actor abort_registration copy
    pub(crate) abort_registration: AbortRegistration,
    /// Registered prometheus metrics
    pub(crate) registered_metrics: Vec<Box<dyn prometheus::core::Collector>>,
    /// Visiable/ exposed global resources
    pub(crate) visible_data: std::collections::HashSet<std::any::TypeId>,
}
/// InitializedRx signal receiver
pub struct InitializedRx(ScopeId, tokio::sync::oneshot::Receiver<ActorResult<Service>>);
type InitSignalTx = tokio::sync::oneshot::Sender<ActorResult<Service>>;

impl InitializedRx {
    /// Await till the actor get initialized
    pub async fn initialized(self) -> ActorResult<(ScopeId, Service)> {
        let service = self.1.await.expect("Expected functional CheckInit oneshot")?;
        Ok((self.0, service))
    }
}
enum Direction {
    Child(String),
    Parent,
}
/// Helper struct, to traverse the supervision tree and locate the scope id
pub struct LocateScopeId {
    start: Option<ScopeId>,
    directory_path: std::collections::VecDeque<Direction>,
}

impl LocateScopeId {
    /// Create new LocateScopeId struct with null start scope_id
    pub fn new() -> Self {
        Self {
            start: None,
            directory_path: std::collections::VecDeque::new(),
        }
    }
    /// Create new LocateScopeId struct with scope id as its start id
    pub fn with_scope_id(start: ScopeId) -> Self {
        Self {
            start: Some(start),
            directory_path: std::collections::VecDeque::new(),
        }
    }
    /// Add child with the provided name to the directory path
    pub fn child<D: Into<String>>(mut self, dir_name: D) -> Self {
        self.directory_path.push_back(Direction::Child(dir_name.into()));
        self
    }
    /// Add parent to the directory path
    pub fn parent(mut self) -> Self {
        self.directory_path.push_back(Direction::Parent);
        self
    }
    /// Add grand parent to the directory path
    pub fn grandparent(self) -> Self {
        self.parent().parent()
    }
    /// Process the directory path and get the scope_id
    pub async fn scope_id(&self) -> Option<ScopeId> {
        if let Some(start) = self.start.as_ref() {
            let mut recent = *start;
            let mut iter = self.directory_path.iter();
            while let Some(dir_name) = iter.next() {
                let scopes_index = recent % *OVERCLOCK_PARTITIONS;
                match dir_name {
                    Direction::Child(dir_name) => {
                        let lock = SCOPES[scopes_index].read().await;
                        if let Some(scope) = lock.get(&recent) {
                            if let Some(scope_id) = scope.active_directories.get(dir_name) {
                                recent = *scope_id;
                                drop(lock);
                                continue;
                            } else {
                                drop(lock);
                                return None;
                            }
                        }
                    }
                    Direction::Parent => {
                        let lock = SCOPES[scopes_index].read().await;
                        if let Some(scope) = lock.get(&recent) {
                            if let Some(scope_id) = scope.parent_id.as_ref() {
                                recent = *scope_id;
                                drop(lock);
                                continue;
                            } else {
                                drop(lock);
                                return None;
                            }
                        }
                    }
                }
            }
            Some(recent)
        } else {
            None
        }
    }
}

impl<A: Actor<S>, S> Rt<A, S>
where
    Self: Send,
    S: Send,
{
    /// Start traversing from the child with the provided dir name
    pub fn child<D: Into<String>>(&self, dir_name: D) -> LocateScopeId {
        let locate = LocateScopeId::with_scope_id(self.scope_id());
        locate.child(dir_name)
    }
    /// Start traversing from the parent
    pub fn parent(&self) -> LocateScopeId {
        if let Some(parent_id) = self.parent_scope_id {
            LocateScopeId::with_scope_id(parent_id)
        } else {
            LocateScopeId::new()
        }
    }
    /// Start traversing from the sibling with the provided dir name
    pub fn sibling<D: Into<String>>(&self, dir_name: D) -> LocateScopeId {
        self.parent().child(dir_name)
    }
    /// Start traversing from the grand parent
    pub fn grandparent(&self) -> LocateScopeId {
        self.parent().parent()
    }
    /// Start traversing from the uncle
    pub fn uncle<D: Into<String>>(&self, dir_name: D) -> LocateScopeId {
        self.grandparent().child(dir_name)
    }
    /// Create abortable future which will be aborted if the actor got shutdown signal.
    pub fn abortable<F>(&self, fut: F) -> Abortable<F>
    where
        F: std::future::Future + Send + Sync,
    {
        let abort_registration = self.abort_registration.clone();
        Abortable::new(fut, abort_registration)
    }
    /// Spawn the provided child and await till it's initialized
    pub async fn start<Dir: Into<Option<String>>, Child>(
        &mut self,
        directory: Dir,
        mut child: Child,
    ) -> ActorResult<<Child::Channel as Channel>::Handle>
    where
        Child: 'static + Actor<<A::Channel as Channel>::Handle> + ChannelBuilder<Child::Channel>,
        <A::Channel as Channel>::Handle: SupHandle<Child>,
        Self: Send,
    {
        // try to create the actor's channel
        let channel = Abortable::new(child.build_channel(), self.abort_registration.clone())
            .await
            .map_err(|_| {
                let msg = format!(
                    "Aborted inside start method while building channel for child: {}",
                    Child::type_name(),
                );
                ActorError::aborted_msg(msg)
            })??;
        self.start_with_channel(directory, child, channel).await
    }
    /// Spawn the child, and returns its handle and initialized rx to check if it got initialized
    pub async fn spawn<Dir: Into<Option<String>>, Child>(
        &mut self,
        directory: Dir,
        mut child: Child,
    ) -> ActorResult<(<Child::Channel as Channel>::Handle, InitializedRx)>
    where
        <A::Channel as Channel>::Handle: Clone,
        Child: 'static
            + ChannelBuilder<<Child as Actor<<A::Channel as Channel>::Handle>>::Channel>
            + Actor<<A::Channel as Channel>::Handle>,
        Child::Channel: Send,
        <Child as Actor<<A::Channel as Channel>::Handle>>::Channel: Send,
        Dir: Into<Option<String>>,
        <A::Channel as Channel>::Handle: SupHandle<Child>,
    {
        // try to create the actor's channel
        let channel = Abortable::new(child.build_channel(), self.abort_registration.clone())
            .await
            .map_err(|_| {
                let msg = format!(
                    "Aborted inside spawn method while building channel for child: {}",
                    Child::type_name(),
                );
                ActorError::aborted_msg(msg)
            })??;
        self.spawn_with_channel(directory, child, channel).await
    }
    /// Spawn the provided child and await till it's initialized
    pub async fn start_with_channel<Dir: Into<Option<String>>, Child>(
        &mut self,
        directory: Dir,
        child: Child,
        channel: Child::Channel,
    ) -> ActorResult<<Child::Channel as Channel>::Handle>
    where
        Child: 'static + Actor<<A::Channel as Channel>::Handle>,
        <A::Channel as Channel>::Handle: SupHandle<Child>,
        Self: Send,
    {
        let (h, init_signal) = self.spawn_with_channel(directory, child, channel).await?;
        if let Ok(Ok((scope_id, service))) = self.abortable(init_signal.initialized()).await {
            self.upsert_microservice(scope_id, service);
            Ok(h)
        } else {
            let msg = format!(
                "Aborted inside start method while awaiting child: {}, to get initialized",
                Child::type_name(),
            );
            Err(ActorError::aborted_msg(msg))
        }
    }
    /// Spawn the child, and returns its handle and initialized rx to check if it got initialized
    pub async fn spawn_with_channel<Dir: Into<Option<String>>, Child>(
        &mut self,
        directory: Dir,
        child: Child,
        channel: Child::Channel,
    ) -> ActorResult<(<Child::Channel as Channel>::Handle, InitializedRx)>
    where
        <A::Channel as Channel>::Handle: Clone,
        Child: 'static + Actor<<A::Channel as Channel>::Handle>,
        Dir: Into<Option<String>>,
        <A::Channel as Channel>::Handle: SupHandle<Child>,
    {
        // try to create the actor's channel
        let parent_id = self.scope_id;
        let mut scopes_index;
        let mut child_scope_id;
        let handle;
        let inbox;
        let abort_registration;
        let mut metric;
        // create the service
        let mut dir = directory.into();
        // check if the dir already reserved by another child
        if dir.is_some()
            && self
                .service
                .microservices
                .iter()
                .any(|(_, ms)| (ms.directory == dir) && !ms.is_stopped())
        {
            let msg = format!(
                "Unable to spawn child: {}, error: directory '{}' already exist",
                Child::type_name(),
                dir.unwrap_or_default(),
            );
            return Err(ActorError::aborted_msg(msg));
        }
        let task_name = dir.clone().unwrap_or_default();
        let mut service = Service::new::<Child, <A::Channel as Channel>::Handle>(dir.clone());
        loop {
            // generate unique scope_id
            child_scope_id = SCOPE_ID_RANGE.sample(&mut thread_rng());
            // check which scopes partition owns the scope_id
            scopes_index = child_scope_id % *OVERCLOCK_PARTITIONS;
            // access the scope
            let mut lock = SCOPES[scopes_index].write().await;
            if lock.contains_key(&child_scope_id) {
                drop(lock);
                continue;
            }
            // finialize the channel
            let mut r = channel.channel::<Child>(child_scope_id);
            handle = r.0;
            inbox = r.1;
            abort_registration = r.2;
            metric = r.3;
            service.update_status(ServiceStatus::Initializing);
            let mut scope = Scope::new(Some(parent_id), Box::new(handle.clone()));
            // add route (if any)
            if let Some(route) = r.4.take() {
                scope.router.insert(route);
            }
            let data = Data::with_resource(service.clone());
            scope.data_and_subscribers.insert(data);
            let data = Data::with_resource(handle.clone());
            scope.data_and_subscribers.insert(data);
            lock.insert(child_scope_id, scope);
            drop(lock);
            break;
        }
        self.service.microservices.insert(child_scope_id, service.clone());
        dir.as_ref().and_then(|dir| {
            self.service
                .inactive
                .remove(dir)
                .and_then(|old_scope_id| self.service.microservices.remove(&old_scope_id))
        });
        if let Some(dir_name) = dir.take() {
            let mut lock = SCOPES[self.scopes_index].write().await;
            let my_scope = lock.get_mut(&self.scope_id).expect("Self scope to exist");
            my_scope.active_directories.insert(dir_name, child_scope_id);
            drop(lock);
        }
        // add the child handle to the children_handles
        self.children_handles.insert(child_scope_id, Box::new(handle.clone()));
        // create supervisor handle
        let sup = self.handle.clone();
        // created visible data
        let visible_data = std::collections::HashSet::new();
        // the child depth, relative to the supervision tree height
        let child_depth = self.depth + 1;
        // create child context
        let mut child_context = Rt::<Child, <A::Channel as Channel>::Handle>::new(
            child_depth,
            service,
            scopes_index,
            child_scope_id,
            Some(parent_id),
            sup,
            handle.clone(),
            inbox,
            abort_registration,
            visible_data,
        );
        if let Some(metric) = metric.take() {
            child_context.register(metric).expect("Metric to be registered");
        }
        let (tx_oneshot, rx_oneshot) = tokio::sync::oneshot::channel::<ActorResult<Service>>();
        // create child future;
        let wrapped_fut = actor_fut_with_signal::<Child, _>(child, child_context, tx_oneshot);
        let join_handle = crate::spawn_task(task_name.as_ref(), wrapped_fut);
        self.children_joins.insert(child_scope_id, join_handle);
        Ok((handle, InitializedRx(child_scope_id, rx_oneshot)))
    }

    /// Insert/Update microservice.
    /// Note: it will remove the children handles/joins if the provided service is stopped
    /// and store entry for it in inactive if does have directory
    pub fn upsert_microservice(&mut self, scope_id: ScopeId, service: Service) {
        if service.is_stopped() {
            self.children_handles.remove(&scope_id);
            self.children_joins.remove(&scope_id);
            service
                .directory()
                .as_ref()
                .and_then(|dir| self.service.inactive.insert(dir.clone(), scope_id));
        }
        self.service.microservices.insert(scope_id, service);
    }
    /// Remove the microservice microservice under the provided scope_id
    pub fn remove_microservice(&mut self, scope_id: ScopeId) {
        self.children_handles.remove(&scope_id);
        self.children_joins.remove(&scope_id);
        self.service
            .microservices
            .remove(&scope_id)
            .and_then(|ms| ms.directory.and_then(|dir| self.service.inactive.remove(&dir)));
    }
    /// Returns mutable reference to the actor's inbox
    pub fn inbox_mut(&mut self) -> &mut <A::Channel as Channel>::Inbox {
        &mut self.inbox
    }
    /// Returns mutable reference to the actor's handle
    pub fn handle_mut(&mut self) -> &mut <A::Channel as Channel>::Handle {
        &mut self.handle
    }
    /// Returns immutable reference to the actor's handle
    pub fn handle(&self) -> &<A::Channel as Channel>::Handle {
        &self.handle
    }
    /// Returns mutable reference to the actor's supervisor handle
    pub fn supervisor_handle_mut(&mut self) -> &mut S {
        &mut self.sup_handle
    }
    /// Returns immutable reference to the actor's supervisor handle
    pub fn supervisor_handle(&self) -> &S {
        &self.sup_handle
    }
    /// Get the local service
    pub fn service(&self) -> &Service {
        &self.service
    }
    /// Return the actor's scope id
    pub fn scope_id(&self) -> ScopeId {
        self.scope_id
    }
    /// Return the actor's depth
    pub fn depth(&self) -> super::Depth {
        self.depth
    }
    /// Return the actor parent's scope id
    pub fn parent_id(&self) -> Option<ScopeId> {
        self.parent_scope_id
    }
    /// Shutdown the scope under the provided scope_id
    pub async fn shutdown_scope(&self, scope_id: ScopeId) -> anyhow::Result<()>
    where
        Self: Send,
    {
        let scopes_index = scope_id % *OVERCLOCK_PARTITIONS;
        let lock = SCOPES[scopes_index].read().await;
        if let Some(scope) = lock.get(&scope_id) {
            // we clone the handle to prevent very rare deadlock (which might only happen if channel is bounded)
            let shutdown_handle = scope.shutdown_handle.clone();
            drop(lock);
            shutdown_handle.shutdown().await;
        } else {
            anyhow::bail!("scope doesn't exist");
        };
        Ok(())
    }
    /// Check if microservices are stopped
    pub fn microservices_stopped(&self) -> bool {
        self.service.microservices.iter().all(|(_, s)| s.is_stopped())
    }
    /// Check if all microservices are _
    pub fn microservices_all(&self, are: fn(&Service) -> bool) -> bool {
        self.service.microservices.iter().all(|(_, s)| are(s))
    }
    /// Check if any microservice is _
    pub fn microservices_any(&self, is: fn(&Service) -> bool) -> bool {
        self.service.microservices.iter().any(|(_, s)| is(s))
    }

    /// Shutdown all the children within this actor context
    pub async fn shutdown_children(&mut self) {
        for (_, c) in self.children_handles.drain() {
            c.shutdown().await;
        }
    }
    /// Shutdown child using its scope_id
    pub async fn shutdown_child(
        &mut self,
        child_scope_id: &ScopeId,
    ) -> Option<tokio::task::JoinHandle<ActorResult<()>>> {
        if let Some(h) = self.children_handles.remove(child_scope_id) {
            h.shutdown().await;
            self.children_joins.remove(child_scope_id)
        } else {
            None
        }
    }
    /// Shutdown all the children of a given type within this actor context
    pub async fn shutdown_children_type<T: 'static + Actor<<A::Channel as Channel>::Handle>>(
        &mut self,
    ) -> HashMap<ScopeId, tokio::task::JoinHandle<ActorResult<()>>>
    where
        <A::Channel as Channel>::Handle: SupHandle<T>,
    {
        // extract the scopes for a given type
        let mut iter = self.service.scopes_iter::<T>();
        let mut joins = HashMap::new();
        while let Some(scope_id) = iter.next() {
            if let Some(h) = self.children_handles.remove(&scope_id) {
                h.shutdown().await;
                self.children_joins
                    .remove(&scope_id)
                    .and_then(|j| joins.insert(scope_id, j));
            };
        }
        joins
    }

    /// Add route for T
    pub async fn add_route<T: Send + 'static>(&self) -> anyhow::Result<()>
    where
        <A::Channel as Channel>::Handle: Route<T>,
    {
        let route: Box<dyn Route<T>> = Box::new(self.handle.clone());
        let mut lock = SCOPES[self.scopes_index].write().await;
        let my_scope = lock.get_mut(&self.scope_id).expect("expected self scope to exist");
        my_scope.router.insert(route);
        Ok(())
    }
    /// Remove route of T
    pub async fn remove_route<T: Send + 'static>(&self) -> anyhow::Result<()> {
        let mut lock = SCOPES[self.scopes_index].write().await;
        let my_scope = lock.get_mut(&self.scope_id).expect("expected self scope to exist");
        my_scope.router.remove::<Box<dyn Route<T>>>();
        Ok(())
    }
    /// Try to send message T, to the provided scope_i
    pub async fn send<T: Send + 'static>(&self, scope_id: ScopeId, message: T) -> anyhow::Result<()>
    where
        Self: Send + Sync,
    {
        let scopes_index = scope_id % *OVERCLOCK_PARTITIONS;
        let lock = SCOPES[scopes_index].read().await;
        if let Some(scope) = lock.get(&scope_id) {
            if let Some(route) = scope.router.get::<Box<dyn Route<T>>>() {
                if let Some(message) = route.try_send_msg(message).await? {
                    let cloned_route = route.clone();
                    drop(lock);
                    cloned_route.send_msg(message).await?;
                    Ok(())
                } else {
                    Ok(())
                }
            } else {
                anyhow::bail!("No route available")
            }
        } else {
            anyhow::bail!("No scope available")
        }
    }
}

impl<A: Actor<S>, S: SupHandle<A>> Rt<A, S>
where
    Self: Send,
{
    /// Defines how to breakdown the context and it should aknowledge shutdown to its supervisor
    async fn breakdown(mut self, actor: A, r: ActorResult<()>)
    where
        Self: Send,
    {
        // shutdown children handles (if any)
        self.shutdown_children().await;
        // await on children_joins just to force the contract
        for (_, c) in self.children_joins.drain() {
            let _ = c.await;
        }
        // Iterate all the microservices and set them to stopped
        self.service
            .microservices
            .iter_mut()
            .for_each(|(_, c)| c.update_status(ServiceStatus::Stopped));
        // unregister registered_metrics
        self.unregister_metrics().expect("To unregister metrics");
        // update service to be Stopped
        self.service.update_status(ServiceStatus::Stopped);
        // clone service
        let service = self.service.clone();
        // delete the active scope from the supervisor (if any)
        if let Some(parent_id) = self.parent_scope_id.take() {
            if let Some(dir_name) = service.directory.as_ref() {
                let parent_scopes_index = parent_id % *OVERCLOCK_PARTITIONS;
                let mut lock = SCOPES[parent_scopes_index].write().await;
                if let Some(parent_scope) = lock.get_mut(&parent_id) {
                    parent_scope.active_directories.remove(dir_name);
                }
                drop(lock);
            }
        }
        // publish the service before cleaning up any data
        self.publish(service.clone()).await;
        // drop any visible data
        let depth = self.depth;
        let scope_id = self.scope_id;
        if !self.visible_data.is_empty() {
            let mut lock = VISIBLE_DATA.write().await;
            for type_id in self.visible_data.drain() {
                if let Some(mut vec) = lock.remove(&type_id) {
                    // check if there are other resources for the same type_id
                    if vec.len() != 1 {
                        vec.retain(|&x| x != (depth, scope_id));
                        lock.insert(type_id, vec);
                    }
                };
            }
            drop(lock);
        }
        // drop scope
        let mut lock = SCOPES[self.scopes_index].write().await;
        let mut my_scope = lock.remove(&self.scope_id).expect("Self scope to exist on drop");
        drop(lock);
        for (_type_id, cleanup_self) in my_scope.cleanup_data.drain() {
            cleanup_self
                .cleanup_self(self.scope_id, &mut my_scope.data_and_subscribers)
                .await;
        }
        for (_type_id_scope_id, cleanup_from_other) in my_scope.cleanup.drain() {
            cleanup_from_other.cleanup_from_other(self.scope_id).await;
        }
        // aknshutdown to supervisor
        self.sup_handle.eol(self.scope_id, service, actor, r).await;
    }

    /// Update the service status, and report to the supervisor and subscribers (if any)
    pub async fn update_status(&mut self, service_status: ServiceStatus) {
        self.service.update_status(service_status);
        let service = self.service.clone();
        // report to supervisor
        self.sup_handle.report(self.scope_id, service.clone()).await;
        // publish the service to subscribers
        self.publish(service).await;
    }

    /// Stop the microservices and update status to stopping
    pub async fn stop(&mut self) {
        self.shutdown_children().await;
        self.update_status(ServiceStatus::Stopping).await;
    }
}

/// Unique Identifier for the resource, provided by the subscriber.
pub type ResourceRef = String;

/// Pushed event for a dynamic resources
#[derive(Debug, Clone)]
pub enum Event<T: Resource> {
    /// Scope under the ScopeId, it published the Resource T under given resource reference (string)
    Published(ScopeId, ResourceRef, T),
    /// Pushed when the resource is dropped
    Dropped(ScopeId, ResourceRef),
}

impl<T: Resource> Event<T> {
    /// Check if the resource is included
    pub fn is_included(&self) -> bool {
        if let Self::Dropped(_, _) = &self {
            true
        } else {
            false
        }
    }
}
impl<A: Actor<S>, S: SupHandle<A>> Rt<A, S> {
    /// Create Overclock context
    pub fn new(
        depth: usize,
        service: Service,
        scopes_index: usize,
        scope_id: ScopeId,
        parent_scope_id: Option<ScopeId>,
        sup_handle: S,
        handle: <A::Channel as Channel>::Handle,
        inbox: <A::Channel as Channel>::Inbox,
        abort_registration: AbortRegistration,
        visible_data: std::collections::HashSet<std::any::TypeId>,
    ) -> Self {
        Self {
            depth,
            service,
            scopes_index,
            scope_id,
            parent_scope_id,
            sup_handle,
            handle,
            inbox,
            children_handles: std::collections::HashMap::new(),
            children_joins: std::collections::HashMap::new(),
            abort_registration,
            registered_metrics: Vec::new(),
            visible_data,
        }
    }
    /// Register a metric
    pub fn register<T: Collector + Clone + 'static>(&mut self, metric: T) -> prometheus::Result<()> {
        let r = super::registry::PROMETHEUS_REGISTRY.register(Box::new(metric.clone()));
        if r.is_ok() {
            self.registered_metrics.push(Box::new(metric))
        }
        r
    }
    /// Unregister all the metrics
    pub(crate) fn unregister_metrics(&mut self) -> prometheus::Result<()> {
        for m in self.registered_metrics.pop() {
            super::registry::PROMETHEUS_REGISTRY.unregister(m)?
        }
        Ok(())
    }
}

impl<A: Actor<S>, S: SupHandle<A>> Rt<A, S> {
    /// Add exposed resource
    pub async fn add_resource<T: Resource>(&mut self, resource: T) {
        // publish the resource in the local scope data_store
        self.publish(resource).await;
        // index the scope_id by the resource type_id
        self.expose::<T>().await
    }
    /// Expose resource as visible data
    pub async fn expose<T: Resource>(&mut self) {
        let type_id = std::any::TypeId::of::<T>();
        self.visible_data.insert(type_id.clone());
        let depth = self.depth;
        let scope_id = self.scope_id;
        // expose globally
        let mut lock = VISIBLE_DATA.write().await;
        lock.entry(type_id)
            .and_modify(|data_vec| {
                data_vec.push((depth, scope_id));
                data_vec.sort_by(|a, b| a.0.cmp(&b.0));
            })
            .or_insert(vec![(depth, scope_id)]);
        drop(lock);
    }
    /// Get the highest resource's scope id (at the most top level) for a given global visiable resource
    pub async fn highest_scope_id<T: Resource>(&self) -> Option<ScopeId> {
        let type_id = std::any::TypeId::of::<T>();
        let lock = VISIBLE_DATA.read().await;
        if let Some(vec) = lock.get(&type_id) {
            Some(vec[0].clone().1)
        } else {
            None
        }
    }
    /// Gets the lowest resource's scope id for a given global visiable resource
    pub async fn lowest_scope_id<T: Resource>(&self) -> Option<ScopeId> {
        let type_id = std::any::TypeId::of::<T>();
        let lock = VISIBLE_DATA.read().await;
        if let Some(vec) = lock.get(&type_id) {
            Some(vec.last().expect("not empty scopes").1)
        } else {
            None
        }
    }
    /// Try to borrow resource and invoke fn_once
    pub async fn try_borrow<'a, T: Resource, R>(
        &self,
        resource_scope_id: ScopeId,
        fn_once: fn(&T) -> R,
    ) -> Option<R::Output>
    where
        R: std::future::Future + Send + 'static,
    {
        let scopes_index = resource_scope_id % *OVERCLOCK_PARTITIONS;
        let lock = SCOPES[scopes_index].read().await;
        if let Some(my_scope) = lock.get(&resource_scope_id) {
            if let Some(data) = my_scope.data_and_subscribers.get::<Data<T>>() {
                if let Some(resource) = data.resource.as_ref() {
                    return Some(fn_once(resource).await);
                }
            }
        }
        None
    }
    /// Try to borrow_mut the resource and invoke fn_once
    pub async fn try_borrow_mut<'a, T: Resource, R>(
        &self,
        resource_scope_id: ScopeId,
        fn_once: fn(&T) -> R,
    ) -> Option<R::Output>
    where
        R: std::future::Future + Send + 'static,
    {
        let scopes_index = resource_scope_id % *OVERCLOCK_PARTITIONS;
        let mut lock = SCOPES[scopes_index].write().await;
        if let Some(my_scope) = lock.get_mut(&resource_scope_id) {
            if let Some(data) = my_scope.data_and_subscribers.get::<Data<T>>() {
                if let Some(resource) = data.resource.as_ref() {
                    return Some(fn_once(resource).await);
                }
            }
        }
        None
    }
    /// Publish resource
    pub async fn publish<T: Resource>(&self, resource: T) {
        let scope_id = self.scope_id;
        let scopes_index = self.scopes_index;
        let mut lock = SCOPES[scopes_index].write().await;
        let my_scope = lock.get_mut(&scope_id).expect("Self scope to exist");
        if let Some(data) = my_scope.data_and_subscribers.get_mut::<Data<T>>() {
            let previous = data.resource.replace(resource.clone());
            let mut active_subscribers = HashMap::<ScopeId, Subscriber<T>>::new();
            // require further read locks
            let mut should_get_shutdown = Vec::<Box<dyn Shutdown>>::new();
            let mut should_get_dyn_resource = Vec::<(ScopeId, String, Box<dyn Route<Event<T>>>)>::new();
            // publish copy to existing subscriber(s)
            for (sub_scope_id, subscriber) in data.subscribers.drain() {
                match subscriber {
                    Subscriber::OneCopy(one_sender) => {
                        one_sender.send(Ok(resource.clone())).ok();
                    }
                    Subscriber::LinkedCopy(mut one_sender_opt, shutdown_handle, hard_link) => {
                        if let Some(one_sender) = one_sender_opt.take() {
                            one_sender.send(Ok(resource.clone())).ok();
                            // reinsert into our new subscribers
                            active_subscribers
                                .insert(sub_scope_id, Subscriber::LinkedCopy(None, shutdown_handle, hard_link));
                        } else {
                            // check if the resource already existed, which mean the actor already cloned a copy
                            if previous.is_some() && hard_link {
                                // note: we don't shut it down yet as it might cause deadlock in very rare race
                                // condition
                                should_get_shutdown.push(shutdown_handle);
                            }
                        };
                    }
                    Subscriber::DynCopy(resource_ref, route) => {
                        let event = Event::Published(scope_id, resource_ref.clone(), resource.clone());
                        match route.try_send_msg(event).await {
                            Ok(Some(_)) => {
                                should_get_dyn_resource.push((sub_scope_id, resource_ref.clone(), route.clone()));
                                active_subscribers.insert(sub_scope_id, Subscriber::DynCopy(resource_ref, route));
                            }
                            Ok(None) => {
                                active_subscribers.insert(sub_scope_id, Subscriber::DynCopy(resource_ref, route));
                                log::debug!("Message published to subscriber: {}", sub_scope_id);
                            }
                            Err(e) => {
                                // the subscriber is not active anymore
                                log::error!("{}", e);
                            }
                        };
                    }
                }
            }
            data.subscribers = active_subscribers;
            drop(lock);
            for shutdown_handle in should_get_shutdown.pop() {
                shutdown_handle.shutdown().await;
            }
            // publish the resource to other scopes
            for (sub_id, res_ref, route) in should_get_dyn_resource {
                let event = Event::Published(scope_id, res_ref, resource.clone());
                match route.send_msg(event).await {
                    Ok(()) => {
                        log::debug!("Message published to subscriber: {}", sub_id);
                    }
                    Err(e) => {
                        log::error!("{}", e);
                    }
                };
            }
        } else {
            let data = Data::<T>::with_resource(resource);
            let cleanup_data = CleanupData::<T>::new();
            let type_id = std::any::TypeId::of::<T>();
            my_scope.data_and_subscribers.insert(data);
            my_scope.cleanup_data.insert(type_id, Box::new(cleanup_data));
            drop(lock);
        };
    }
    /// Lookup for resource T under the provider scope_id
    pub async fn lookup<T: Resource>(&self, resource_scope_id: ScopeId) -> Option<T> {
        Scope::lookup(resource_scope_id).await
    }
    /// Remove the resource, and inform the interested dyn subscribers
    pub async fn remove_resource<T: Resource>(&self) -> Option<T> {
        // require further read locks
        let mut should_get_shutdown = Vec::<Box<dyn Shutdown>>::new();
        // dynamic subscribers who should be notified because the resource got dropped
        let mut should_get_notification = Vec::<(ResourceRef, Box<dyn Route<Event<T>>>)>::new();
        let mut lock = SCOPES[self.scopes_index].write().await;
        let my_scope = lock.get_mut(&self.scope_id).expect("self scope to exist");

        let mut resource = None;

        if let Some(mut data) = my_scope.data_and_subscribers.remove::<Data<T>>() {
            resource = data.resource.take();
            // iterate the subscribers
            // drain subscribers
            for (_sub_scope_id, subscriber) in data.subscribers.drain() {
                match subscriber {
                    // this happen when the actor drops a resource before it publishes it,
                    // it might happen because it had some critical dep that were required to create this resource.
                    Subscriber::OneCopy(one_sender) => {
                        one_sender.send(Err(anyhow::Error::msg("Resource got dropped"))).ok();
                    }
                    Subscriber::LinkedCopy(mut one_sender_opt, shutdown_handle, _hard_link) => {
                        if let Some(one_sender) = one_sender_opt.take() {
                            one_sender.send(Err(anyhow::Error::msg("Resource got dropped"))).ok();
                            should_get_shutdown.push(shutdown_handle);
                        } else {
                            should_get_shutdown.push(shutdown_handle);
                        };
                    }
                    Subscriber::DynCopy(res_ref, boxed_route) => {
                        should_get_notification.push((res_ref, boxed_route));
                    }
                }
            }
        }; // else no active subscribers, so nothing to do;
        for (res_ref, route) in should_get_notification.drain(..) {
            let dropped = Event::Dropped(self.scope_id, res_ref);
            route.send_msg(dropped).await.ok();
        }
        for shutdown_handle in should_get_shutdown.drain(..) {
            shutdown_handle.shutdown().await;
        }

        resource
    }
    /// Depends on resource T, it will await/block till the resource is available
    pub async fn depends_on<T: Resource>(&self, resource_scope_id: ScopeId) -> anyhow::Result<T, anyhow::Error> {
        let my_scope_id = self.scope_id;
        let resource_scopes_index = resource_scope_id % *OVERCLOCK_PARTITIONS;
        let mut lock = SCOPES[resource_scopes_index].write().await;
        if let Some(resource_scope) = lock.get_mut(&resource_scope_id) {
            // get the resource if it's available
            if let Some(data) = resource_scope.data_and_subscribers.get_mut::<Data<T>>() {
                if let Some(resource) = data.resource.clone().take() {
                    Ok(resource)
                } else {
                    let (tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<T>>();
                    let subscriber = Subscriber::<T>::OneCopy(tx);
                    data.subscribers.insert(my_scope_id, subscriber);
                    drop(lock);
                    let abortable = Abortable::new(rx, self.abort_registration.clone());
                    if let Ok(r) = abortable.await {
                        r?
                    } else {
                        anyhow::bail!("Aborted")
                    }
                }
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<T>>();
                let subscriber = Subscriber::<T>::OneCopy(tx);
                let data = Data::<T>::with_subscriber(my_scope_id, subscriber);
                let cleanup_data = CleanupData::<T>::new();
                let type_id = std::any::TypeId::of::<T>();
                resource_scope.cleanup_data.insert(type_id, Box::new(cleanup_data));
                resource_scope.data_and_subscribers.insert(data);
                drop(lock);
                let abortable = Abortable::new(rx, self.abort_registration.clone());
                if let Ok(r) = abortable.await {
                    r?
                } else {
                    anyhow::bail!("Aborted")
                }
            }
        } else {
            // resource_scope got dropped or it doesn't exist
            anyhow::bail!("Resource scope doesn't exist");
        }
    }
    /// Link to resource, which will await till it's available.
    /// NOTE: similar to depends_on, but only shutdown self actor if the resource got dropped
    /// hard_link: if shutdown the subscriber when the publisher publishes a new copy
    pub async fn link<T: Resource>(
        &self,
        resource_scope_id: ScopeId,
        hard_link: bool,
    ) -> anyhow::Result<T, anyhow::Error> {
        let my_scope_id = self.scope_id;
        let resource_scopes_index = resource_scope_id % *OVERCLOCK_PARTITIONS;
        let mut lock = SCOPES[resource_scopes_index].write().await;
        if let Some(resource_scope) = lock.get_mut(&resource_scope_id) {
            // get the resource if it's available
            if let Some(data) = resource_scope.data_and_subscribers.get_mut::<Data<T>>() {
                if let Some(resource) = data.resource.clone().take() {
                    let shutdown_handle = Box::new(self.handle.clone());
                    let subscriber = Subscriber::<T>::LinkedCopy(None, shutdown_handle, hard_link);
                    data.subscribers.insert(self.scope_id, subscriber);
                    drop(lock);
                    // the self actor might get shutdown before the resource provider,
                    // therefore we should cleanup self Subscriber::<T>::LinkedOneCopy from the provider
                    self.add_cleanup_from_other_obj::<T>(resource_scope_id).await;
                    Ok(resource)
                } else {
                    let (tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<T>>();
                    let shutdown_handle = Box::new(self.handle.clone());
                    let subscriber = Subscriber::<T>::LinkedCopy(Some(tx), shutdown_handle, hard_link);
                    data.subscribers.insert(my_scope_id, subscriber);
                    drop(lock);
                    let abortable = Abortable::new(rx, self.abort_registration.clone());
                    if let Ok(r) = abortable.await {
                        let r = r?;
                        if r.is_ok() {
                            // as mentioned above, this needed to cleanup the provider if the linked subscriber shutdown
                            // before the provider
                            self.add_cleanup_from_other_obj::<T>(resource_scope_id).await;
                        };
                        r
                    } else {
                        anyhow::bail!("Aborted")
                    }
                }
            } else {
                let (tx, rx) = tokio::sync::oneshot::channel::<anyhow::Result<T>>();
                let shutdown_handle = Box::new(self.handle.clone());
                let subscriber = Subscriber::<T>::LinkedCopy(Some(tx), shutdown_handle, hard_link);
                let cleanup_data = CleanupData::<T>::new();
                let type_id = std::any::TypeId::of::<T>();
                let data = Data::<T>::with_subscriber(my_scope_id, subscriber);
                resource_scope.data_and_subscribers.insert(data);
                resource_scope.cleanup_data.insert(type_id, Box::new(cleanup_data));
                drop(lock);
                let abortable = Abortable::new(rx, self.abort_registration.clone());
                if let Ok(r) = abortable.await {
                    let r = r?;
                    if r.is_ok() {
                        self.add_cleanup_from_other_obj::<T>(resource_scope_id).await;
                    };
                    r
                } else {
                    anyhow::bail!("Aborted")
                }
            }
        } else {
            // resource_scope got dropped or it doesn't exist
            anyhow::bail!("Resource scope doesn't exist");
        }
    }
    /// Depends on dynamic Resource T
    pub async fn subscribe<T: Resource>(
        &self,
        resource_scope_id: ScopeId,
        resource_ref: ResourceRef,
    ) -> anyhow::Result<Option<T>, anyhow::Error>
    where
        <A::Channel as Channel>::Handle: Route<Event<T>>,
    {
        let my_scope_id = self.scope_id;
        // add route first
        let mut lock = SCOPES[self.scopes_index].write().await;
        let my_scope = lock.get_mut(&my_scope_id).expect("Self scope to exist");
        let route: Box<dyn Route<Event<T>>> = Box::new(self.handle.clone());
        my_scope.router.insert(route.clone());
        let cleanup = Cleanup::<T>::new(resource_scope_id);
        let type_id = std::any::TypeId::of::<T>();
        my_scope.cleanup.insert((type_id, resource_scope_id), Box::new(cleanup));
        drop(lock);
        let resource_scopes_index = resource_scope_id % *OVERCLOCK_PARTITIONS;
        let mut lock = SCOPES[resource_scopes_index].write().await;
        if let Some(resource_scope) = lock.get_mut(&resource_scope_id) {
            // get the resource if it's available
            if let Some(data) = resource_scope.data_and_subscribers.get_mut::<Data<T>>() {
                let subscriber = Subscriber::<T>::DynCopy(resource_ref, route);
                data.subscribers.insert(my_scope_id, subscriber);
                if let Some(resource) = data.resource.clone().take() {
                    drop(lock);
                    Ok(Some(resource))
                } else {
                    drop(lock);
                    Ok(None)
                }
            } else {
                let subscriber = Subscriber::<T>::DynCopy(resource_ref, route);
                let cleanup_data = CleanupData::<T>::new();
                let type_id = std::any::TypeId::of::<T>();
                let data = Data::<T>::with_subscriber(my_scope_id, subscriber);
                resource_scope.data_and_subscribers.insert(data);
                resource_scope.cleanup_data.insert(type_id, Box::new(cleanup_data));
                drop(lock);
                Ok(None)
            }
        } else {
            let mut lock = SCOPES[self.scopes_index].write().await;
            let my_scope = lock.get_mut(&my_scope_id).expect("Self scope to exist");
            my_scope.cleanup.remove(&(type_id, resource_scope_id));
            drop(lock);
            // resource_scope got dropped or it doesn't exist
            anyhow::bail!("Resource scope doesn't exist");
        }
    }
    async fn add_cleanup_from_other_obj<T: Resource>(&self, resource_scope_id: ScopeId) {
        let cleanup = Cleanup::<T>::new(resource_scope_id);
        let type_id = std::any::TypeId::of::<T>();
        let mut lock = SCOPES[self.scopes_index].write().await;
        let my_scope = lock.get_mut(&self.scope_id).expect("Self scope to exist");
        my_scope.cleanup.insert((type_id, resource_scope_id), Box::new(cleanup));
        drop(lock);
    }
}

/// Overclock runtime, spawn the root actor, and server (if enabled)
pub struct Runtime<H> {
    #[allow(unused)]
    scope_id: ScopeId,
    join_handle: tokio::task::JoinHandle<ActorResult<()>>,
    handle: H,
    server: Option<Box<dyn Shutdown>>,
    server_join_handle: Option<tokio::task::JoinHandle<ActorResult<()>>>,
}

impl<H> Runtime<H>
where
    H: Clone + Shutdown,
{
    /// Create and spawn runtime using an existing latest config
    #[cfg(feature = "config")]
    pub async fn from_config<A>() -> ActorResult<Self>
    where
        A: 'static + ChannelBuilder<<A as Actor<NullSupervisor>>::Channel> + Actor<NullSupervisor> + Send + Sync,
        <A as Actor<NullSupervisor>>::Channel: Channel<Handle = H>,
        A: FileSystemConfig
            + DeserializeOwned
            + std::fmt::Debug
            + Serialize
            + Resource
            + std::default::Default
            + Config,
        H: Shutdown + Clone,
        VersionedValue<A>: DeserializeOwned,
        <A as FileSystemConfig>::ConfigType: ValueType,
        <<A as FileSystemConfig>::ConfigType as ValueType>::Value:
            std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
        for<'de> <<A as FileSystemConfig>::ConfigType as ValueType>::Value: Deserializer<'de>,
        for<'de> <<<A as FileSystemConfig>::ConfigType as ValueType>::Value as Deserializer<'de>>::Error:
            Into<anyhow::Error>,
    {
        #[cfg(feature = "console")]
        console_subscriber::init();

        let root_dir: Option<String> = A::type_name().to_string().into();
        let history = History::<HistoricalConfig<VersionedConfig<A>>>::load(20).map_err(|e| ActorError::exit(e))?;
        let child = history.latest().config;
        let runtime = Self::with_supervisor(root_dir, child, NullSupervisor).await?;
        Runtime::with_scope_id(3, "config".to_string(), history, NullSupervisor).await?;
        Ok(runtime)
    }
    /// Create and spawn runtime with null supervisor handle
    pub async fn new<T, A>(root_dir: T, child: A) -> ActorResult<Self>
    where
        A: 'static + ChannelBuilder<<A as Actor<NullSupervisor>>::Channel> + Actor<NullSupervisor>,
        T: Into<Option<String>>,
        <A as Actor<NullSupervisor>>::Channel: Channel<Handle = H>,
        H: Shutdown + Clone,
    {
        #[cfg(feature = "console")]
        console_subscriber::init();

        Self::with_supervisor(root_dir, child, NullSupervisor).await
    }
    /// Create new runtime with provided supervisor handle
    pub async fn with_supervisor<T, A, S>(dir: T, child: A, supervisor: S) -> ActorResult<Self>
    where
        A: 'static + ChannelBuilder<<A as Actor<S>>::Channel> + Actor<S>,
        T: Into<Option<String>>,
        <A as Actor<S>>::Channel: Channel<Handle = H>,
        S: SupHandle<A>,
        H: Shutdown + Clone,
    {
        Self::with_scope_id(0, dir, child, supervisor).await
    }
    /// Create new runtime with provided supervisor handle
    async fn with_scope_id<T, A, S>(child_scope_id: ScopeId, dir: T, mut child: A, supervisor: S) -> ActorResult<Self>
    where
        A: 'static + ChannelBuilder<<A as Actor<S>>::Channel> + Actor<S>,
        T: Into<Option<String>>,
        <A as Actor<S>>::Channel: Channel<Handle = H>,
        S: SupHandle<A>,
        H: Shutdown + Clone,
    {
        // try to create the actor's channel
        let (handle, inbox, abort_registration, mut metric, mut route) =
            child.build_channel().await?.channel::<A>(child_scope_id);
        // this is the root runtime, so we are going to use 0 as the parent_id
        let shutdown_handle = Box::new(handle.clone());
        // check which scopes partition owns the scope_id
        let scopes_index = child_scope_id % *OVERCLOCK_PARTITIONS;
        // create the service
        let dir = dir.into();
        let task_name = dir.clone().unwrap_or_default();
        let mut service = Service::new::<A, S>(dir.clone());
        let mut lock = SCOPES[scopes_index].write().await;
        service.update_status(ServiceStatus::Initializing);
        let mut scope = Scope::new(None, shutdown_handle.clone());
        if let Some(route) = route.take() {
            scope.router.insert(route);
        }
        let data = Data::with_resource(service.clone());
        scope.data_and_subscribers.insert(data);
        let data = Data::with_resource(handle.clone());
        scope.data_and_subscribers.insert(data);
        lock.insert(child_scope_id, scope);
        drop(lock);
        let visible_data = std::collections::HashSet::new();
        let depth = 0;
        // create child context
        let mut child_context = Rt::<A, S>::new(
            depth,
            service,
            scopes_index,
            child_scope_id,
            None,
            supervisor,
            handle.clone(),
            inbox,
            abort_registration,
            visible_data,
        );
        if let Some(metric) = metric.take() {
            child_context.register(metric).expect("Metric to be registered");
        }
        let (tx_oneshot, rx_oneshot) = tokio::sync::oneshot::channel::<ActorResult<Service>>();
        // create child future;
        let wrapped_fut = actor_fut_with_signal(child, child_context, tx_oneshot);
        let join_handle = crate::spawn_task(task_name.as_ref(), wrapped_fut);
        // only spawn ctrl_c for root with scope_id = 0
        if child_scope_id == 0 {
            crate::spawn_task("ctrl_c", ctrl_c(handle.clone()));
        }

        if let Err(e) = rx_oneshot.await.expect("oneshot to be alive") {
            log::error!("Unable to successfully initialize the root actor");
            join_handle.await.ok();
            return Err(e);
        };

        // todo print banner
        Ok(Self {
            scope_id: child_scope_id,
            handle,
            join_handle,
            server: None,
            server_join_handle: None,
        })
    }
    /// Returns immutable reference to the root actor
    pub fn handle(&self) -> &H {
        &self.handle
    }
    /// Returns mutable reference to the root actor
    pub fn handle_mut(&mut self) -> &mut H {
        &mut self.handle
    }

    #[cfg(feature = "websocket_server")]
    /// Enable the websocket server
    pub async fn websocket_server(mut self, addr: std::net::SocketAddr, mut ttl: Option<u32>) -> ActorResult<Self>
    where
        Websocket: Actor<NullSupervisor> + ChannelBuilder<<Websocket as Actor<NullSupervisor>>::Channel>,
        <Websocket as Actor<NullSupervisor>>::Channel: Channel,
    {
        let websocket_scope_id = 1;
        let mut websocket = Websocket::new(addr.clone()).link_to(Box::new(self.handle.clone()));
        if let Some(ttl) = ttl.take() {
            websocket = websocket.set_ttl(ttl);
        }
        let channel: <Websocket as Actor<NullSupervisor>>::Channel = websocket.build_channel().await?;
        let (handle, inbox, abort_registration, mut metric, mut route) =
            channel.channel::<Websocket>(websocket_scope_id);
        let shutdown_handle = Box::new(handle.clone());
        let scopes_index = websocket_scope_id % *OVERCLOCK_PARTITIONS;
        // create the service
        let dir_name: String = format!("ws@{}", addr);
        let mut service = Service::new::<Websocket, NullSupervisor>(Some(dir_name));
        let mut lock = SCOPES[scopes_index].write().await;
        service.update_status(ServiceStatus::Initializing);
        let mut scope = Scope::new(None, shutdown_handle.clone());
        if let Some(route) = route.take() {
            scope.router.insert(route);
        }
        lock.insert(websocket_scope_id, scope);
        drop(lock);
        let visible_data = std::collections::HashSet::new();
        let depth = 0;
        // create child context
        let mut child_context = Rt::<Websocket, NullSupervisor>::new(
            depth,
            service,
            scopes_index,
            1,
            None,
            NullSupervisor,
            handle.clone(),
            inbox,
            abort_registration,
            visible_data,
        );
        if let Some(metric) = metric.take() {
            child_context.register(metric).expect("Metric to be registered");
        }
        let wrapped_fut = async move {
            let mut child = websocket;
            let mut rt = child_context;
            let f = child.init(&mut rt);
            match f.await {
                Err(err) => {
                    // breakdown the child
                    let f = rt.breakdown(child, Err(err.clone()));
                    f.await;
                    Err(err)
                }
                Ok(deps) => {
                    if rt.service().is_initializing() {
                        rt.update_status(ServiceStatus::Running).await
                    }
                    let f = child.run(&mut rt, deps);
                    let r = f.await;
                    let f = rt.breakdown(child, r.clone());
                    f.await;
                    r
                }
            }
        };
        let join_handle = crate::spawn_task("websocket server", wrapped_fut);
        self.server_join_handle.replace(join_handle);
        self.server.replace(Box::new(handle.clone()));
        Ok(self)
    }

    #[cfg(feature = "backserver")]
    /// Enable backserver (websocket server + http + prometheus metrics)
    pub async fn backserver(mut self, addr: std::net::SocketAddr) -> ActorResult<Self>
    where
        crate::prefab::backserver::Backserver: Actor<NullSupervisor>
            + ChannelBuilder<<crate::prefab::backserver::Backserver as Actor<NullSupervisor>>::Channel>,
        <Websocket as Actor<NullSupervisor>>::Channel: Channel,
    {
        use crate::prefab::backserver::Backserver;
        let server_scope_id = 1;
        let mut websocket = Backserver::new(addr.clone(), self.scope_id).link_to(Box::new(self.handle.clone()));
        let channel: <Backserver as Actor<NullSupervisor>>::Channel = websocket.build_channel().await?;
        let (handle, inbox, abort_registration, mut metric, mut route) = channel.channel::<Backserver>(server_scope_id);
        let shutdown_handle = Box::new(handle.clone());
        let scopes_index = server_scope_id % *OVERCLOCK_PARTITIONS;
        // create the service
        let dir_name: String = format!("backserver@{}", addr);
        let mut service = Service::new::<Backserver, NullSupervisor>(Some(dir_name));
        let mut lock = SCOPES[scopes_index].write().await;
        service.update_status(ServiceStatus::Initializing);
        let mut scope = Scope::new(None, shutdown_handle.clone());
        if let Some(route) = route.take() {
            scope.router.insert(route);
        }
        lock.insert(server_scope_id, scope);
        drop(lock);
        let visible_data = std::collections::HashSet::new();
        let depth = 0;
        // create child context
        let mut child_context = Rt::<Backserver, NullSupervisor>::new(
            depth,
            service,
            scopes_index,
            1,
            None,
            NullSupervisor,
            handle.clone(),
            inbox,
            abort_registration,
            visible_data,
        );
        if let Some(metric) = metric.take() {
            child_context.register(metric).expect("Metric to be registered");
        }
        let wrapped_fut = async move {
            let mut child = websocket;
            let mut rt = child_context;
            let f = child.init(&mut rt);
            match f.await {
                Err(err) => {
                    // breakdown the child
                    let f = rt.breakdown(child, Err(err.clone()));
                    f.await;
                    Err(err)
                }
                Ok(deps) => {
                    if rt.service().is_initializing() {
                        rt.update_status(ServiceStatus::Running).await
                    }
                    let f = child.run(&mut rt, deps);
                    let r = f.await;
                    let f = rt.breakdown(child, r.clone());
                    f.await;
                    r
                }
            }
        };
        let join_handle = crate::spawn_task("backserver", wrapped_fut);
        self.server_join_handle.replace(join_handle);
        self.server.replace(Box::new(handle.clone()));
        Ok(self)
    }
    /// Block on the runtime till it shutdown gracefully
    pub async fn block_on(mut self) -> ActorResult<()> {
        let r = self.join_handle.await.expect("to join the root actor successfully");
        if let Some(ws_server_handle) = self.server.take() {
            ws_server_handle.shutdown().await;
            self.server_join_handle.expect("websocket join handle").await.ok();
        }
        r
    }
}

/// The actor wrapped future
async fn actor_fut_with_signal<A: Actor<S>, S: SupHandle<A>>(
    mut actor: A,
    mut rt: Rt<A, S>,
    check_init: InitSignalTx,
) -> ActorResult<()> {
    let f = actor.init(&mut rt);
    match f.await {
        Err(err) => {
            // breakdown the child
            let f = rt.breakdown(actor, Err(err.clone()));
            f.await;
            // inform oneshot receiver
            check_init.send(Err(err.clone())).ok();
            Err(err)
        }
        Ok(deps) => {
            if rt.service().is_initializing() {
                rt.update_status(ServiceStatus::Running).await
            }
            let service = rt.service().clone();
            // inform oneshot receiver
            check_init.send(Ok(service)).ok();
            let f = actor.run(&mut rt, deps);
            let r = f.await;
            let f = rt.breakdown(actor, r.clone());
            f.await;
            r
        }
    }
}
/// Useful function to exit program using ctrl_c signal
async fn ctrl_c<H: Shutdown>(handle: H) {
    // await on ctrl_c
    if let Err(e) = tokio::signal::ctrl_c().await {
        log::error!("Tokio ctrl_c signal error: {}", e)
    };
    handle.shutdown().await;
}

#[cfg(test)]
mod tests {
    use crate::core::*;

    ////////// Custom Overclock start /////////////;
    struct Overclock;
    #[derive(Debug)]
    enum OverclockEvent {
        Shutdown,
    }
    impl ShutdownEvent for OverclockEvent {
        fn shutdown_event() -> Self {
            OverclockEvent::Shutdown
        }
    }
    #[async_trait::async_trait]
    impl<S> Actor<S> for Overclock
    where
        S: SupHandle<Self>,
    {
        type Data = ();
        type Channel = UnboundedChannel<OverclockEvent>;
        async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
            // build and spawn your apps actors using the rt
            rt.handle().shutdown().await;
            Ok(())
        }
        async fn run(&mut self, rt: &mut Rt<Self, S>, _data: Self::Data) -> ActorResult<()> {
            while let Some(event) = rt.inbox_mut().next().await {
                match event {
                    OverclockEvent::Shutdown => {
                        log::info!("overclock got shutdown");
                        break;
                    }
                }
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn custom_overclock() {
        env_logger::init();
        let overclock = Overclock;
        let runtime = Runtime::new(Some("Overclock".into()), overclock)
            .await
            .expect("Runtime to build");
        runtime.block_on().await.expect("Runtime to run");
    }
    ////////// Custom Overclock end /////////
}
