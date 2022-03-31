// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use super::{
    file::{ConfigFileType, FileSystemConfig},
    versioned::*,
    *,
};
use serde::{Deserialize, Serialize};
use std::marker::PhantomData;

impl<CT> CurrentVersion for MyConfig<CT> {
    const CURRENT_VERSION: usize = 3;
}

#[derive(Debug, Serialize, Deserialize)]
struct MyConfig<CT> {
    pub name: String,
    #[serde(skip)]
    _config_type: PhantomData<fn(CT) -> CT>,
}

impl<CT> PartialEq for MyConfig<CT> {
    fn eq(&self, other: &Self) -> bool {
        self.name == other.name
    }
}
impl<CT> Eq for MyConfig<CT> {}

impl<CT> Default for MyConfig<CT> {
    fn default() -> Self {
        Self {
            name: Default::default(),
            _config_type: Default::default(),
        }
    }
}

impl<CT> Clone for MyConfig<CT> {
    fn clone(&self) -> Self {
        Self {
            name: self.name.clone(),
            _config_type: Default::default(),
        }
    }
}

impl<CT: ConfigFileType> FileSystemConfig for MyConfig<CT> {
    type ConfigType = CT;
}

#[cfg(feature = "ron_config")]
#[test]
fn test_load_ron() {
    if let Err(_) = VersionedConfig::<MyConfig<RONConfig>>::load_or_save_default() {
        VersionedConfig::<MyConfig<RONConfig>>::load().unwrap();
    }
}

#[cfg(feature = "toml_config")]
#[test]
fn test_load_toml() {
    if let Err(_) = VersionedConfig::<MyConfig<TOMLConfig>>::load_or_save_default() {
        VersionedConfig::<MyConfig<TOMLConfig>>::load().unwrap();
    }
}

#[cfg(feature = "json_config")]
#[test]
fn test_load_json() {
    if let Err(_) = VersionedConfig::<MyConfig<JSONConfig>>::load_or_save_default() {
        VersionedConfig::<MyConfig<JSONConfig>>::load().unwrap();
    }
}

#[tokio::test]
#[cfg(feature = "ron_config")]
async fn test_load_historic() {
    use super::persist::PersistHandle;
    use tokio::sync::RwLock;

    let history = History::<HistoricalConfig<VersionedConfig<MyConfig<RONConfig>>>>::load(20)
        .or_else(|_| History::<HistoricalConfig<VersionedConfig<MyConfig<RONConfig>>>>::load(20))
        .unwrap();
    let rw_lock = RwLock::new(history);
    let mut history_handle: PersistHandle<_> = rw_lock.write().await.into();
    let mut latest = history_handle.latest().clone();
    latest.name = "update1".to_string();
    history_handle.update(latest.clone());
    latest.name = "update2".to_string();
    history_handle.update(latest.clone());
    latest.name = "update3".to_string();
    history_handle.update(latest);
}
