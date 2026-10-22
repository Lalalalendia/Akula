use super::Method;
use crate::i18n::t;

impl Method {
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
