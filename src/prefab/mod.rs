// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

#[cfg(feature = "backserver")]
/// Backserver provides websocket, http and prometheus server
pub mod backserver;
#[cfg(feature = "hyper")]
/// Hyper, provides hyper channel and prefab functionality
pub mod hyper;
#[cfg(feature = "tungstenite")]
/// The websocket server
pub mod websocket;

#[cfg(feature = "rocket")]
/// The rocket webserver
pub mod rocket;

#[cfg(feature = "tonicserver")]
/// Tonic, provides Tonic channel and prefab functionality
pub mod tonic;

/// Axum, provides Axum channel and prefab functionality
#[cfg(feature = "axumserver")]
pub mod axum;
