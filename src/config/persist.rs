use super::{file::FileSystemConfig, *};

/// Specifies that the implementor should be able to persist itself
pub trait Persist {
    /// Persist this value
    fn persist(&self) -> anyhow::Result<()>;
}

/// A handle which will persist when dropped
pub struct PersistHandle<'a, C: 'a + Config + Persist + FileSystemConfig>
where
    HistoricalConfig<C>: FileSystemConfig,
{
    guard: tokio::sync::RwLockWriteGuard<'a, History<HistoricalConfig<C>>>,
}

impl<'a, C: Config + Persist + FileSystemConfig>
    From<tokio::sync::RwLockWriteGuard<'a, History<HistoricalConfig<C>>>> for PersistHandle<'a, C>
where
    HistoricalConfig<C>: FileSystemConfig,
{
    fn from(guard: tokio::sync::RwLockWriteGuard<'a, History<HistoricalConfig<C>>>) -> Self {
        Self { guard }
    }
}

impl<'a, C: Config + Persist + FileSystemConfig> Deref for PersistHandle<'a, C>
where
    HistoricalConfig<C>: FileSystemConfig,
{
    type Target = tokio::sync::RwLockWriteGuard<'a, History<HistoricalConfig<C>>>;

    fn deref(&self) -> &Self::Target {
        &self.guard
    }
}

impl<'a, C: Config + Persist + FileSystemConfig> DerefMut for PersistHandle<'a, C>
where
    HistoricalConfig<C>: FileSystemConfig,
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.guard
    }
}

impl<'a, C: Config + Persist + FileSystemConfig> std::ops::Drop for PersistHandle<'a, C>
where
    HistoricalConfig<C>: FileSystemConfig,
{
    fn drop(&mut self) {
        if let Err(e) = self.persist() {
            error!("{}", e);
        }
    }
}
