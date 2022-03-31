// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

use super::{Actor, ScopeId};
use ptree::TreeItem;
use serde::{Deserialize, Serialize};
use std::time::SystemTime;
/// The possible statuses a service (application) can be
#[repr(u8)]
#[derive(Copy, Clone, PartialEq, Debug, Serialize, Deserialize)]
pub enum ServiceStatus {
    /// Early bootup
    Starting = 0,
    /// Late bootup
    Initializing = 1,
    /// The service is operational but one or more services failed(Degraded or Maintenance) or in process of being
    /// fully operational while startup.
    Degraded = 2,
    /// The service is fully operational.
    Running = 3,
    /// The service is shutting down, should be handled accordingly by active dependent services
    Stopping = 4,
    /// The service is maintenance mode, should be handled accordingly by active dependent services
    Maintenance = 5,
    /// The service is not running, should be handled accordingly by active dependent services
    Stopped = 6,
    /// The service is idle.
    Idle = 7,
    /// The service is in outage.
    Outage = 8,
}

impl std::fmt::Display for ServiceStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            match self {
                ServiceStatus::Starting => "Starting",
                ServiceStatus::Initializing => "Initializing",
                ServiceStatus::Degraded => "Degraded",
                ServiceStatus::Running => "Running",
                ServiceStatus::Idle => "Idle",
                ServiceStatus::Outage => "Outage",
                ServiceStatus::Stopping => "Stopping",
                ServiceStatus::Maintenance => "Maintenance",
                ServiceStatus::Stopped => "Stopped",
            }
        )
    }
}

impl Default for ServiceStatus {
    fn default() -> Self {
        Self::Starting
    }
}

/// An actor's service metrics
#[derive(Clone, Debug, Serialize)]
pub struct Service {
    #[serde(skip_serializing)]
    /// The actor's type id, who owns/manages the service
    pub actor_type_id: std::any::TypeId,
    /// The actor type name, only for debuging or to provide context
    pub actor_type_name: &'static str,
    /// The status of the actor
    pub status: ServiceStatus,
    /// The directory name of the actor, must be unique within the same spawned level
    pub directory: Option<String>,
    /// The start timestamp, used to calculate uptime
    pub up_since: SystemTime,
    /// Accumulated downtime
    pub downtime_ms: u64,
    /// Microservices
    pub microservices: std::collections::HashMap<ScopeId, Self>,
    /// Highlight any stopped service in microservices;
    pub inactive: std::collections::HashMap<String, ScopeId>,
}
/// The microservices scopes ids iterator
pub struct ServiceScopesIterator<'a> {
    actor_type_id: std::any::TypeId,
    inner: std::collections::hash_map::Iter<'a, ScopeId, Service>,
}
impl<'a> ServiceScopesIterator<'a> {
    fn new<T: 'static>(iter: std::collections::hash_map::Iter<'a, ScopeId, Service>) -> Self {
        Self {
            actor_type_id: std::any::TypeId::of::<T>(),
            inner: iter,
        }
    }
}
impl<'a> Iterator for ServiceScopesIterator<'a> {
    type Item = ScopeId;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some((scope_id, service)) = self.inner.next() {
            if service.actor_type_id() == self.actor_type_id {
                return Some(*scope_id);
            } else {
                continue;
            }
        }
        None
    }
}

impl Service {
    /// Create a new Service
    pub fn new<A: 'static + Actor<S>, S: SupHandle<A>>(directory: Option<String>) -> Self {
        Self {
            actor_type_id: std::any::TypeId::of::<A>(),
            actor_type_name: A::type_name(),
            directory: directory.into(),
            status: ServiceStatus::Starting,
            up_since: SystemTime::now(),
            downtime_ms: 0,
            microservices: std::collections::HashMap::new(),
            inactive: std::collections::HashMap::new(),
        }
    }
    /// Create scopes iterator for a the provided Child type
    pub fn scopes_iter<'a, Child: 'static>(&'a self) -> ServiceScopesIterator<'a> {
        ServiceScopesIterator::new::<Child>(self.microservices.iter())
    }
    /// Return the actor type id
    pub fn actor_type_id(&self) -> std::any::TypeId {
        self.actor_type_id
    }
    /// Return actor type name, note: only for debuging
    pub fn actor_type_name(&self) -> &'static str {
        self.actor_type_name
    }
    /// Set the service status
    pub fn with_status(mut self, service_status: ServiceStatus) -> Self {
        self.status = service_status;
        self
    }
    /// Update the service status
    pub fn update_status(&mut self, service_status: ServiceStatus) {
        // todo update the uptime/downtime if needed
        self.status = service_status;
    }
    /// Return the directory name of the actor (if available)
    pub fn directory(&self) -> &Option<String> {
        &self.directory
    }
    /// Set the service downtime in milliseconds
    pub fn with_downtime_ms(mut self, downtime_ms: u64) -> Self {
        self.downtime_ms = downtime_ms;
        self
    }
    /// Check the owner actor's type of the service
    pub fn is_type<T: 'static>(&self) -> bool {
        let is_type_id = std::any::TypeId::of::<T>();
        self.actor_type_id == is_type_id
    }
    /// Check if the service is stopping
    pub fn is_stopping(&self) -> bool {
        self.status == ServiceStatus::Stopping
    }
    /// Check if the service is stopped
    pub fn is_stopped(&self) -> bool {
        self.status == ServiceStatus::Stopped
    }
    /// Check if the service is running
    pub fn is_running(&self) -> bool {
        self.status == ServiceStatus::Running
    }
    /// Check if the service is idle
    pub fn is_idle(&self) -> bool {
        self.status == ServiceStatus::Idle
    }
    /// Check if the service status is outage
    pub fn is_outage(&self) -> bool {
        self.status == ServiceStatus::Outage
    }
    /// Check if the service is starting
    pub fn is_starting(&self) -> bool {
        self.status == ServiceStatus::Starting
    }
    /// Check if the service is initializing
    pub fn is_initializing(&self) -> bool {
        self.status == ServiceStatus::Initializing
    }
    /// Check if the service is in maintenance
    pub fn is_maintenance(&self) -> bool {
        self.status == ServiceStatus::Maintenance
    }
    /// Check if the service is degraded
    pub fn is_degraded(&self) -> bool {
        self.status == ServiceStatus::Degraded
    }
    /// Get the service status
    pub fn status(&self) -> &ServiceStatus {
        &self.status
    }
    /// Return immutable reference to the microservices HashMap
    pub fn microservices(&self) -> &std::collections::HashMap<ScopeId, Service> {
        &self.microservices
    }
    /// Return mutable reference to the microservices HashMap
    #[allow(unused)]
    pub(crate) fn microservices_mut(&mut self) -> &mut std::collections::HashMap<ScopeId, Service> {
        &mut self.microservices
    }
    /// Return mutable reference to the inactive directories
    #[allow(unused)]
    pub(crate) fn inactive_mut(&mut self) -> &mut std::collections::HashMap<String, ScopeId> {
        &mut self.inactive
    }
}

impl TreeItem for Service {
    type Child = Service;

    fn write_self<W: std::io::Write>(&self, f: &mut W, _style: &ptree::Style) -> std::io::Result<()> {
        write!(
            f,
            "actor: {}, dir: {:?}, status: {}, uptime: {}",
            self.actor_type_name,
            self.directory,
            self.status,
            self.up_since.elapsed().expect("Expected elapsed to unwrap").as_millis()
        )
    }

    fn children(&self) -> std::borrow::Cow<[Self::Child]> {
        self.microservices
            .clone()
            .into_iter()
            .map(|(_, c)| c)
            .collect::<Vec<Self::Child>>()
            .into()
    }
}

impl std::fmt::Display for Service {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut buf = std::io::Cursor::new(Vec::<u8>::new());
        ptree::write_tree(self, &mut buf).ok();
        write!(f, "{}", String::from_utf8_lossy(&buf.into_inner()))
    }
}

#[async_trait::async_trait]
/// The sup handle which supervise T: Actor<Self>
pub trait SupHandle<T: Send>: 'static + Send + Sized + Sync {
    /// The supervisor event type
    type Event;
    /// Report any status & service changes
    /// return Some(()) if the report success
    async fn report(&self, scope_id: super::ScopeId, service: Service) -> Option<()>
    where
        T: Actor<Self>,
        Self: SupHandle<T>;
    /// Report End of life for a T actor
    /// return Some(()) if the report success
    async fn eol(self, scope_id: super::ScopeId, service: Service, actor: T, r: super::ActorResult<()>) -> Option<()>
    where
        T: Actor<Self>;
}

/// Ideally it should be implemented using proc_macro on the event type
pub trait ServiceEvent<T>: Send + 'static {
    /// Creates report event that will be pushed as status change from the Child: T
    fn report_event(scope: super::ScopeId, service: Service) -> Self;
    /// Creates eol event that will be pushed as end of life event once the Child: T breakdown
    fn eol_event(scope: super::ScopeId, service: Service, actor: T, r: super::ActorResult<()>) -> Self;
}
