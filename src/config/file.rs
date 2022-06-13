// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::*;

/// Configuration type which defines how configuration files are serialized and deserialized
pub trait ConfigFileType {
    /// The extension of this config type
    fn extension() -> &'static str;

    /// Serialize the config to a writer
    fn serialize<C: Serialize, W: Write>(config: &C, writer: &mut W) -> anyhow::Result<()>;

    /// Deserialize the config from a reader
    fn deserialize<C: DeserializeOwned, R: Read>(reader: &mut R) -> anyhow::Result<C>;
}

/// Defines a value type which can be used as an intermediate step when serializing and deserializing
pub trait ValueType {
    /// The value type
    type Value;
}

/// Marker trait which enables the default file save implementation
pub trait DefaultFileSave {}

/// Marker trait which enables the default file load implementation
pub trait DefaultFileLoad {}

impl<T> SerializableConfig for T
where
    T: FileSystemConfig + Serialize,
{
    fn save(&self) -> anyhow::Result<()> {
        let dir = T::dir();
        debug!("Saving config to {}", dir.to_string_lossy());
        if !dir.exists() {
            std::fs::create_dir_all(dir)?;
        }
        T::write_file()?.write_config(self)
    }
}

impl<T> LoadableConfig for T
where
    T: FileSystemConfig + DefaultFileLoad + DeserializeOwned,
{
    fn load() -> anyhow::Result<Self>
    where
        Self: Sized,
    {
        T::read_file()?.read_config()
    }
}

impl<C: FileSystemConfig + DeserializeOwned> ConfigReader<C> for File {
    fn read_config(mut self) -> anyhow::Result<C> {
        C::ConfigType::deserialize(&mut self)
    }
}

impl<C: FileSystemConfig + Serialize> ConfigWriter<C> for File {
    fn write_config(&mut self, config: &C) -> anyhow::Result<()> {
        C::ConfigType::serialize(config, self)
    }
}

/// Defines values which can be used to load a config from a serialized file
pub trait FileSystemConfig {
    /// The type of config. Can be anything which defines a `ConfigFileType` implementation.
    type ConfigType: ConfigFileType;
    /// The directory from which to load the config file
    const CONFIG_DIR: &'static str = ".";
    /// The config filename
    const FILENAME: &'static str = DEFAULT_FILENAME;

    /// Get the directory from which to load the config file.
    /// Defaults to checking the `CONFIG_DIR` env variable, then the defined constant.
    fn dir() -> PathBuf {
        std::env::var("CONFIG_DIR")
            .ok()
            .unwrap_or(Self::CONFIG_DIR.to_string())
            .into()
    }

    /// Open the config file for reading
    fn read_file() -> anyhow::Result<File> {
        OpenOptions::new()
            .read(true)
            .open(Self::dir().join(format!("{}.{}", Self::FILENAME, Self::ConfigType::extension())))
            .map_err(|e| anyhow!(e))
    }

    /// Open the config file for writing (or create one)
    fn write_file() -> anyhow::Result<File> {
        OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(Self::dir().join(format!("{}.{}", Self::FILENAME, Self::ConfigType::extension())))
            .map_err(|e| anyhow!(e))
    }
}

#[cfg(feature = "ron_config")]
#[cfg(not(feature = "toml_config"))]
#[cfg(not(feature = "json_config"))]
impl<T> FileSystemConfig for T
where
    T: crate::core::Actor<crate::core::NullSupervisor>,
{
    type ConfigType = super::RONConfig;
}

#[cfg(feature = "toml_config")]
#[cfg(not(feature = "ron_config"))]
#[cfg(not(feature = "json_config"))]
impl<T> FileSystemConfig for T
where
    T: crate::core::Actor<crate::core::NullSupervisor>,
{
    type ConfigType = super::TOMLConfig;
}

#[cfg(feature = "json_config")]
#[cfg(not(feature = "ron_config"))]
#[cfg(not(feature = "toml_config"))]
impl<T> FileSystemConfig for T
where
    T: crate::core::Actor<crate::core::NullSupervisor>,
{
    type ConfigType = super::JSONConfig;
}
