// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
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
