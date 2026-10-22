use super::Method;
use crate::i18n::t;

impl Method {
    pub const ORDER: [Method; 3] = [Method::Bm25f, Method::AdamicAdar, Method::Wikixiv];

    pub fn key(self) -> &'static str {
        match self {
            Method::Bm25f => "bm25f",
            Method::AdamicAdar => "adamicAdar",
            Method::Wikixiv => "wixiv",
        }
    }

    pub fn from_key(key: &str) -> Option<Method> {
        Method::ORDER.into_iter().find(|m| m.key() == key)
    }

    pub fn label(self) -> String {
        t(&format!("analysis.methods.{}", self.key()))
    }

    pub fn enabled(config: &aquilum_core::settings::models::AppConfig) -> Vec<Method> {
        let a = &config.analysis;
        Method::ORDER
            .into_iter()
            .filter(|m| match m {
                Method::Bm25f => a.enable_bm25f,
                Method::AdamicAdar => a.enable_adamic_adar,
                Method::Wikixiv => a.enable_wikixiv,
            })
            .collect()
    }
}
