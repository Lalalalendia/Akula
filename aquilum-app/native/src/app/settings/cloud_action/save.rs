use super::*;
use crate::ui::settings::schema::CloudProvider;

#[cfg(feature = "dropbox-readonly-preview")]
fn matches_advanced_fields(config: &aquilum_core::settings::models::AppConfig, input: &[&str]) -> bool {
    // Invalid typed fields must not silently keep a stale Vault/HEAD pin.
    let d = &config.dropbox_readonly;
    let mut actual = [d.vault_id.as_str(), d.profile_id.as_str(), d.trusted_head_blake3.as_str()];
    if input.len() != actual.len() { return false; }
    let mut requested = input.to_vec();
    actual.sort_unstable();
    requested.sort_unstable();
    actual.as_slice() == requested.as_slice()
}

impl App {
    pub(super) fn save_cloud_popup(
        &mut self, root: &mut RenderRoot,
        mut dialog: SettingsDialog, cloud: CloudPopup,
    ) -> Outcome {
        if !matches!(cloud.provider, CloudProvider::Yandex | CloudProvider::Dropbox) {
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::None, root);
        }
        // Commit the visible popup draft atomically, not on keyboard focus loss.
        let texts = cloud.bindings.iter().filter_map(|(id, binding)| {
            let Binding::Text { setter, .. } = binding else { return None };
            let text = root.get_widget(*id)?.downcast::<TextEditor>()?.inner().text();
            Some((*setter, text))
        }).collect::<Vec<_>>();
        let before = self.workspace.core.settings.get_config();
        let mut proposed = before.clone();
        for (setter, text) in &texts {
            if let crate::ui::settings::schema::Setter::Config(apply) = setter {
                apply(&mut proposed, &Value::Text(text.clone()));
            }
        }
        #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
        if cloud.provider == CloudProvider::Yandex && texts.len() == 1
            && !aquilum_core::settings::models::YandexS3Onboarding::valid_bucket(&texts[0].1)
        {
            dialog.yandex_notice = Some(crate::i18n::t("settings.yandex.keysNeedBucket"));
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::Content, root);
        }
        if cloud.provider == CloudProvider::Yandex
            && !aquilum_core::settings::models::YandexS3Onboarding::valid_bucket(&proposed.s3_onboarding.bucket)
        {
            dialog.yandex_notice = Some(crate::i18n::t("settings.yandex.keysNeedBucket"));
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::Content, root);
        }
        #[cfg(feature = "dropbox-readonly-preview")]
        if cloud.provider == CloudProvider::Dropbox {
            let entered = texts.iter().map(|(_, val)| val.as_str()).collect::<Vec<_>>();
            if !matches_advanced_fields(&proposed, &entered) {
                self.dropbox_preview.status = "DROPBOX_INVALID_ADVANCED_CONFIG / SYNC_DISABLED".into();
                dialog.cloud_popup = Some(cloud);
                return self.keep(dialog, None, Effect::Content, root);
            }
        }
        let new_bucket = proposed.s3_onboarding.bucket.clone();
        #[cfg(feature = "dropbox-readonly-preview")]
        let dropbox_changed = before.dropbox_readonly != proposed.dropbox_readonly;
        if self.workspace.core.settings.update_config(proposed).is_err() {
            dialog.yandex_notice = Some(crate::i18n::t("settings.cloud.saveFailed"));
            #[cfg(feature = "dropbox-readonly-preview")]
            if cloud.provider == CloudProvider::Dropbox {
                self.dropbox_preview.status = "DROPBOX_ADVANCED_CONFIG_SAVE_FAILED / SYNC_DISABLED".into();
            }
            dialog.cloud_popup = Some(cloud);
            return self.keep(dialog, None, Effect::Content, root);
        }
        if before.s3_onboarding.bucket != new_bucket {
            self.yandex_probe.set_bucket(&new_bucket);
            #[cfg(all(target_os = "windows", feature = "s3-pair-beta"))]
            self.s3_pair.invalidate_verification();
        }
        #[cfg(feature = "dropbox-readonly-preview")]
        if dropbox_changed { self.dropbox_preview.invalidate(); }
        crate::ui::dialog::close(root, cloud.layer);
        root.focus_on(Some(dialog.layer));
        self.keep(dialog, None, Effect::Content, root)
    }
}

#[cfg(all(test, feature = "dropbox-readonly-preview"))]
mod tests {
    use super::*;
    use aquilum_core::settings::models::AppConfig;

    #[test]
    fn invalid_draft_is_rejected_instead_of_silently_keeping_stale_head() {
        let mut cfg = AppConfig::default();
        cfg.dropbox_readonly.vault_id = "12345678-1234-1234-1234-123456789abc".into();
        cfg.dropbox_readonly.profile_id = "test".into();
        cfg.dropbox_readonly.trusted_head_blake3 = "a".repeat(64);
        let values = [
            cfg.dropbox_readonly.vault_id.as_str(),
            cfg.dropbox_readonly.profile_id.as_str(),
            cfg.dropbox_readonly.trusted_head_blake3.as_str(),
        ];
        assert!(matches_advanced_fields(&cfg, &values));
        assert!(matches_advanced_fields(&cfg, &[values[2], values[0], values[1]]));
        assert!(!matches_advanced_fields(&cfg, &[values[0], "../bad", values[2]]));
        assert!(!matches_advanced_fields(&cfg, &[values[0], values[1]]));
    }
}
