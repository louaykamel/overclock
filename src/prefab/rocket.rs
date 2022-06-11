// Copyright 2022 Louay Kamel
// Copyright 2021 IOTA Stiftung
// SPDX-License-Identifier: Apache-2.0

use std::time::SystemTime;

use crate::core::{Actor, ActorError, ActorResult, ChannelBuilder, Rt, SupHandle};
use async_trait::async_trait;
use prometheus::{HistogramOpts, HistogramVec, IntCounter, IntCounterVec, Opts};
use rocket::{
    catch,
    fairing::{Fairing, Info, Kind},
    Data, Ignite, Request, Response, Rocket,
};

/// Rocket server
pub struct RocketServer {
    rocket: Option<::rocket::Rocket<Ignite>>,
}

impl RocketServer {
    /// Create new rocket server
    pub fn new(rocket: ::rocket::Rocket<Ignite>) -> RocketServer {
        RocketServer { rocket: Some(rocket) }
    }
}
#[async_trait]
impl ChannelBuilder<::rocket::Rocket<Ignite>> for RocketServer {
    async fn build_channel(&mut self) -> ActorResult<::rocket::Rocket<Ignite>> {
        if let Some(rocket) = self.rocket.take() {
            Ok(rocket)
        } else {
            log::error!("No provided rocket server to build");
            return Err(ActorError::exit_msg("No provided rocket server to build"));
        }
    }
}

#[async_trait]
impl<S: SupHandle<Self>> Actor<S> for RocketServer {
    type Data = String;
    type Channel = Rocket<Ignite>;

    async fn init(&mut self, rt: &mut Rt<Self, S>) -> ActorResult<Self::Data> {
        let name: String = rt.service().directory().clone().unwrap_or_else(|| "rocket".into());
        log::info!("{}: {}", name, rt.service().status());
        Ok(name)
    }

    async fn run(&mut self, rt: &mut Rt<Self, S>, name: Self::Data) -> ActorResult<()> {
        if let Some(rocket) = rt.inbox_mut().rocket() {
            log::info!("{} is {}", name, rt.service().status());
            let _ = rocket.launch().await.map_err(|e| {
                log::error!("{}: {}", name, e);
                ActorError::exit(e)
            })?;
        } else {
            unreachable!("the inbox must have server")
        };
        log::info!("{} gracefully shutdown", name);
        Ok(())
    }
}

/// Rocket CORS helper struct
pub struct CORS;

#[async_trait]
impl Fairing for CORS {
    fn info(&self) -> rocket::fairing::Info {
        Info {
            name: "Add CORS Headers",
            kind: Kind::Response,
        }
    }

    async fn on_response<'r>(&self, _request: &'r Request<'_>, response: &mut Response<'r>) {
        response.set_raw_header("Access-Control-Allow-Origin", "*");
        response.set_raw_header("Access-Control-Allow-Methods", "GET, OPTIONS");
        response.set_raw_header("Access-Control-Allow-Headers", "*");
        response.set_raw_header("Access-Control-Allow-Credentials", "true");
    }
}

/// Request timer helper struct
pub struct RequestTimer {
    requests_collector: IntCounter,
    response_code_collector: IntCounterVec,
    response_time_collector: HistogramVec,
}

impl Default for RequestTimer {
    fn default() -> Self {
        Self {
            requests_collector: IntCounter::new("incoming_requests", "Incoming Requests")
                .expect("failed to create metric"),
            response_code_collector: IntCounterVec::new(
                Opts::new("response_code", "Response Codes"),
                &["statuscode", "type"],
            )
            .expect("failed to create metric"),
            response_time_collector: HistogramVec::new(
                HistogramOpts::new("response_time", "Response Times"),
                &["endpoint"],
            )
            .expect("failed to create metric"),
        }
    }
}

#[derive(Copy, Clone)]
struct TimerStart(Option<SystemTime>);

#[rocket::async_trait]
impl Fairing for RequestTimer {
    fn info(&self) -> Info {
        Info {
            name: "Request Timer",
            kind: Kind::Request | Kind::Response,
        }
    }

    /// Stores the start time of the request in request-local state.
    async fn on_request(&self, request: &mut Request<'_>, _: &mut Data<'_>) {
        // Store a `TimerStart` instead of directly storing a `SystemTime`
        // to ensure that this usage doesn't conflict with anything else
        // that might store a `SystemTime` in request-local cache.
        request.local_cache(|| TimerStart(Some(SystemTime::now())));
        self.requests_collector.inc();
    }

    /// Adds a header to the response indicating how long the server took to
    /// process the request.
    async fn on_response<'r>(&self, req: &'r Request<'_>, res: &mut Response<'r>) {
        let start_time = req.local_cache(|| TimerStart(None));
        if let Some(Ok(duration)) = start_time.0.map(|st| st.elapsed()) {
            let ms = (duration.as_secs() * 1000 + duration.subsec_millis() as u64) as f64;
            self.response_time_collector
                .with_label_values(&[&format!("{} {}", req.method(), req.uri())])
                .observe(ms)
        }
        match res.status().code {
            500..=599 => self
                .response_code_collector
                .with_label_values(&[&res.status().code.to_string(), "500"])
                .inc(),
            400..=499 => self
                .response_code_collector
                .with_label_values(&[&res.status().code.to_string(), "400"])
                .inc(),
            300..=399 => self
                .response_code_collector
                .with_label_values(&[&res.status().code.to_string(), "300"])
                .inc(),
            200..=299 => self
                .response_code_collector
                .with_label_values(&[&res.status().code.to_string(), "200"])
                .inc(),
            100..=199 => self
                .response_code_collector
                .with_label_values(&[&res.status().code.to_string(), "100"])
                .inc(),
            _ => (),
        }
    }
}

#[catch(500)]
/// Returns internal error
pub fn internal_error() -> &'static str {
    "Internal server error!"
}

#[catch(404)]
/// Returns not found error
pub fn not_found() -> &'static str {
    "No endpoint found!"
}
