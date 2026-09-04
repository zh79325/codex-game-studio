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
use tracing::Level;
use tracing::enabled;
use tracing::error;
use tracing::info;

pub type ByteStream = BoxStream<'static, Result<Bytes, TransportError>>;

pub struct StreamResponse {
    pub status: StatusCode,
    pub headers: HeaderMap,
    pub bytes: ByteStream,
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
        if self.client.request_logging_enabled() {
            info!(method = %method, url, %status, response_headers = ?headers, "HTTP stream opened");
        }
        let log_response = self.client.request_logging_enabled() && enabled!(Level::INFO);
        let stream_method = method.clone();
        let stream_url = url.clone();
        let stream = resp.bytes_stream().map(move |result| match result {
            Ok(bytes) => {
                if log_response {
                    info!(
                        method = %stream_method,
                        url = %stream_url,
                        response = %String::from_utf8_lossy(&bytes),
                        "HTTP stream response"
                    );
                }
                Ok(bytes)
            }
            Err(error) => {
                let error = Self::map_error(error);
                if log_response {
                    error!(
                        method = %stream_method,
                        url = %stream_url,
                        error = %error,
                        "HTTP stream failed"
                    );
                }
                Err(error)
            }
        });
        Ok(StreamResponse {
            status,
            headers,
            bytes: Box::pin(stream),
        })
    }
}

#[cfg(test)]
#[path = "transport_tests.rs"]
mod tests;
