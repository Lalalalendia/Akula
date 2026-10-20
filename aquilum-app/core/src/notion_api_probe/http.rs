//! Explicitly read-only HTTP adapter. The remote host is pinned; no custom URLs,
//! redirects, writes, mutation endpoints, or outbound calls from tests.
use super::{NotionReadTransport, ReadRequest, API_VERSION};
use serde_json::Value;
use std::io::Read;
use std::time::Duration;

pub struct HttpReadClient {
    token: String,
    agent: ureq::Agent,
}
impl HttpReadClient {
    pub fn new(token: String) -> Result<Self, String> {
        if token.is_empty() || token.bytes().any(|b| b == b'\r' || b == b'\n') {
            return Err("invalid credential".into());
        }
        Ok(Self {
            token,
            agent: ureq::AgentBuilder::new()
                .timeout(Duration::from_secs(25))
                .redirects(0)
                .build(),
        })
    }
}
fn path_of(req: &ReadRequest) -> (bool, String, Option<Value>) {
    let encode = |s: &str| s.replace('%', "%25").replace('/', "%2F").replace('#', "%23");
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
impl NotionReadTransport for HttpReadClient {
    fn read(&mut self, req: &ReadRequest) -> Result<Value, String> {
        let (post, path, body) = path_of(req);
        let url = format!("https://api.notion.com/v1/{path}");
        let auth = format!("Bearer {}", self.token);
        let builder = if post { self.agent.post(&url) } else { self.agent.get(&url) }
            .set("Authorization", &auth)
            .set("Notion-Version", API_VERSION)
            .set("Content-Type", "application/json");
        let response = match body {
            Some(json) => builder.send_json(json),
            None => builder.call(),
        };
        let response = match response {
            Ok(value) => value,
            Err(ureq::Error::Status(code, _)) => {
                // Do NOT log body: it may include user data.
                return Err(format!("Notion returned HTTP {code}; partial result rejected"));
            }
            Err(ureq::Error::Transport(_)) => return Err("Notion transport failed; no data accepted".into()),
        };
        let mut body = String::new();
        response.into_reader().take(8 * 1024 * 1024 + 1)
            .read_to_string(&mut body).map_err(|_| "Notion returned invalid UTF-8 or I/O error")?;
        if body.len() > 8 * 1024 * 1024 { return Err("Notion API response too large".into()); }
        serde_json::from_str(&body).map_err(|_| "Notion returned invalid JSON".into())
    }
}
