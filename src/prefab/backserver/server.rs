// Copyright 2021 IOTA Stiftung
// Copyright 2022 Louay Kamel
// SPDX-License-Identifier: Apache-2.0

use super::BackserverEvent;
use crate::core::{Scope, ScopeId, Service, UnboundedHandle};
use hyper::{Body, Request, Response};

use std::{
    future::Future,
    pin::Pin,
    task::{Context, Poll},
};

pub(crate) struct ListenerSvc {
    /// The root scope_id, for now it's mostly 0
    #[allow(unused)]
    root_scope_id: ScopeId,
    backserver_handle: UnboundedHandle<BackserverEvent>,
}

impl hyper::service::Service<Request<Body>> for ListenerSvc {
    type Response = Response<Body>;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, req: Request<Body>) -> Self::Future {
        fn mk_response(s: String) -> Result<Response<Body>, hyper::Error> {
            Ok(Response::builder().body(Body::from(s)).unwrap())
        }

        if hyper_tungstenite::is_upgrade_request(&req) {
            let (response, websocket) = hyper_tungstenite::upgrade(req, None).unwrap();
            self.backserver_handle
                .send(BackserverEvent::HyperWebsocket(websocket))
                .ok();
            Box::pin(async { Ok(response) })
        } else {
            let res = match req.uri().path() {
                "/metrics" => {
                    use prometheus::Encoder;
                    let encoder = prometheus::TextEncoder::new();
                    let mut buffer = Vec::new();
                    if let Err(e) = encoder.encode(&prometheus::gather(), &mut buffer) {
                        log::error!("could not encode prometheus default metrics: {}", e);
                    };
                    let mut res = match String::from_utf8(buffer.clone()) {
                        Ok(v) => v,
                        Err(e) => {
                            log::error!("prometheus default metrics could not be from_utf8'd: {}", e);
                            String::default()
                        }
                    };
                    buffer.clear();
                    if let Err(e) = encoder.encode(&crate::core::PROMETHEUS_REGISTRY.gather(), &mut buffer) {
                        log::error!("could not encode prometheus default metrics: {}", e);
                    };
                    let res_custom = match String::from_utf8(buffer.clone()) {
                        Ok(v) => v,
                        Err(e) => {
                            log::error!("overclock metrics could not be from_utf8'd: {}", e);
                            String::default()
                        }
                    };
                    buffer.clear();
                    res.push_str(&res_custom);
                    mk_response(res)
                }
                "/service" => {
                    let scope_id = self.root_scope_id;
                    return Box::pin(async move {
                        if let Some(service) = Scope::lookup::<Service>(scope_id).await {
                            let res = serde_json::to_string(&service).unwrap_or("Unable to serialize service".into());
                            mk_response(res)
                        } else {
                            mk_response(format!("Service for scope_id: {}, not found", scope_id))
                        }
                    });
                }
                "/info" => mk_response(format!("overclock info, commit header, version and such.")),
                "/readme" => mk_response(format!("todo return readme.md")),
                "/docs" => mk_response(format!("todo return docs or redirect ot docs")),
                _ => return Box::pin(async { mk_response("oh no! not found".into()) }),
            };
            Box::pin(async { res })
        }
    }
}

pub(crate) struct MakeListenerSvc {
    root_scope_id: ScopeId,
    backserver_handle: UnboundedHandle<BackserverEvent>,
}

impl MakeListenerSvc {
    pub(crate) fn new(root_scope_id: ScopeId, backserver_handle: UnboundedHandle<BackserverEvent>) -> Self {
        Self {
            root_scope_id,
            backserver_handle,
        }
    }
}
impl<T> hyper::service::Service<T> for MakeListenerSvc {
    type Response = ListenerSvc;
    type Error = hyper::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, _: &mut Context) -> Poll<Result<(), Self::Error>> {
        Poll::Ready(Ok(()))
    }

    fn call(&mut self, _: T) -> Self::Future {
        let root_scope_id = self.root_scope_id.clone();
        let backserver_handle = self.backserver_handle.clone();
        let fut = async move {
            Ok(ListenerSvc {
                root_scope_id,
                backserver_handle,
            })
        };
        Box::pin(fut)
    }
}
