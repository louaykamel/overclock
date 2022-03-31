// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

use std::{fmt, time::Duration};
use thiserror::Error;

/// The returned result by the actor
pub type ActorResult<T> = std::result::Result<T, ActorError>;

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
/// Actor shutdown error reason.
pub enum ActorRequest {
    /// The actor got Aborted while working on something critical.
    Aborted,
    /// The actor exit, as it cannot do anything further.
    // example maybe the disk is full or a bind address is in-use
    Exit,
    /// The actor is asking for restart, if possible.
    Restart(Option<Duration>),
}

/// An actor's error which contains an optional request for the supervisor
#[derive(Error, Debug)]
#[error("Source: {source}, request: {request:?}")]
pub struct ActorError {
    /// The error source
    pub source: anyhow::Error,
    /// The actor request
    pub request: Option<ActorRequest>,
}

impl Default for ActorError {
    fn default() -> Self {
        Self {
            source: anyhow::anyhow!("An unknown error occurred!"),
            request: None,
        }
    }
}

/// ActorResult extension trait
pub trait ActorResultExt {
    /// The actor error
    type Error;
    /// The error request
    fn err_request(self, request: ActorRequest) -> ActorResult<Self::Error>
    where
        Self: Sized;
}

impl<T> ActorResultExt for anyhow::Result<T> {
    type Error = T;
    fn err_request(self, request: ActorRequest) -> ActorResult<T>
    where
        Self: Sized,
    {
        match self {
            Err(source) => Err(ActorError {
                source,
                request: request.into(),
            }),
            Ok(t) => Ok(t),
        }
    }
}

impl From<anyhow::Error> for ActorError {
    fn from(source: anyhow::Error) -> Self {
        Self { source, request: None }
    }
}

impl Clone for ActorError {
    fn clone(&self) -> Self {
        Self {
            source: anyhow::anyhow!(self.source.to_string()),
            request: self.request.clone(),
        }
    }
}

impl ActorError {
    /// Create exit error from anyhow error
    pub fn exit<E: Into<anyhow::Error>>(error: E) -> Self {
        Self {
            source: error.into(),
            request: ActorRequest::Exit.into(),
        }
    }
    /// Create exit error from message
    pub fn exit_msg<E>(msg: E) -> Self
    where
        E: fmt::Display + fmt::Debug + Send + Sync + 'static,
    {
        Self {
            source: anyhow::Error::msg(msg),
            request: ActorRequest::Exit.into(),
        }
    }
    /// Create Aborted error from anyhow::error, note: this soft error
    pub fn aborted<E: Into<anyhow::Error>>(error: E) -> Self {
        Self {
            source: error.into(),
            request: ActorRequest::Aborted.into(),
        }
    }
    /// Create Aborted error from message, note: this soft error
    pub fn aborted_msg<E>(msg: E) -> Self
    where
        E: fmt::Display + fmt::Debug + Send + Sync + 'static,
    {
        Self {
            source: anyhow::Error::msg(msg),
            request: ActorRequest::Aborted.into(),
        }
    }
    /// Create restart error, it means the actor is asking the supervisor for restart/reschedule if possible
    pub fn restart<E: Into<anyhow::Error>, D: Into<Option<Duration>>>(error: E, after: D) -> Self {
        Self {
            source: error.into(),
            request: ActorRequest::Restart(after.into()).into(),
        }
    }
    /// Create restart error, it means the actor is asking the supervisor for restart/reschedule if possible
    pub fn restart_msg<E, D: Into<Option<Duration>>>(msg: E, after: D) -> Self
    where
        E: fmt::Display + fmt::Debug + Send + Sync + 'static,
    {
        Self {
            source: anyhow::Error::msg(msg),
            request: ActorRequest::Restart(after.into()).into(),
        }
    }
}
