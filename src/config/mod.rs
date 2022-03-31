// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

#![warn(missing_docs)]
//! Config definitions for overclock use

use anyhow::{anyhow, bail};
use glob::glob;
use log::{debug, error};
use serde::{de::DeserializeOwned, Serialize};
use std::{
    collections::BinaryHeap,
    fmt::Debug,
    fs::{File, OpenOptions},
    io::{Read, Write},
    ops::{Deref, DerefMut},
    path::PathBuf,
};

mod types;
pub use types::*;

mod history;
pub use history::*;

/// File system loading utilities
pub mod file;
pub use file::{FileSystemConfig, ValueType};
/// Peristence trait and handle implementations
pub mod persist;
pub use persist::Persist;
/// Versioned config wrappers
pub mod versioned;
pub use versioned::*;

#[cfg(all(test, any(feature = "ron_config", feature = "json_config", feature = "toml_config")))]
mod test;

/// The default config file name
pub const DEFAULT_FILENAME: &str = "config";

/// Defines a configuration file which can be loaded and saved
pub trait Config: Default + Clone + PartialEq + Debug + serde::Serialize {}
impl<T: Default + Clone + PartialEq + Debug + serde::Serialize> Config for T {}

/// Defines a wrapper type which wraps a dereference-able inner value
pub trait Wrapper: Deref {
    /// Consume the wrapper and retrieve the inner type
    fn into_inner(self) -> Self::Target;
}

/// Defines a config file which can be saved somehow via the Serialize trait
pub trait SerializableConfig: Serialize {
    /// Save the config
    fn save(&self) -> anyhow::Result<()>;
}

/// Defines a config which can be loaded somehow
pub trait LoadableConfig {
    /// Load the config
    fn load() -> anyhow::Result<Self>
    where
        Self: Sized;

    /// Load the config or return a default value
    fn load_or_default() -> Self
    where
        Self: Sized + Default,
    {
        Self::load().unwrap_or_default()
    }

    /// Load the config or save a default value
    fn load_or_save_default() -> anyhow::Result<Self>
    where
        Self: Sized + Default + SerializableConfig,
    {
        let res = Self::load();
        if res.is_err() {
            Self::default().save()?;
            bail!(
                "Config file was not found! Saving a default config file. Please edit it and restart the application!",
            );
        } else {
            res
        }
    }
}

/// Defines a type which can be read into a config
pub trait ConfigReader<C> {
    /// Read the config
    fn read_config(self) -> anyhow::Result<C>;
}

#[cfg(feature = "ron_config")]
impl<C: LoadableConfig + DeserializeOwned> ConfigReader<C> for ron::Value {
    fn read_config(self) -> anyhow::Result<C> {
        C::deserialize(self).map_err(|e| anyhow!(e))
    }
}

#[cfg(feature = "json_config")]
impl<C: LoadableConfig + DeserializeOwned> ConfigReader<C> for serde_json::Value {
    fn read_config(self) -> anyhow::Result<C> {
        C::deserialize(self).map_err(|e| anyhow!(e))
    }
}

#[cfg(feature = "toml_config")]
impl<C: LoadableConfig + DeserializeOwned> ConfigReader<C> for toml::Value {
    fn read_config(self) -> anyhow::Result<C> {
        C::deserialize(self).map_err(|e| anyhow!(e))
    }
}

/// Defines a type which can be written to using a config
pub trait ConfigWriter<C> {
    /// Write the config
    fn write_config(&mut self, config: &C) -> anyhow::Result<()>;
}
