// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

use super::*;
use async_trait::async_trait;
use core::pin::Pin;
use futures::{
    future::Aborted,
    stream::StreamExt,
    task::{AtomicWaker, Context, Poll},
};
use pin_project_lite::pin_project;
use prometheus::core::Collector;
use std::sync::atomic::{AtomicBool, Ordering};
pub use tokio::net::TcpListener;
use tokio::sync::mpsc::{error::TrySendError, Receiver, Sender, UnboundedReceiver, UnboundedSender};
use tokio_stream::wrappers::IntervalStream;
pub use tokio_stream::wrappers::TcpListenerStream;

// Inner type storing the waker to awaken and a bool indicating that it
// should be cancelled.
#[derive(Debug)]
struct AbortInner {
    waker: AtomicWaker,
    aborted: AtomicBool,
}
/// AbortHandle used to abort Abortable<T>
#[derive(Debug, Clone)]
pub struct AbortHandle {
    inner: std::sync::Arc<AbortInner>,
}

impl AbortHandle {
    /// Abort the abortable task
    pub fn abort(&self) {
        self.inner.aborted.store(true, Ordering::Relaxed);
        self.inner.waker.wake();
    }
    /// Create new (abort_handle, abort_registration) pair
    /// Note: AbortRegistration: should be used in sync order only.
    pub fn new_pair() -> (Self, AbortRegistration) {
        let inner = std::sync::Arc::new(AbortInner {
            waker: AtomicWaker::new(),
            aborted: AtomicBool::new(false),
        });

        (AbortHandle { inner: inner.clone() }, AbortRegistration { inner })
    }
}

/// Sync AbortRegistration, should only be used within the actor lifecycle.
#[derive(Debug, Clone)]
pub struct AbortRegistration {
    inner: std::sync::Arc<AbortInner>,
}

impl AbortRegistration {
    /// Check if the registeration is aborted
    pub fn is_aborted(&self) -> bool {
        self.inner.aborted.load(Ordering::Acquire)
    }
}

pin_project! {
    /// A future/stream which can be remotely short-circuited using an `AbortHandle`.
    #[derive(Debug, Clone)]
    #[must_use = "futures/streams do nothing unless you poll them"]
    pub struct Abortable<T> {
        #[pin]
        task: T,
        inner: std::sync::Arc<AbortInner>,
    }
}
impl<T> std::ops::Deref for Abortable<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.task
    }
}
impl<T> std::ops::DerefMut for Abortable<T> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.task
    }
}

impl<T> std::convert::AsMut<T> for Abortable<T> {
    fn as_mut(&mut self) -> &mut T {
        &mut self.task
    }
}
/// NOTE: forked from rust futures
impl<T> Abortable<T> {
    /// Create new abortable future/stream
    pub fn new(task: T, reg: AbortRegistration) -> Self {
        Self { task, inner: reg.inner }
    }
    /// Check if the abortable got aborted
    pub fn is_aborted(&self) -> bool {
        self.inner.aborted.load(Ordering::Relaxed)
    }
    fn try_poll<I>(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        mut poll: impl FnMut(Pin<&mut T>, &mut Context<'_>) -> Poll<I>,
    ) -> Poll<Result<I, Aborted>> {
        // Check if the task has been aborted
        if self.is_aborted() {
            return Poll::Ready(Err(Aborted));
        }
        // attempt to complete the task
        if let Poll::Ready(x) = poll(self.as_mut().project().task, cx) {
            return Poll::Ready(Ok(x));
        }
        // Register to receive a wakeup if the task is aborted in the future
        self.inner.waker.register(cx.waker());

        // Check to see if the task was aborted between the first check and
        // registration.
        // Checking with `is_aborted` which uses `Relaxed` is sufficient because
        // `register` introduces an `AcqRel` barrier.
        if self.is_aborted() {
            return Poll::Ready(Err(Aborted));
        }
        Poll::Pending
    }
}

/// Implementation to make Abortable future
impl<Fut> futures::future::Future for Abortable<Fut>
where
    Fut: futures::future::Future,
{
    type Output = Result<Fut::Output, Aborted>;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        self.try_poll(cx, |fut, cx| fut.poll(cx))
    }
}

/// Implementation to make Abortable Stream
impl<St> futures::stream::Stream for Abortable<St>
where
    St: futures::stream::Stream,
{
    type Item = St::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.try_poll(cx, |stream, cx| stream.poll_next(cx))
            .map(Result::ok)
            .map(Option::flatten)
    }
}

/// Implementation to make Abortable AsyncRead
impl<R> tokio::io::AsyncRead for Abortable<R>
where
    R: tokio::io::AsyncRead,
{
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut tokio::io::ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let r = self.try_poll(cx, |inner_fut, cx| inner_fut.poll_read(cx, buf));
        match r {
            Poll::Ready(outer_res) => match outer_res {
                Ok(inner_res) => Poll::Ready(inner_res),
                Err(Aborted) => {
                    return Poll::Ready(Ok(()));
                }
            },
            Poll::Pending => Poll::Pending,
        }
    }
}

/// Implementation to make Abortable AsyncWrite
impl<R> tokio::io::AsyncWrite for Abortable<R>
where
    R: tokio::io::AsyncWrite,
{
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<std::io::Result<usize>> {
        let r = self.try_poll(cx, |inner_fut, cx| inner_fut.poll_write(cx, buf));
        match r {
            Poll::Ready(outer_res) => match outer_res {
                Ok(inner_res) => Poll::Ready(inner_res),
                Err(Aborted) => {
                    return Poll::Ready(Ok(0));
                }
            },
            Poll::Pending => Poll::Pending,
        }
    }

    fn poll_write_vectored(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        bufs: &[std::io::IoSlice<'_>],
    ) -> Poll<std::io::Result<usize>> {
        let r = self.try_poll(cx, |inner_fut, cx| inner_fut.poll_write_vectored(cx, bufs));
        match r {
            Poll::Ready(outer_res) => match outer_res {
                Ok(inner_res) => Poll::Ready(inner_res),
                Err(Aborted) => {
                    return Poll::Ready(Ok(0));
                }
            },
            Poll::Pending => Poll::Pending,
        }
    }

    fn is_write_vectored(&self) -> bool {
        self.task.is_write_vectored()
    }

    #[inline]
    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.as_mut().project().task.poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        self.as_mut().project().task.poll_shutdown(cx)
    }
}

/// Defines a channel which becomes a sender and receiver half
#[async_trait]
pub trait Channel: Send + Sized {
    /// The channel Event type
    type Event: Send;
    /// The sender half of the channel
    type Handle: Send + Clone + super::Shutdown;
    /// The receiver half of the channel
    type Inbox: Send + Sync;
    /// Metric Collector
    type Metric: Collector + Clone;
    /// Create a sender and receiver of the appropriate types
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<Self::Metric>,
        Option<Box<dyn Route<Self::Event>>>,
    );
    /// Get this channel's name
    fn type_name() -> std::borrow::Cow<'static, str> {
        std::any::type_name::<Self>().into()
    }
}

/// Channel builder trait, to be implemented on the actor type
#[async_trait::async_trait]
pub trait ChannelBuilder<C: Channel> {
    /// Implement how to build the channel for the corresponding actor
    async fn build_channel(&mut self) -> ActorResult<C>;
}

/// Dynamic route as trait object, should be implemented on the actor's handle
#[async_trait::async_trait]
pub trait Route<M>: Send + Sync + dyn_clone::DynClone {
    /// Try to send message to the channel.
    async fn try_send_msg(&self, message: M) -> anyhow::Result<Option<M>>;
    /// if the Route<M> is behind lock, drop the lock before invoking this method.
    async fn send_msg(&self, message: M) -> anyhow::Result<()>;
}

dyn_clone::clone_trait_object!(<M> Route<M>);

/// A tokio unbounded mpsc channel implementation
pub struct UnboundedChannel<E> {
    abort_handle: AbortHandle,
    abort_registration: AbortRegistration,
    tx: UnboundedSender<E>,
    rx: UnboundedReceiver<E>,
}

impl<E> UnboundedChannel<E> {
    /// Create new unbounded channel
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<E>();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        Self {
            abort_handle,
            abort_registration,
            tx,
            rx,
        }
    }
}
/// Unbounded inbox of unbounded channel
#[derive(Debug)]
pub struct UnboundedInbox<T> {
    metric: prometheus::IntGauge,
    inner: UnboundedReceiver<T>,
}

impl<T> UnboundedInbox<T> {
    /// Create a new `UnboundedReceiver`.
    pub fn new(recv: UnboundedReceiver<T>, gauge: prometheus::IntGauge) -> Self {
        Self {
            inner: recv,
            metric: gauge,
        }
    }
    /// Closes the receiving half of a channel without dropping it.
    pub fn close(&mut self) {
        self.inner.close()
    }
}

impl<T> tokio_stream::Stream for UnboundedInbox<T> {
    type Item = T;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let r = self.inner.poll_recv(cx);
        if let &Poll::Ready(Some(_)) = &r {
            self.metric.dec();
        }
        r
    }
}

/// Unbounded handle
#[derive(Debug)]
pub struct UnboundedHandle<T> {
    scope_id: ScopeId,
    abort_handle: AbortHandle,
    metric: prometheus::IntGauge,
    inner: UnboundedSender<T>,
}

impl<T> Clone for UnboundedHandle<T> {
    fn clone(&self) -> Self {
        Self {
            scope_id: self.scope_id,
            abort_handle: self.abort_handle.clone(),
            metric: self.metric.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T> UnboundedHandle<T> {
    /// Create new abortable handle
    pub fn new(
        sender: UnboundedSender<T>,
        gauge: prometheus::IntGauge,
        abort_handle: AbortHandle,
        scope_id: ScopeId,
    ) -> Self {
        Self {
            scope_id,
            abort_handle,
            metric: gauge,
            inner: sender,
        }
    }
    /// Send Message to the channel
    pub fn send(&self, message: T) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        let r = self.inner.send(message);
        if r.is_ok() {
            self.metric.inc()
        }
        r
    }
    /// Send message to the channel after the duration pass
    pub fn send_after(
        &self,
        message: T,
        duration: std::time::Duration,
    ) -> tokio::task::JoinHandle<Result<(), tokio::sync::mpsc::error::SendError<T>>>
    where
        T: Send + 'static,
    {
        let h = self.clone();
        let fut = async move {
            tokio::time::sleep(duration).await;
            h.send(message)
        };
        tokio::spawn(fut)
    }
    /// Await till the channel unbounded receiver is dropped/closed
    pub async fn closed(&self) {
        self.inner.closed().await
    }
    /// Check if the channel is closed
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    /// Returns true if senders belong to the same channel
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

#[async_trait::async_trait]
impl<A: Send + 'static, T: ServiceEvent<A>> SupHandle<A> for UnboundedHandle<T> {
    type Event = T;
    async fn report(&self, scope_id: ScopeId, data: Service) -> Option<()> {
        self.send(T::report_event(scope_id, data)).ok()
    }
    async fn eol(self, scope_id: super::ScopeId, service: Service, actor: A, r: ActorResult<()>) -> Option<()> {
        self.send(T::eol_event(scope_id, service, actor, r)).ok()
    }
}

#[async_trait::async_trait]
impl<E: ShutdownEvent + 'static, T> ChannelBuilder<UnboundedChannel<E>> for T
where
    T: Send,
{
    async fn build_channel(&mut self) -> ActorResult<UnboundedChannel<E>> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<E>();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        Ok(UnboundedChannel {
            abort_handle,
            abort_registration,
            tx,
            rx,
        })
    }
}

impl<E: ShutdownEvent + 'static> Channel for UnboundedChannel<E> {
    type Event = E;
    type Handle = UnboundedHandle<E>;
    type Inbox = UnboundedInbox<E>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<Self::Metric>,
        Option<Box<dyn Route<Self::Event>>>,
    ) {
        let metric_fq_name = format!("ScopeId:{}", scope_id);
        let metric_helper_name = format!(
            "ScopeId: {}, Actor: {}, Channel: {}",
            scope_id,
            std::any::type_name::<T>(),
            Self::type_name()
        );
        let gauge = prometheus::core::GenericGauge::new(metric_fq_name, metric_helper_name)
            .expect("channel gauge can be created");
        let sender = self.tx;
        let recv = self.rx;
        let abort_handle = self.abort_handle;
        let abort_registration = self.abort_registration;
        let unbounded_handle = UnboundedHandle::new(sender, gauge.clone(), abort_handle, scope_id);
        let unbounded_inbox = UnboundedInbox::new(recv, gauge.clone());
        let route = Box::new(unbounded_handle.clone());
        (
            unbounded_handle,
            unbounded_inbox,
            abort_registration,
            Some(gauge),
            Some(route),
        )
    }
}

#[async_trait::async_trait]
impl<E: 'static + ShutdownEvent> super::Shutdown for UnboundedHandle<E> {
    async fn shutdown(&self) {
        self.abort_handle.abort();
        self.send(E::shutdown_event()).ok();
    }
    fn scope_id(&self) -> super::ScopeId {
        self.scope_id
    }
}

#[async_trait::async_trait]
impl<M: 'static + Send, E: Send> Route<M> for UnboundedHandle<E>
where
    E: std::convert::TryFrom<M>,
{
    async fn try_send_msg(&self, message: M) -> anyhow::Result<Option<M>> {
        if let Ok(event) = E::try_from(message) {
            if let Err(error) = self.send(event) {
                anyhow::bail!("SendError: {}", error)
            };
        } else {
            anyhow::bail!("Unable to convert the provided message into channel event")
        };
        Ok(None)
    }
    async fn send_msg(&self, message: M) -> anyhow::Result<()> {
        if let Ok(event) = E::try_from(message) {
            if let Err(error) = self.send(event) {
                anyhow::bail!("{}", error)
            };
            Ok(())
        } else {
            anyhow::bail!("Unable to convert the provided message into channel event")
        }
    }
}

/// Abortable tokio unbounded mpsc channel implementation
pub struct AbortableUnboundedChannel<E> {
    abort_handle: AbortHandle,
    abort_registration: AbortRegistration,
    tx: UnboundedSender<E>,
    rx: UnboundedReceiver<E>,
}

impl<E> AbortableUnboundedChannel<E> {
    /// Create abortable unbounded channel
    pub fn new() -> Self {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<E>();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        AbortableUnboundedChannel {
            abort_handle,
            abort_registration,
            tx,
            rx,
        }
    }
}

#[async_trait::async_trait]
impl<A: Send + 'static, T: ServiceEvent<A>> SupHandle<A> for AbortableUnboundedHandle<T> {
    type Event = T;
    async fn report(&self, scope_id: ScopeId, service: Service) -> Option<()> {
        self.send(T::report_event(scope_id, service)).ok()
    }
    async fn eol(self, scope_id: super::ScopeId, service: Service, actor: A, r: super::ActorResult<()>) -> Option<()> {
        self.send(T::eol_event(scope_id, service, actor, r)).ok()
    }
}

#[async_trait::async_trait]
impl<E: Send + 'static, T> ChannelBuilder<AbortableUnboundedChannel<E>> for T
where
    T: Send,
{
    async fn build_channel(&mut self) -> ActorResult<AbortableUnboundedChannel<E>> {
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<E>();
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        Ok(AbortableUnboundedChannel {
            abort_handle,
            abort_registration,
            tx,
            rx,
        })
    }
}
/// Abortable unbounded inbox, it returns None if the actor/channel got aborted
#[derive(Debug)]
pub struct AbortableUnboundedInbox<T> {
    inner: Abortable<UnboundedInbox<T>>,
}
impl<T> AbortableUnboundedInbox<T> {
    /// Create a new `AbortableUnboundedReceiver`.
    pub fn new(inbox: UnboundedInbox<T>, abort_registration: AbortRegistration) -> Self {
        let abortable = Abortable::new(inbox, abort_registration);
        Self { inner: abortable }
    }
    /// Closes the receiving half of a channel without dropping it.
    pub fn close(&mut self) {
        self.inner.as_mut().close()
    }
}

impl<T> tokio_stream::Stream for AbortableUnboundedInbox<T>
where
    Abortable<UnboundedInbox<T>>: tokio_stream::Stream,
{
    type Item = <Abortable<UnboundedInbox<T>> as tokio_stream::Stream>::Item;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.inner.poll_next_unpin(cx)
    }
}

/// Abortable unbounded handle
#[derive(Debug)]
pub struct AbortableUnboundedHandle<T> {
    inner: UnboundedHandle<T>,
}

impl<T> Clone for AbortableUnboundedHandle<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> AbortableUnboundedHandle<T> {
    /// Create new abortable unbounded handle
    pub fn new(sender: UnboundedHandle<T>) -> Self {
        Self { inner: sender }
    }
    /// Send Message to the channel
    pub fn send(&self, message: T) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        self.inner.send(message)
    }
    /// Send message to the channel after the duration pass
    pub fn send_after(
        &self,
        message: T,
        duration: std::time::Duration,
    ) -> tokio::task::JoinHandle<Result<(), tokio::sync::mpsc::error::SendError<T>>>
    where
        T: Send + 'static,
    {
        self.inner.send_after(message, duration)
    }
    /// Await till the channel is closed
    pub async fn closed(&self) {
        self.inner.closed().await
    }
    /// Check if the channel is closed
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    /// Returns true if senders belong to the same channel
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

impl<E: Send + 'static> Channel for AbortableUnboundedChannel<E> {
    type Event = E;
    type Handle = AbortableUnboundedHandle<E>;
    type Inbox = AbortableUnboundedInbox<E>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<E>>>,
    ) {
        let metric_fq_name = format!("ScopeId:{}", scope_id);
        let metric_helper_name = format!(
            "ScopeId: {}, Actor: {}, Channel: {}",
            scope_id,
            std::any::type_name::<T>(),
            Self::type_name()
        );
        let gauge = prometheus::core::GenericGauge::new(metric_fq_name, metric_helper_name)
            .expect("channel gauge can be created");
        let sender = self.tx;
        let recv = self.rx;
        let abort_handle = self.abort_handle;
        let abort_registration = self.abort_registration;
        let unbounded_handle = UnboundedHandle::new(sender, gauge.clone(), abort_handle, scope_id);
        let unbounded_inbox = UnboundedInbox::new(recv, gauge.clone());
        let abortable_unbounded_handle = AbortableUnboundedHandle::new(unbounded_handle);
        let abortable_unbounded_inbox = AbortableUnboundedInbox::new(unbounded_inbox, abort_registration.clone());
        let route = Box::new(abortable_unbounded_handle.clone());
        (
            abortable_unbounded_handle,
            abortable_unbounded_inbox,
            abort_registration,
            Some(gauge),
            Some(route),
        )
    }
}
#[async_trait::async_trait]
impl<E: Send + 'static> super::Shutdown for AbortableUnboundedHandle<E> {
    async fn shutdown(&self) {
        self.inner.abort_handle.abort();
    }
    fn scope_id(&self) -> ScopeId {
        self.inner.scope_id
    }
}

#[async_trait::async_trait]
impl<M: 'static + Send, E: Send> Route<M> for AbortableUnboundedHandle<E>
where
    E: std::convert::TryFrom<M>,
{
    async fn try_send_msg(&self, message: M) -> anyhow::Result<Option<M>> {
        self.inner.try_send_msg(message).await
    }
    async fn send_msg(&self, message: M) -> anyhow::Result<()> {
        if let Ok(event) = E::try_from(message) {
            if let Err(error) = self.send(event) {
                anyhow::bail!("{}", error)
            };
            Ok(())
        } else {
            anyhow::bail!("Unable to convert the provided message into channel event")
        }
    }
}

////////////// BoundedChannel
/// A tokio bounded mpsc channel implementation
pub struct BoundedChannel<E, const C: usize> {
    abort_handle: AbortHandle,
    abort_registration: AbortRegistration,
    tx: Sender<E>,
    rx: Receiver<E>,
}

/// Bounded inbox
#[derive(Debug)]
pub struct BoundedInbox<T> {
    metric: prometheus::IntGauge,
    inner: Receiver<T>,
}

impl<T> BoundedInbox<T> {
    /// Create a new `BoundedInbox`.
    pub fn new(recv: Receiver<T>, gauge: prometheus::IntGauge) -> Self {
        Self {
            inner: recv,
            metric: gauge,
        }
    }
    /// Closes the receiving half of a channel without dropping it.
    pub fn close(&mut self) {
        self.inner.close()
    }
}

impl<T> tokio_stream::Stream for BoundedInbox<T> {
    type Item = T;
    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let r = self.inner.poll_recv(cx);
        if let &Poll::Ready(Some(_)) = &r {
            self.metric.dec();
        }
        r
    }
}

/// Bounded handle
#[derive(Debug)]
pub struct BoundedHandle<T> {
    scope_id: ScopeId,
    abort_handle: AbortHandle,
    metric: prometheus::IntGauge,
    inner: Sender<T>,
}

impl<T> Clone for BoundedHandle<T> {
    fn clone(&self) -> Self {
        Self {
            scope_id: self.scope_id.clone(),
            abort_handle: self.abort_handle.clone(),
            metric: self.metric.clone(),
            inner: self.inner.clone(),
        }
    }
}

impl<T> BoundedHandle<T> {
    /// Create new bounded handle
    pub fn new(sender: Sender<T>, gauge: prometheus::IntGauge, abort_handle: AbortHandle, scope_id: ScopeId) -> Self {
        Self {
            scope_id,
            abort_handle,
            metric: gauge,
            inner: sender,
        }
    }
    /// Send message to the bounded channel
    pub async fn send(&self, message: T) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        let r = self.inner.send(message).await;
        if r.is_ok() {
            self.metric.inc()
        }
        r
    }
    /// Send message to the channel after the duration pass
    pub fn send_after(
        &self,
        message: T,
        duration: std::time::Duration,
    ) -> tokio::task::JoinHandle<Result<(), tokio::sync::mpsc::error::SendError<T>>>
    where
        T: Send + 'static,
    {
        let h = self.clone();
        let fut = async move {
            tokio::time::sleep(duration).await;
            h.send(message).await
        };
        tokio::spawn(fut)
    }
    /// Await till the channel is closed
    pub async fn closed(&self) {
        self.inner.closed().await
    }
    /// Attempt to send message to the bounded channel
    pub fn try_send(&self, message: T) -> Result<(), tokio::sync::mpsc::error::TrySendError<T>> {
        let r = self.inner.try_send(message);
        if r.is_ok() {
            self.metric.inc();
        }
        r
    }
    #[cfg(feature = "time")]
    #[cfg_attr(docsrs, doc(cfg(feature = "time")))]
    /// Send message within timeout limit
    pub async fn send_timeout(
        &self,
        value: T,
        timeout: Duration,
    ) -> Result<(), tokio::sync::mpsc::error::SendTimeoutError<T>> {
        let r = self.inner.send_timeout(message).await;
        if r.is_ok() {
            self.metric.inc();
        }
        r
    }
    #[cfg(feature = "sync")]
    /// Blocking send
    pub fn blocking_send(&self, value: T) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        let r = self.inner.blocking_send(message);
        if r.is_ok() {
            self.metric.inc();
        }
        r
    }
    /// check if the channel is closed
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    /// Returns true if the handle belongs to same channel
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
    // todo support permits
}

#[async_trait::async_trait]
impl<A: Send + 'static, T: ServiceEvent<A>> SupHandle<A> for AbortableBoundedHandle<T> {
    type Event = T;
    async fn report(&self, scope_id: ScopeId, service: Service) -> Option<()> {
        self.send(T::report_event(scope_id, service)).await.ok()
    }
    async fn eol(self, scope_id: super::ScopeId, service: Service, actor: A, r: ActorResult<()>) -> Option<()> {
        self.send(T::eol_event(scope_id, service, actor, r)).await.ok()
    }
}

#[async_trait::async_trait]
impl<E: ShutdownEvent + 'static, T, const C: usize> ChannelBuilder<BoundedChannel<E, C>> for T
where
    T: Send,
{
    async fn build_channel(&mut self) -> ActorResult<BoundedChannel<E, C>> {
        let (tx, rx) = tokio::sync::mpsc::channel::<E>(C);
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        Ok(BoundedChannel {
            abort_handle,
            abort_registration,
            tx,
            rx,
        })
    }
}

impl<E: ShutdownEvent + 'static, const C: usize> Channel for BoundedChannel<E, C> {
    type Event = E;
    type Handle = BoundedHandle<E>;
    type Inbox = BoundedInbox<E>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<Self::Metric>,
        Option<Box<dyn Route<E>>>,
    ) {
        let metric_fq_name = format!("ScopeId:{}", scope_id);
        let metric_helper_name = format!(
            "ScopeId: {}, Actor: {}, Channel: {}",
            scope_id,
            std::any::type_name::<T>(),
            Self::type_name()
        );
        let gauge = prometheus::core::GenericGauge::new(metric_fq_name, metric_helper_name)
            .expect("channel gauge can be created");
        let sender = self.tx;
        let recv = self.rx;
        let abort_handle = self.abort_handle;
        let abort_registration = self.abort_registration;
        let unbounded_handle = BoundedHandle::new(sender, gauge.clone(), abort_handle, scope_id);
        let unbounded_inbox = BoundedInbox::new(recv, gauge.clone());
        let route = Box::new(unbounded_handle.clone());
        (
            unbounded_handle,
            unbounded_inbox,
            abort_registration,
            Some(gauge),
            Some(route),
        )
    }
}

#[async_trait::async_trait]
impl<E: 'static + ShutdownEvent> super::Shutdown for BoundedHandle<E> {
    async fn shutdown(&self) {
        self.abort_handle.abort();
        self.send(E::shutdown_event()).await.ok();
    }
    fn scope_id(&self) -> ScopeId {
        self.scope_id
    }
}

#[async_trait::async_trait]
impl<M: 'static + Send, E: Send> Route<M> for BoundedHandle<E>
where
    E: std::convert::TryFrom<M>,
    E::Error: Send,
{
    async fn try_send_msg(&self, message: M) -> anyhow::Result<Option<M>> {
        // try to reserve permit
        match self.inner.try_reserve() {
            Ok(permit) => {
                // it's safe to send the message without causing deadlock
                if let Ok(event) = E::try_from(message) {
                    self.metric.inc();
                    permit.send(event);
                    Ok(None)
                } else {
                    anyhow::bail!("Unabled to convert the provided message into event type");
                }
            }
            Err(err) => {
                if let TrySendError::Full(()) = err {
                    Ok(Some(message))
                } else {
                    anyhow::bail!("Closed channel")
                }
            }
        }
    }
    async fn send_msg(&self, message: M) -> anyhow::Result<()> {
        if let Ok(event) = E::try_from(message) {
            self.send(event).await.map_err(|e| anyhow::Error::msg(format!("{}", e)))
        } else {
            anyhow::bail!("Unabled to convert the provided message into event type")
        }
    }
}

////////////// bounded channel end
/// Abortable tokio bounded mpsc channel implementation
pub struct AbortableBoundedChannel<E, const C: usize> {
    abort_handle: AbortHandle,
    abort_registration: AbortRegistration,
    tx: Sender<E>,
    rx: Receiver<E>,
}
/// Abortable bounded inbox.
#[derive(Debug)]
pub struct AbortableBoundedInbox<T> {
    inner: Abortable<BoundedInbox<T>>,
}

#[async_trait::async_trait]
impl<E: Send + 'static, T, const C: usize> ChannelBuilder<AbortableBoundedChannel<E, C>> for T
where
    T: Send,
{
    async fn build_channel(&mut self) -> ActorResult<AbortableBoundedChannel<E, C>> {
        let (tx, rx) = tokio::sync::mpsc::channel::<E>(C);
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        Ok(AbortableBoundedChannel {
            abort_handle,
            abort_registration,
            tx,
            rx,
        })
    }
}

impl<T> AbortableBoundedInbox<T> {
    /// Create a new `AbortableBoundedReceiver`.
    pub fn new(inbox: BoundedInbox<T>, abort_registration: AbortRegistration) -> Self {
        let abortable = Abortable::new(inbox, abort_registration);
        Self { inner: abortable }
    }
    /// Closes the receiving half of a channel without dropping it.
    pub fn close(&mut self) {
        self.inner.as_mut().close()
    }
}

#[derive(Debug)]
/// Abortable bounded handle
pub struct AbortableBoundedHandle<T> {
    inner: BoundedHandle<T>,
}

impl<T> Clone for AbortableBoundedHandle<T> {
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
        }
    }
}

impl<T> AbortableBoundedHandle<T> {
    /// Crete new abortable bounded handle
    pub fn new(sender: BoundedHandle<T>) -> Self {
        Self { inner: sender }
    }
    /// Send message to the channel
    pub async fn send(&self, message: T) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        self.inner.send(message).await
    }
    /// Send message to the channel after the duration pass
    pub fn send_after(
        &self,
        message: T,
        duration: std::time::Duration,
    ) -> tokio::task::JoinHandle<Result<(), tokio::sync::mpsc::error::SendError<T>>>
    where
        T: Send + 'static,
    {
        self.inner.send_after(message, duration)
    }
    /// Await till the channel is closed
    pub async fn closed(&self) {
        self.inner.closed().await
    }
    /// Attempts to send message to the channel
    pub fn try_send(&self, message: T) -> Result<(), tokio::sync::mpsc::error::TrySendError<T>> {
        self.inner.try_send(message)
    }
    #[cfg(feature = "time")]
    #[cfg_attr(docsrs, doc(cfg(feature = "time")))]
    /// Send message within defined timeout limit
    pub async fn send_timeout(
        &self,
        value: T,
        timeout: Duration,
    ) -> Result<(), tokio::sync::mpsc::error::SendTimeoutError<T>> {
        self.inner.send_timeout(message).await
    }
    #[cfg(feature = "sync")]
    /// Send message to the channel in blocking fashion
    pub fn blocking_send(&self, value: T) -> Result<(), tokio::sync::mpsc::error::SendError<T>> {
        self.inner.blocking_send(message)
    }
    /// Check if the channel is closed
    pub fn is_closed(&self) -> bool {
        self.inner.is_closed()
    }
    /// Returns true if the handle belongs to same channel
    pub fn same_channel(&self, other: &Self) -> bool {
        self.inner.same_channel(&other.inner)
    }
}

impl<E: Send + 'static, const C: usize> Channel for AbortableBoundedChannel<E, C> {
    type Event = E;
    type Handle = AbortableBoundedHandle<E>;
    type Inbox = AbortableBoundedInbox<E>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<E>>>,
    ) {
        let metric_fq_name = format!("ScopeId:{}", scope_id);
        let metric_helper_name = format!(
            "ScopeId: {}, Actor: {}, Channel: {}",
            scope_id,
            std::any::type_name::<T>(),
            Self::type_name()
        );
        let gauge = prometheus::core::GenericGauge::new(metric_fq_name, metric_helper_name)
            .expect("channel gauge can be created");
        let sender = self.tx;
        let recv = self.rx;
        let abort_handle = self.abort_handle;
        let abort_registration = self.abort_registration;
        let unbounded_handle = BoundedHandle::new(sender, gauge.clone(), abort_handle, scope_id);
        let unbounded_inbox = BoundedInbox::new(recv, gauge.clone());
        let abortable_unbounded_handle = AbortableBoundedHandle::new(unbounded_handle);
        let abortable_unbounded_inbox = AbortableBoundedInbox::new(unbounded_inbox, abort_registration.clone());
        let route = Box::new(abortable_unbounded_handle.clone());
        (
            abortable_unbounded_handle,
            abortable_unbounded_inbox,
            abort_registration,
            Some(gauge),
            Some(route),
        )
    }
}

#[async_trait::async_trait]
impl<E: Send + 'static> super::Shutdown for AbortableBoundedHandle<E> {
    async fn shutdown(&self) {
        self.inner.abort_handle.abort();
    }
    fn scope_id(&self) -> ScopeId {
        self.inner.scope_id
    }
}

#[async_trait::async_trait]
impl<M: 'static + Send, E: Send> Route<M> for AbortableBoundedHandle<E>
where
    E: std::convert::TryFrom<M>,
    E::Error: Send,
{
    async fn try_send_msg(&self, message: M) -> anyhow::Result<Option<M>> {
        self.inner.try_send_msg(message).await
    }
    async fn send_msg(&self, message: M) -> anyhow::Result<()> {
        // it's safe to send the message without causing deadlock
        if let Ok(event) = E::try_from(message) {
            self.send(event).await.map_err(|e| anyhow::Error::msg(format!("{}", e)))
        } else {
            anyhow::bail!("Unabled to convert the provided message into event type")
        }
    }
}
#[derive(Clone)]
/// Tcp listener handle, for tokio TcpListenerStream channel
pub struct TcpListenerHandle(AbortHandle, ScopeId);
#[async_trait::async_trait]
impl super::Shutdown for TcpListenerHandle {
    async fn shutdown(&self) {
        self.0.abort();
    }
    fn scope_id(&self) -> ScopeId {
        self.1
    }
}

impl Channel for TcpListenerStream {
    type Event = ();
    type Handle = TcpListenerHandle;
    type Inbox = Abortable<TcpListenerStream>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<()>>>,
    ) {
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let abortable_inbox = Abortable::new(self, abort_registration.clone());
        let abortable_handle = TcpListenerHandle(abort_handle, scope_id);
        (abortable_handle, abortable_inbox, abort_registration, None, None)
    }
}

#[cfg(feature = "hyper")]
mod hyper_channels {
    use super::*;
    pub use ::hyper;
    /// Hyper channel wrapper
    pub struct HyperChannel<S> {
        server: ::hyper::Server<::hyper::server::conn::AddrIncoming, S>,
    }

    impl<S: Send> HyperChannel<S> {
        /// Create new hyper channel
        pub fn new(server: ::hyper::Server<::hyper::server::conn::AddrIncoming, S>) -> Self {
            Self { server }
        }
    }

    use ::hyper::{server::conn::AddrStream, Body, Request, Response};
    impl<S, E, R, F> Channel for HyperChannel<S>
    where
        for<'a> S: ::hyper::service::Service<&'a AddrStream, Error = E, Response = R, Future = F> + Send,
        E: std::error::Error + Send + Sync + 'static,
        S: Send + 'static + Sync,
        F: Send + std::future::Future<Output = Result<R, E>> + 'static,
        R: Send + ::hyper::service::Service<Request<Body>, Response = Response<Body>> + 'static,
        R::Error: std::error::Error + Send + Sync,
        R::Future: Send,
    {
        type Event = ();
        type Handle = HyperHandle;
        type Inbox = HyperInbox;
        type Metric = prometheus::IntGauge;
        fn channel<T>(
            self,
            scope_id: ScopeId,
        ) -> (
            Self::Handle,
            Self::Inbox,
            AbortRegistration,
            Option<prometheus::IntGauge>,
            Option<Box<dyn Route<()>>>,
        ) {
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let f = futures::future::pending::<()>();
            let abortable = Abortable::new(f, abort_registration.clone());
            let graceful = self.server.with_graceful_shutdown(async {
                abortable.await.ok();
            });
            let hyper_handle = HyperHandle::new(abort_handle, scope_id);
            let hyper_inbox = HyperInbox::new(Box::pin(graceful));
            (hyper_handle, hyper_inbox, abort_registration, None, None)
        }
    }

    #[derive(Clone)]
    /// Hyper channel's handle
    pub struct HyperHandle {
        abort_handle: AbortHandle,
        scope_id: ScopeId,
    }

    impl HyperHandle {
        /// Create new Hyper channel's handle
        pub fn new(abort_handle: AbortHandle, scope_id: ScopeId) -> Self {
            Self { abort_handle, scope_id }
        }
    }

    #[async_trait::async_trait]
    impl Shutdown for HyperHandle {
        async fn shutdown(&self) {
            self.abort_handle.abort();
        }
        fn scope_id(&self) -> ScopeId {
            self.scope_id
        }
    }

    /// Hyper channel's inbox, used to ignite hyper server inside the actor run loop
    pub struct HyperInbox {
        pined_graceful: Option<
            Pin<Box<dyn futures::Future<Output = Result<(), ::hyper::Error>> + std::marker::Send + Sync + 'static>>,
        >,
    }
    impl HyperInbox {
        /// Ignite hyper server
        pub async fn ignite(&mut self) -> Result<(), ::hyper::Error> {
            if let Some(server) = self.pined_graceful.take() {
                server.await?
            }
            Ok(())
        }
    }
    impl HyperInbox {
        /// Create new hyper inbox
        pub fn new(
            pined_graceful: Pin<
                Box<dyn futures::Future<Output = Result<(), ::hyper::Error>> + std::marker::Send + Sync + 'static>,
            >,
        ) -> Self {
            Self {
                pined_graceful: Some(pined_graceful),
            }
        }
    }
}

#[cfg(feature = "hyper")]
pub use hyper_channels::*;

#[cfg(feature = "tungstenite")]
pub use tokio_tungstenite;

/// A tokio IntervalStream channel implementation, which emit Instants
pub struct IntervalChannel<const I: u64>;

#[derive(Clone)]
/// Interval channel's handle, can be used to abort the interval channel
pub struct IntervalHandle {
    scope_id: ScopeId,
    abort_handle: AbortHandle,
}

impl IntervalHandle {
    /// Create new interval handle
    pub fn new(abort_handle: AbortHandle, scope_id: ScopeId) -> Self {
        Self { scope_id, abort_handle }
    }
}

#[async_trait::async_trait]
impl<T, const I: u64> ChannelBuilder<IntervalChannel<I>> for T
where
    T: Send,
{
    async fn build_channel(&mut self) -> ActorResult<IntervalChannel<I>> {
        Ok(IntervalChannel::<I>)
    }
}

impl<const I: u64> Channel for IntervalChannel<I> {
    type Event = std::time::Instant;
    type Handle = IntervalHandle;
    type Inbox = Abortable<IntervalStream>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<Self::Event>>>,
    ) {
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let interval = tokio::time::interval(std::time::Duration::from_millis(I));
        let interval_handle = IntervalHandle::new(abort_handle, scope_id);
        let abortable_inbox = Abortable::new(IntervalStream::new(interval), abort_registration.clone());
        (interval_handle, abortable_inbox, abort_registration, None, None)
    }
}

impl Channel for std::time::Duration {
    type Event = std::time::Instant;
    type Handle = IntervalHandle;
    type Inbox = Abortable<IntervalStream>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<Self::Event>>>,
    ) {
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let interval = tokio::time::interval(self);
        let interval_handle = IntervalHandle::new(abort_handle, scope_id);
        let abortable_inbox = Abortable::new(IntervalStream::new(interval), abort_registration.clone());
        (interval_handle, abortable_inbox, abort_registration, None, None)
    }
}

#[async_trait::async_trait]
impl super::Shutdown for IntervalHandle {
    async fn shutdown(&self) {
        self.abort_handle.abort();
    }
    fn scope_id(&self) -> ScopeId {
        self.scope_id
    }
}

#[async_trait::async_trait]
impl<T> ChannelBuilder<NullChannel> for T
where
    T: Send,
{
    async fn build_channel(&mut self) -> ActorResult<NullChannel> {
        Ok(NullChannel)
    }
}
/// Null channel
pub struct NullChannel;
/// Null Inbox
pub struct NullInbox;

#[derive(Clone)]
/// Null handle, note: invoking abort will abort the manually defined abortable future/streams in the actor's run
/// lifecycle (if any)
pub struct NullHandle {
    scope_id: ScopeId,
    abort_handle: AbortHandle,
}

impl NullHandle {
    /// Create new null handle
    pub fn new(scope_id: ScopeId, abort_handle: AbortHandle) -> Self {
        Self { scope_id, abort_handle }
    }
}

impl Channel for NullChannel {
    type Event = ();
    type Handle = NullHandle;
    type Inbox = NullInbox;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<Self::Event>>>,
    ) {
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let null_handle = NullHandle::new(scope_id, abort_handle);
        (null_handle, NullInbox, abort_registration, None, None)
    }
}

#[async_trait::async_trait]
impl super::Shutdown for NullHandle {
    async fn shutdown(&self) {
        self.abort_handle.abort();
    }
    fn scope_id(&self) -> ScopeId {
        self.scope_id
    }
}

/// IoChannel(Stream/AsyncRead/AsyncWrite/fut) wrapper
pub struct IoChannel<T>(pub T);
impl<T> IoChannel<T> {
    /// Create new io channel
    pub fn new(channel: T) -> Self {
        Self(channel)
    }
}
#[derive(Clone)]
/// IoChannel's handle
pub struct IoHandle(AbortHandle, ScopeId);
impl IoHandle {
    fn new(abort_handle: AbortHandle, scope_id: ScopeId) -> Self {
        Self(abort_handle, scope_id)
    }
}

#[async_trait::async_trait]
impl Shutdown for IoHandle {
    async fn shutdown(&self) {
        self.0.abort()
    }
    fn scope_id(&self) -> ScopeId {
        self.1
    }
}
impl<S> Channel for IoChannel<S>
where
    S: Send + 'static + Sync,
{
    type Event = ();
    type Handle = IoHandle;
    type Inbox = Abortable<S>;
    type Metric = prometheus::IntGauge;
    fn channel<T>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<prometheus::IntGauge>,
        Option<Box<dyn Route<()>>>,
    ) {
        let (abort_handle, abort_registration) = AbortHandle::new_pair();
        let abortable_inbox = Abortable::new(self.0, abort_registration.clone());
        let abortable_handle = IoHandle::new(abort_handle, scope_id);
        (abortable_handle, abortable_inbox, abort_registration, None, None)
    }
}

/// Marker struct useful to propagate bounds through the Supervisor generic
pub struct Marker<T, B> {
    /// The actual type
    inner: T,
    _marker: std::marker::PhantomData<B>,
}

impl<T, B> Clone for Marker<T, B>
where
    T: Clone,
{
    fn clone(&self) -> Self {
        Self {
            inner: self.inner.clone(),
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T, B> Marker<T, B> {
    /// Create new marker wrapper
    pub fn new(inner: T) -> Self {
        Self {
            inner,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<T, B> std::ops::Deref for Marker<T, B> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.inner
    }
}

impl<T, B> std::ops::DerefMut for Marker<T, B> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.inner
    }
}

#[async_trait::async_trait]
impl<T, C, B: Send + 'static + Sync> ChannelBuilder<Marker<C, B>> for T
where
    T: Send + ChannelBuilder<C>,
    C: Channel,
{
    async fn build_channel(&mut self) -> ActorResult<Marker<C, B>> {
        let channel = <T as ChannelBuilder<C>>::build_channel(&mut self).await?;
        Ok(Marker::new(channel))
    }
}

impl<T, B: Send + Sync + 'static> Channel for Marker<T, B>
where
    T: Channel,
{
    type Event = T::Event;
    type Handle = Marker<T::Handle, B>;
    type Inbox = T::Inbox;
    type Metric = T::Metric;
    fn channel<A>(
        self,
        scope_id: ScopeId,
    ) -> (
        Self::Handle,
        Self::Inbox,
        AbortRegistration,
        Option<T::Metric>,
        Option<Box<dyn Route<T::Event>>>,
    ) {
        let (h, i, a, m, r) = T::channel::<A>(self.inner, scope_id);
        (Marker::new(h), i, a, m, r)
    }
}

#[async_trait::async_trait]
impl<T: Shutdown + Clone, B: Send + Sync + 'static> Shutdown for Marker<T, B> {
    async fn shutdown(&self) {
        self.inner.shutdown().await
    }
    fn scope_id(&self) -> ScopeId {
        self.inner.scope_id()
    }
}

#[cfg(feature = "rocket")]
mod rocket_channels {
    use super::*;
    pub use ::rocket;
    use ::rocket::Ignite;

    impl Channel for ::rocket::Rocket<Ignite> {
        type Event = ();
        type Handle = RocketHandle;
        type Inbox = RocketInbox;
        type Metric = prometheus::IntGauge;
        fn channel<T>(
            self,
            scope_id: ScopeId,
        ) -> (
            Self::Handle,
            Self::Inbox,
            AbortRegistration,
            Option<prometheus::IntGauge>,
            Option<Box<dyn Route<()>>>,
        ) {
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let rocket_shutdown = self.shutdown();
            let rocket_handle = RocketHandle::new(rocket_shutdown, abort_handle, scope_id);
            let rocket_inbox = RocketInbox::new(self);
            (rocket_handle, rocket_inbox, abort_registration, None, None)
        }
    }

    #[derive(Clone)]
    /// Rocket channel's handle
    pub struct RocketHandle {
        abort_handle: AbortHandle,
        rocket_shutdown: ::rocket::Shutdown,
        scope_id: ScopeId,
    }

    impl RocketHandle {
        /// Create new Rocket channel's handle
        pub fn new(rocket_shutdown: ::rocket::Shutdown, abort_handle: AbortHandle, scope_id: ScopeId) -> Self {
            Self {
                abort_handle,
                rocket_shutdown,
                scope_id,
            }
        }
    }

    #[async_trait::async_trait]
    impl Shutdown for RocketHandle {
        async fn shutdown(&self) {
            self.rocket_shutdown.clone().notify();
            self.abort_handle.abort();
        }
        fn scope_id(&self) -> ScopeId {
            self.scope_id
        }
    }

    /// The rocket inbox, which hold the rocket server ignite state
    pub struct RocketInbox {
        server: Option<::rocket::Rocket<::rocket::Ignite>>,
    }

    impl RocketInbox {
        /// Create new Rocket channel's inbox
        pub fn new(server: ::rocket::Rocket<::rocket::Ignite>) -> Self {
            Self { server: Some(server) }
        }
        /// Returns rocket server (if any)
        pub fn rocket(&mut self) -> Option<::rocket::Rocket<::rocket::Ignite>> {
            self.server.take()
        }
    }
}

#[cfg(feature = "rocket")]
pub use self::rocket_channels::*;

#[cfg(feature = "paho-mqtt")]
mod paho_mqtt_channels {
    use super::*;
    pub use paho_mqtt;
    use paho_mqtt::{AsyncClient, AsyncReceiver, Message};

    /// Mqtt channel using paho-mqtt lib
    pub struct MqttChannel {
        async_client: AsyncClient,
        stream: AsyncReceiver<Option<Message>>,
    }
    impl MqttChannel {
        /// Create new channel from a connected async_client, and existing stream
        pub fn new(async_client: AsyncClient, stream: AsyncReceiver<Option<Message>>) -> Self {
            Self { async_client, stream }
        }
    }
    impl Channel for MqttChannel {
        type Event = ();
        type Handle = MqttHandle;
        type Inbox = MqttInbox;
        type Metric = prometheus::IntGauge;
        fn channel<T>(
            self,
            scope_id: ScopeId,
        ) -> (
            Self::Handle,
            Self::Inbox,
            AbortRegistration,
            Option<prometheus::IntGauge>,
            Option<Box<dyn Route<()>>>,
        ) {
            let (abort_handle, abort_registration) = AbortHandle::new_pair();
            let inbox = MqttInbox::new(self.async_client, self.stream, abort_registration.clone());
            let handle = MqttHandle::new(abort_handle, scope_id);
            (handle, inbox, abort_registration, None, None)
        }
    }

    #[derive(Clone)]
    /// Paho mqtt channel's handle
    pub struct MqttHandle {
        abort_handle: AbortHandle,
        scope_id: ScopeId,
    }

    impl MqttHandle {
        /// Create new Mqtt channel's handle
        pub fn new(abort_handle: AbortHandle, scope_id: ScopeId) -> Self {
            Self { abort_handle, scope_id }
        }
    }

    #[async_trait::async_trait]
    impl Shutdown for MqttHandle {
        async fn shutdown(&self) {
            self.abort_handle.abort();
        }
        fn scope_id(&self) -> ScopeId {
            self.scope_id
        }
    }

    /// Mqtt's channel inbox
    pub struct MqttInbox {
        /// Async client
        async_client: AsyncClient,
        stream: Abortable<AsyncReceiver<Option<Message>>>,
        abort_registration: AbortRegistration,
    }

    impl MqttInbox {
        /// Create Mqtt's channel inbox
        pub fn new(
            async_client: AsyncClient,
            stream: AsyncReceiver<Option<Message>>,
            abort_registration: AbortRegistration,
        ) -> Self {
            Self {
                async_client,
                stream: Abortable::new(stream, abort_registration.clone()),
                abort_registration,
            }
        }
        /// Reconnect mqtt after the given duration
        pub async fn reconnect_after<D: Into<std::time::Duration>>(
            &mut self,
            duration: D,
        ) -> ActorResult<::paho_mqtt::Result<::paho_mqtt::ServerResponse>> {
            Abortable::new(tokio::time::sleep(duration.into()), self.abort_registration.clone())
                .await
                .map_err(|e| ActorError::aborted_msg(format!("mqtt aborted while reconnecting: {}", e)))?;
            self.reconnect().await
        }
        /// Reconnect mqtt
        pub async fn reconnect(&mut self) -> ActorResult<::paho_mqtt::Result<::paho_mqtt::ServerResponse>> {
            let reconnect_fut = async { self.async_client.reconnect().await };
            Abortable::new(reconnect_fut, self.abort_registration.clone())
                .await
                .map_err(|e| ActorError::aborted_msg(format!("mqtt aborted while reconnecting: {}", e)))
        }
        /// subscribe to the provided topic
        pub async fn subscribe<S>(
            &self,
            topic: S,
            qos: i32,
        ) -> ActorResult<::paho_mqtt::Result<::paho_mqtt::ServerResponse>>
        where
            S: Into<String>,
        {
            let subscribe_fut = async { self.async_client.subscribe(topic, qos).await };
            Abortable::new(subscribe_fut, self.abort_registration.clone())
                .await
                .map_err(|e| ActorError::aborted_msg(format!("mqtt aborted while subscribing: {}", e)))
        }
        /// Return the stream
        pub fn stream(&mut self) -> &mut Abortable<AsyncReceiver<Option<Message>>> {
            &mut self.stream
        }
        // todo add rest helpful method (subscribe_many, with_opt, etc)
    }
}

#[cfg(feature = "paho-mqtt")]
pub use self::paho_mqtt_channels::*;
