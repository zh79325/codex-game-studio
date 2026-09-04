use crate::client::HttpClient;
use crate::client::RequestBuilder;
use crate::error::TransportError;
use crate::request::Request;
use crate::request::RequestBody;
use crate::request::Response;
use bytes::Bytes;
use futures::StreamExt;
use futures::stream::BoxStream;
use http::HeaderMap;
use http::Method;
use http::StatusCode;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::OnceLock;
use tracing::Level;
use tracing::enabled;
use tracing::error;
use tracing::info;

pub type ByteStream = BoxStream<'static, Result<Bytes, TransportError>>;

/// An abnormal termination associated with a streaming response.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamResponseAuditEvent {
    StreamTerminated {
        stage: &'static str,
        reason: String,
    },
    EventConsumerDropped {
        stage: &'static str,
    },
    DeltaWithoutActiveItem {
        event_type: &'static str,
        delta_bytes: usize,
        action: &'static str,
    },
}

/// Receives abnormal streaming lifecycle events for request-scoped auditing.
pub trait StreamResponseAudit: Send + Sync {
    fn record_stream_event(&self, event: StreamResponseAuditEvent);
}

type StreamResponseAudits = HashMap<String, Arc<dyn StreamResponseAudit>>;

static STREAM_RESPONSE_AUDITS: OnceLock<Mutex<StreamResponseAudits>> = OnceLock::new();

pub fn register_stream_response_audit(thread_id: String, audit: Arc<dyn StreamResponseAudit>) {
    let audits = STREAM_RESPONSE_AUDITS.get_or_init(Default::default);
    if let Ok(mut audits) = audits.lock() {
        audits.insert(thread_id, audit);
    }
}

pub fn unregister_stream_response_audit(thread_id: &str) {
    let Some(audits) = STREAM_RESPONSE_AUDITS.get() else {
        return;
    };
    if let Ok(mut audits) = audits.lock() {
        audits.remove(thread_id);
    }
}

pub fn record_stream_response_audit_event(thread_id: &str, event: StreamResponseAuditEvent) {
    let audit = STREAM_RESPONSE_AUDITS
        .get()
        .and_then(|audits| audits.lock().ok()?.get(thread_id).cloned());
    if let Some(audit) = audit {
        audit.record_stream_event(event);
    }
}

fn stream_response_audit(req: &Request) -> Option<Arc<dyn StreamResponseAudit>> {
    let thread_id = req.headers.get("x-client-request-id")?.to_str().ok()?;
    STREAM_RESPONSE_AUDITS
        .get()?
        .lock()
        .ok()?
        .get(thread_id)
        .cloned()
}

pub struct StreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub bytes: ByteStream,
    pub audit: Option<Arc<dyn StreamResponseAudit>>,
}

pub trait HttpTransport: Send + Sync {
    fn execute(
        &self,
        req: Request,
    ) -> impl std::future::Future<Output = Result<Response, TransportError>> + Send;
    fn stream(
        &self,
        req: Request,
    ) -> impl std::future::Future<Output = Result<StreamResponse, TransportError>> + Send;
}

#[derive(Clone, Debug)]
pub struct ReqwestTransport {
    client: HttpClient,
}

impl ReqwestTransport {
    pub fn new(client: reqwest::Client) -> Self {
        Self {
            client: HttpClient::new(client),
        }
    }

    pub fn from_http_client(client: HttpClient) -> Self {
        Self { client }
    }

    fn build(&self, req: Request) -> Result<RequestBuilder, TransportError> {
        let prepared = req.prepare_body_for_send().map_err(TransportError::Build)?;

        let Request {
            method,
            url,
            headers: _,
            body: _,
            compression: _,
            timeout,
        } = req;

        let mut builder = self.client.request(
            Method::from_bytes(method.as_str().as_bytes()).unwrap_or(Method::GET),
            &url,
        );

        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }

        builder = builder.headers(prepared.headers);
        if let Some(body) = prepared.body {
            builder = builder.body(body);
        }
        Ok(builder)
    }

    fn map_error(err: reqwest::Error) -> TransportError {
        if err.is_connect() {
            TransportError::Connection(err.without_url())
        } else if err.is_timeout() {
            TransportError::Timeout
        } else {
            TransportError::Network(err.to_string())
        }
    }

    fn log_request(&self, req: &Request) {
        if self.client.request_logging_enabled() && enabled!(Level::INFO) {
            info!(
                method = %req.method,
                url = %req.url,
                request = %request_body_for_trace(req),
                "HTTP request"
            );
        }
    }

    fn log_error(&self, method: &Method, url: &str, error: &TransportError) {
        if self.client.request_logging_enabled() {
            error!(method = %method, url, error = %error, "HTTP request failed");
        }
    }
}

fn request_body_for_trace(req: &Request) -> String {
    match req.body.as_ref() {
        Some(RequestBody::Json(body)) => body.to_string(),
        Some(RequestBody::EncodedJson(body)) => {
            String::from_utf8_lossy(body.trace_bytes()).into_owned()
        }
        Some(RequestBody::Raw(body)) => format!("<raw body: {} bytes>", body.len()),
        None => String::new(),
    }
}

impl HttpTransport for ReqwestTransport {
    async fn execute(&self, req: Request) -> Result<Response, TransportError> {
        self.log_request(&req);

        let method = req.method.clone();
        let url = req.url.clone();
        let builder = self.build(req).inspect_err(|error| {
            self.log_error(&method, &url, error);
        })?;
        let resp = builder
            .send()
            .await
            .map_err(Self::map_error)
            .inspect_err(|error| self.log_error(&method, &url, error))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp
            .bytes()
            .await
            .map_err(Self::map_error)
            .inspect_err(|error| self.log_error(&method, &url, error))?;
        let body = String::from_utf8_lossy(&bytes);
        if self.client.request_logging_enabled() {
            info!(method = %method, url, %status, response = %body, "HTTP response");
        }
        if !status.is_success() {
            let error = TransportError::Http {
                status,
                url: Some(url.clone()),
                headers: Some(headers),
                body: Some(body.into_owned()),
            };
            self.log_error(&method, &url, &error);
            return Err(error);
        }
        Ok(Response {
            status,
            headers,
            body: bytes,
        })
    }

    async fn stream(&self, req: Request) -> Result<StreamResponse, TransportError> {
        self.log_request(&req);

        let stream_audit = stream_response_audit(&req);
        let method = req.method.clone();
        let url = req.url.clone();
        let builder = self.build(req).inspect_err(|error| {
            self.log_error(&method, &url, error);
        })?;
        let resp = builder
            .send()
            .await
            .map_err(Self::map_error)
            .inspect_err(|error| self.log_error(&method, &url, error))?;
        let status = resp.status();
        let headers = resp.headers().clone();
        if !status.is_success() {
            let body = resp.text().await.ok();
            let error = TransportError::Http {
                status,
                url: Some(url.clone()),
                headers: Some(headers),
                body,
            };
            self.log_error(&method, &url, &error);
            return Err(error);
        }
        let log_stream_errors = self.client.request_logging_enabled();
        let stream_method = method.clone();
        let stream_url = url.clone();
        let response_audit = stream_audit.clone();
        let stream = resp.bytes_stream().map(move |result| {
            result.map_err(|error| {
                let error = Self::map_error(error);
                if let Some(audit) = &response_audit {
                    audit.record_stream_event(StreamResponseAuditEvent::StreamTerminated {
                        stage: "http_transport",
                        reason: error.to_string(),
                    });
                }
                if log_stream_errors {
                    error!(
                        method = %stream_method,
                        url = %stream_url,
                        error = %error,
                        "HTTP stream failed"
                    );
                }
                error
            })
        });
        Ok(StreamResponse {
            status,
            headers,
            bytes: Box::pin(stream),
            audit: stream_audit,
        })
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
