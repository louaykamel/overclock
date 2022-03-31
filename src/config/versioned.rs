// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::{bail, file::*, persist::Persist, LoadableConfig, SerializableConfig};
use serde::{de::DeserializeOwned, Deserialize, Deserializer, Serialize};
use std::{
    convert::{TryFrom, TryInto},
    ops::{Deref, DerefMut},
};

/// Version marker
pub trait CurrentVersion {
    /// The current static version of the config
    const CURRENT_VERSION: usize;
}

impl<T> CurrentVersion for T
where
    T: crate::core::Actor<crate::core::NullSupervisor>,
{
    const CURRENT_VERSION: usize = T::VERSION;
}

/// A versioned configuration
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct VersionedConfig<C>
where
    C: CurrentVersion,
{
    pub(crate) version: usize,
    pub(crate) config: C,
}

impl<C: FileSystemConfig + CurrentVersion + DeserializeOwned> LoadableConfig for VersionedConfig<C>
where
    VersionedValue<C>: DeserializeOwned,
    <C as FileSystemConfig>::ConfigType: ValueType,
    <<C as FileSystemConfig>::ConfigType as ValueType>::Value:
        std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
    for<'de> <<C as FileSystemConfig>::ConfigType as ValueType>::Value: Deserializer<'de>,
    for<'de> <<<C as FileSystemConfig>::ConfigType as ValueType>::Value as Deserializer<'de>>::Error:
        Into<anyhow::Error>,
{
    /// Load a versioned configuration from a file. Will fail if the file does not exist or the actual version does not
    /// match the requested one.
    fn load() -> anyhow::Result<Self> {
        VersionedValue::load()?.verify_version()?.try_into()
    }

    /// Load a versioned configuration from a file. Will fail if the file does not exist or the actual version does not
    /// match the requested one. If the file does not exist, a default config will be saved to the disk.
    fn load_or_save_default() -> anyhow::Result<Self>
    where
        Self: Default + SerializableConfig,
    {
        match VersionedValue::load() {
            Ok(v) => v.verify_version()?.try_into(),
            Err(e) => {
                Self::default().save()?;
                bail!(
                    "Config file was not found! Saving a default config file. Please edit it and restart the application! (Error: {})", e,
                );
            }
        }
    }
}

impl<C: Default + CurrentVersion> Default for VersionedConfig<C> {
    fn default() -> Self {
        Self {
            version: C::CURRENT_VERSION,
            config: Default::default(),
        }
    }
}

impl<C: CurrentVersion> Deref for VersionedConfig<C> {
    type Target = C;

    fn deref(&self) -> &Self::Target {
        &self.config
    }
}

impl<C: CurrentVersion> DerefMut for VersionedConfig<C> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.config
    }
}

impl<C: FileSystemConfig + CurrentVersion + Serialize> Persist for VersionedConfig<C> {
    fn persist(&self) -> anyhow::Result<()> {
        self.save()
    }
}

// Could be made into proc macro
impl<C: FileSystemConfig + CurrentVersion> FileSystemConfig for VersionedConfig<C> {
    type ConfigType = <C as FileSystemConfig>::ConfigType;
    const CONFIG_DIR: &'static str = C::CONFIG_DIR;
    const FILENAME: &'static str = C::FILENAME;
}
impl<C: CurrentVersion> DefaultFileSave for VersionedConfig<C> {}

impl<C: FileSystemConfig + CurrentVersion + DeserializeOwned> TryFrom<VersionedValue<C>> for VersionedConfig<C>
where
    <C as FileSystemConfig>::ConfigType: ValueType,
    <<C as FileSystemConfig>::ConfigType as ValueType>::Value:
        std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
    for<'de> <<C as FileSystemConfig>::ConfigType as ValueType>::Value: Deserializer<'de>,
    for<'de> <<<C as FileSystemConfig>::ConfigType as ValueType>::Value as Deserializer<'de>>::Error:
        Into<anyhow::Error>,
{
    type Error = anyhow::Error;

    fn try_from(VersionedValue { version, config }: VersionedValue<C>) -> anyhow::Result<VersionedConfig<C>> {
        Ok(Self {
            version,
            config: C::deserialize(config).map_err(|e| anyhow::anyhow!(e))?,
        })
    }
}

/// A variant of `VersionedConfig` which is used to deserialize
/// a config file independent of its inner structure so the version can
/// be validated.
#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct VersionedValue<C>
where
    C: FileSystemConfig + CurrentVersion,
    <C as FileSystemConfig>::ConfigType: ValueType,
    <<C as FileSystemConfig>::ConfigType as ValueType>::Value:
        std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
{
    version: usize,
    config: <<C as FileSystemConfig>::ConfigType as ValueType>::Value,
}

impl<C> VersionedValue<C>
where
    C: FileSystemConfig + CurrentVersion,
    <C as FileSystemConfig>::ConfigType: ValueType,
    <<C as FileSystemConfig>::ConfigType as ValueType>::Value:
        std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
{
    fn verify_version(self) -> anyhow::Result<Self> {
        anyhow::ensure!(
            self.version == C::CURRENT_VERSION,
            "Config file version mismatch! Expected: {}, Actual: {}",
            C::CURRENT_VERSION,
            self.version
        );
        Ok(self)
    }
}

impl<C> FileSystemConfig for VersionedValue<C>
where
    C: FileSystemConfig + CurrentVersion,
    <C as FileSystemConfig>::ConfigType: ValueType,
    <<C as FileSystemConfig>::ConfigType as ValueType>::Value:
        std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
{
    type ConfigType = <C as FileSystemConfig>::ConfigType;
    const CONFIG_DIR: &'static str = C::CONFIG_DIR;
    const FILENAME: &'static str = C::FILENAME;
}
impl<C> DefaultFileLoad for VersionedValue<C>
where
    C: FileSystemConfig + CurrentVersion,
    <C as FileSystemConfig>::ConfigType: ValueType,
    <<C as FileSystemConfig>::ConfigType as ValueType>::Value:
        std::fmt::Debug + Serialize + DeserializeOwned + Clone + PartialEq,
{
}
