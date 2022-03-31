//! Overclock actor framework

#![warn(missing_docs)]
/// Overclock core functionality
pub mod core;
#[cfg(feature = "prefabs")]
/// Overclock core functionality
pub mod prefab;

#[cfg(feature = "config")]
pub mod config;

/// Spawn a task with a provided name, if tokio console tracing is enabled
#[allow(unused_variables)]
pub fn spawn_task<T>(name: &str, future: T) -> tokio::task::JoinHandle<T::Output>
where
    T: futures::Future + Send + 'static,
    T::Output: Send + 'static,
{
    #[cfg(all(tokio_unstable, feature = "console"))]
    return tokio::task::Builder::new().name(name).spawn(future);

    #[cfg(not(all(tokio_unstable, feature = "console")))]
    return tokio::spawn(future);
}
