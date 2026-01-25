use crate::error::SseError;
use crate::event::SseEvent;
use crate::parser::SseParser;
use async_stream::try_stream;
use bytes::Bytes;
use futures_core::Stream;
use futures_util::StreamExt;
use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, ACCEPT_ENCODING, CONTENT_TYPE};
use reqwest::{Method, Url};
use std::time::Duration;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

/// 重试配置：基础间隔 + 最大次数（None 表示无限重试）
#[derive(Debug, Clone)]
pub struct SseRetry {
    pub base_delay: Duration,
    pub max_retries: Option<usize>,
}

impl Default for SseRetry {
    fn default() -> Self {
        Self {
            base_delay: Duration::from_millis(1000),
            max_retries: None,
        }
    }
}

/// 一次 SSE 请求（方向 A：允许自定义 method/headers/body）
#[derive(Debug, Clone)]
pub struct SseRequest {
    pub url: String,
    pub method: Method,
    pub headers: HeaderMap,
    pub body: Option<Bytes>,
    pub retry: SseRetry,
    /// 初始 Last-Event-ID（重连时会自动更新为最近一次收到的 id）
    pub last_event_id: Option<String>,
    /// 是否主动禁用压缩（避免某些链路下的 buffering/decode 影响流式体验）
    pub accept_identity_encoding: bool,
}

impl SseRequest {
    pub fn get(url: impl Into<String>) -> Self {
        Self {
            url: url.into(),
            method: Method::GET,
            headers: HeaderMap::new(),
            body: None,
            retry: SseRetry::default(),
            last_event_id: None,
            accept_identity_encoding: true,
        }
    }
}

/// SSE 客户端：基于 reqwest 的 streaming body + SseParser
#[derive(Clone, Debug, Default)]
pub struct SseClient {
    http: reqwest::Client,
}

impl SseClient {
    pub fn new() -> Self {
        Self {
            http: reqwest::Client::new(),
        }
    }

    /// 返回一个 Stream：不断产出解析后的 `SseEvent`。
    ///
    /// - 会自动设置 `Accept: text/event-stream`
    /// - 会在重连时带上 `Last-Event-ID`
    /// - 若收到事件里包含 `retry` 字段，会更新下一次重连等待时间
    pub fn stream(self, req: SseRequest) -> impl Stream<Item = Result<SseEvent, SseError>> {
        let client = self.http.clone();

        try_stream! {
            let url: Url = req.url.parse().map_err(|e: <Url as std::str::FromStr>::Err| {
                error!(url = %req.url, error = %e, "Failed to parse SSE URL");
                SseError::Url(e.to_string())
            })?;
            let mut last_event_id = req.last_event_id.clone();
            let mut retry_delay = req.retry.base_delay;
            let mut retries = 0usize;

            info!(url = %url, "Starting SSE connection");

            loop {
                // 超过最大重试次数则停止
                if let Some(max) = req.retry.max_retries {
                    if retries > max {
                        warn!(url = %url, retries = retries, max_retries = max, "Max retries exceeded, stopping");
                        break;
                    }
                }

                let mut headers = req.headers.clone();
                headers.insert(ACCEPT, HeaderValue::from_static("text/event-stream"));
                if req.accept_identity_encoding {
                    headers.insert(ACCEPT_ENCODING, HeaderValue::from_static("identity"));
                }
                if let Some(id) = &last_event_id {
                    // 一些服务端大小写不敏感，但这里用规范写法
                    let v = HeaderValue::from_str(id).map_err(|_| SseError::InvalidHeaderValue)?;
                    headers.insert("Last-Event-ID", v);
                    debug!(url = %url, last_event_id = %id, "Setting Last-Event-ID header");
                }

                let mut rb = client.request(req.method.clone(), url.clone()).headers(headers);
                if let Some(body) = &req.body {
                    rb = rb.body(body.clone());
                }

                debug!(url = %url, method = %req.method, retry = retries, "Sending SSE request");

                let resp = match rb.send().await {
                    Ok(resp) => resp,
                    Err(e) => {
                        error!(url = %url, error = %e, retry = retries, "HTTP request failed");
                        Err(e)?
                    }
                };

                // 默认校验 content-type
                let content_type = resp
                    .headers()
                    .get(CONTENT_TYPE)
                    .and_then(|v| v.to_str().ok())
                    .map(|s| s.to_string());
                if content_type
                    .as_deref()
                    .map(|ct| ct.starts_with("text/event-stream"))
                    != Some(true)
                {
                    error!(url = %url, content_type = ?content_type, "Invalid content-type, expected text/event-stream");
                    Err(SseError::InvalidContentType(content_type))?;
                }

                info!(url = %url, "SSE connection established");

                let mut parser = SseParser::new();
                let mut body_stream = resp.bytes_stream();

                while let Some(next) = body_stream.next().await {
                    let chunk = match next {
                        Ok(chunk) => chunk,
                        Err(e) => {
                            warn!(url = %url, error = %e, "Error reading SSE stream chunk");
                            Err(e)?
                        }
                    };
                    for ev in parser.push(&chunk) {
                        // 更新 last_event_id 以便下次重连
                        if let Some(id) = &ev.id {
                            if id.is_empty() {
                                last_event_id = None;
                            } else {
                                last_event_id = Some(id.clone());
                            }
                        }
                        // 若服务端发送 retry，按 spec 更新重连间隔
                        if let Some(ms) = ev.retry {
                            debug!(url = %url, retry_ms = ms, "Server requested retry interval update");
                            retry_delay = Duration::from_millis(ms);
                        }

                        yield ev;
                    }
                }

                // 连接正常结束：按 retry_delay 进行重连
                retries += 1;
                info!(url = %url, retry = retries, delay_ms = retry_delay.as_millis(), "Connection ended, scheduling reconnect");
                sleep(retry_delay).await;
            }
        }
    }
}
