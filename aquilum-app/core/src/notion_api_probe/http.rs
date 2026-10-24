//! Strictly read-only Notion HTTP adapter.
//! Host pinned, redirects disabled, responses bounded, no mutating endpoints.
//! A read-query POST is safe to retry, unlike general POST mutations.
use super::{NotionReadTransport, ReadRequest, API_VERSION};
use serde_json::Value;
use std::io::Read;
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use zeroize::Zeroizing;

const MAX_ATTEMPTS: usize = 5;
const MAX_RESPONSE_BYTES: u64 = 8 * 1024 * 1024;
const MAX_COOLDOWN_SECONDS: u64 = 120;
const MIN_REQUEST_SPACING: Duration = Duration::from_millis(400);

pub struct HttpReadClient {
    token: Zeroizing<String>,
    agent: ureq::Agent,
    next_allowed: Instant,
}
impl HttpReadClient {
    pub fn new(token: String) -> Result<Self, String> {
        if token.is_empty() || token.len() > 8192 || token.bytes().any(|b| !b.is_ascii_graphic()) {
            return Err("Notion credential must contain printable ASCII without spaces or controls".into());
        }
        Ok(Self {
            token: Zeroizing::new(token),
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(25))
                .redirects(0)
                .build(),
            next_allowed: Instant::now(),
        })
    }

    // This client is &mut, intentionally serial: no burst and no hidden concurrency.
    // When a shared daemon is introduced, its scheduler must apply an additional
    // per-workspace/connection budget across every client instance.
    fn pace(&mut self) {
        let now = Instant::now();
        if now < self.next_allowed {
            thread::sleep(self.next_allowed.duration_since(now));
        }
        self.next_allowed = Instant::now() + MIN_REQUEST_SPACING;
    }
}

fn percent_encode(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
            result.push(char::from(b));
        } else {
            result.push('%');
            result.push_str(&format!("{b:02X}"));
        }
    }
    result
}
fn path_of(req: &ReadRequest) -> (bool, String, Option<Value>) {
    let encode = percent_encode;
    match req {
        ReadRequest::Page(id) => (false, format!("pages/{}", encode(id)), None),
        ReadRequest::Markdown(id) => (false, format!("pages/{}/markdown", encode(id)), None),
        ReadRequest::BlockChildren { id, cursor } => {
            (false, format!("blocks/{}/children?page_size=100{}", encode(id),
                cursor.as_ref().map(|s| format!("&start_cursor={}", encode(s))).unwrap_or_default()), None)
        }
        ReadRequest::Database(id) => (false, format!("databases/{}", encode(id)), None),
        ReadRequest::DataSource(id) => (false, format!("data_sources/{}", encode(id)), None),
        ReadRequest::DataSourceRows { id, cursor } => {
            let mut body = serde_json::json!({"page_size":100});
            if let Some(s) = cursor { body["start_cursor"] = Value::String(s.clone()); }
            (true, format!("data_sources/{}/query", encode(id)), Some(body))
        }
        ReadRequest::Views { id, cursor } => {
            (false, format!("views?database_id={}&page_size=100{}", encode(id),
                cursor.as_ref().map(|s| format!("&start_cursor={}", encode(s))).unwrap_or_default()), None)
        }
        ReadRequest::View(id) => (false, format!("views/{}", encode(id)), None),
        ReadRequest::PageProperty { page, property, cursor } => {
            (false, format!("pages/{}/properties/{}?page_size=100{}", encode(page), encode(property),
                cursor.as_ref().map(|s| format!("&start_cursor={}", encode(s))).unwrap_or_default()), None)
        }
    }
}
fn backoff(attempt: usize, header: Option<&str>) -> Result<Duration, String> {
    let exponential = 1_u64 << attempt.min(5);
    let requested = header.and_then(|s| s.parse::<u64>().ok()).unwrap_or(exponential);
    if requested > MAX_COOLDOWN_SECONDS {
        return Err("Notion Retry-After exceeds synchronous probe wait budget: stop without retry".into());
    }
    // Small randomized staggering prevents several clients from retrying in lockstep.
    let jitter = SystemTime::now().duration_since(UNIX_EPOCH)
        .map(|t| u64::from(t.subsec_nanos()) % 250).unwrap_or(0);
    Ok(Duration::from_secs(requested) + Duration::from_millis(jitter))
}
fn retryable_status(code: u16) -> bool {
    matches!(code, 429 | 529 | 500 | 502 | 503 | 504)
}
fn blocked_by_notion(response: ureq::Response) -> bool {
    let mut buf = String::new();
    if response.into_reader().take(16 * 1024 + 1).read_to_string(&mut buf).is_err()
        || buf.len() > 16 * 1024 {
        return false;
    }
    serde_json::from_str::<Value>(&buf).ok()
        .and_then(|v| v.get("additional_data")?.get("rate_limit_reason")?.as_str().map(str::to_owned))
        .as_deref() == Some("public_api_request_blocked")
}
impl NotionReadTransport for HttpReadClient {
    fn read(&mut self, req: &ReadRequest) -> Result<Value, String> {
        let (post, path, json) = path_of(req);
        let url = format!("https://api.notion.com/v1/{path}");
        let auth = format!("Bearer {}", self.token.as_str());
        let mut last_failure = String::from("Notion retry budget exhausted");
        for attempt in 0..MAX_ATTEMPTS {
            self.pace();
            let builder = if post { self.agent.post(&url) } else { self.agent.get(&url) }
                .set("Authorization", &auth)
                .set("Notion-Version", API_VERSION)
                .set("Content-Type", "application/json");
            let response = match &json {
                Some(payload) => {
                    let serialized = serde_json::to_string(payload)
                        .map_err(|_| "cannot serialize Notion read query")?;
                    builder.send_string(&serialized)
                },
                None => builder.call(),
            };
            match response {
                Ok(response) => {
                    let mut body = String::new();
                    response.into_reader().take(MAX_RESPONSE_BYTES + 1)
                        .read_to_string(&mut body)
                        .map_err(|_| "Notion returned invalid UTF-8 or an I/O error")?;
                    if body.len() as u64 > MAX_RESPONSE_BYTES {
                        return Err("Notion response exceeds bounded JSON size".into());
                    }
                    return serde_json::from_str(&body)
                        .map_err(|_| "Notion returned invalid JSON".into());
                }
                Err(ureq::Error::Status(code, response)) => {
                    // A 429 that actually means "blocked" is NOT transient.
                    let retry_after = response.header("Retry-After").map(str::to_owned);
                    let blocked = code == 429 && blocked_by_notion(response);
                    if blocked || !retryable_status(code) {
                        return Err(format!("Notion HTTP {code}: not retryable; snapshot rejected"));
                    }
                    last_failure = format!("Notion HTTP {code}: retry budget exceeded");
                    if attempt + 1 == MAX_ATTEMPTS { break; }
                    thread::sleep(backoff(attempt, retry_after.as_deref())?);
                }
                Err(ureq::Error::Transport(_)) => {
                    last_failure = "Notion network error: retry budget exceeded".into();
                    if attempt + 1 == MAX_ATTEMPTS { break; }
                    thread::sleep(backoff(attempt, None)?);
                }
            }
        }
        Err(last_failure)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn url_encoding_does_not_allow_cursor_query_injection() {
        let (_, path, _) = path_of(&ReadRequest::BlockChildren {
            id: "11111111-1111-1111-1111-111111111111".into(),
            cursor: Some("abc&max=100#bad+tail/=".into()),
        });
        assert!(path.contains("start_cursor=abc%26max%3D100%23bad%2Btail%2F%3D"));
        assert!(!path.contains("&max="));
    }
    #[test]
    fn encoded_property_id_cannot_change_route() {
        let (_, path, _) = path_of(&ReadRequest::PageProperty {
            page: "11111111-1111-1111-1111-111111111111".into(),
            property: "abc/%?#".into(), cursor: None,
        });
        assert!(path.contains("properties/abc%2F%25%3F%23?"));
    }
    #[test]
    fn only_retry_safe_transient_statuses() {
        assert!(retryable_status(429));
        assert!(retryable_status(529));
        assert!(retryable_status(500));
        assert!(!retryable_status(403));
        assert!(!retryable_status(401));
        assert!(!retryable_status(400));
    }
    #[test]
    fn retry_after_is_respected_without_clamping_or_busy_retry() {
        let d = backoff(0, Some("75")).unwrap();
        assert!(d >= Duration::from_secs(75) && d < Duration::from_secs(76));
        assert!(backoff(0, Some("180")).is_err());
        let d = backoff(3, None).unwrap();
        assert!(d >= Duration::from_secs(8) && d < Duration::from_secs(9));
    }
}
