//! Pure, non-authorizing presentation of device observations.
use aquilum_core::sync::DeviceId;
use masonry::core::{NewWidget, PropertySet, WidgetId};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap, Padding};
use masonry::widgets::Flex;
use crate::i18n::t;
use crate::ui::tokens::size;
use crate::ui::widgets::{TextButton, Variant};
use super::{details, hint};

/// Read-only UI classification. These labels never convey Vault/File Gate
/// entitlement or grant authority to send. A signed owner registry is not
/// yet connected, so shipping Akula supplies no IdentityChecked rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceStatus {
    Unverified,
    IdentityChecked,
    Revoked,
}

#[derive(Clone, Debug)]
pub struct DeviceRow {
    pub id: DeviceId,
    pub key: [u8; 32],
    pub name: String,
    pub fingerprint: String,
    pub status: DeviceStatus,
    pub selected: bool,
}

pub(super) fn render(
    devices: &[DeviceRow],
    error: Option<&str>,
) -> (NewWidget<Flex>, Vec<(WidgetId, DeviceId, [u8; 32])>) {
    let mut device_actions = Vec::new();
    let mut info = Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details(&t("transfer.devices")));
    if devices.is_empty() {
        info = info.with_fixed(hint(&t("transfer.noDevices")));
    }
    // Device rows do not initiate transfers. An identity-checked key is not
    // an owner-signed Vault ACL; selecting it only prepares a UI draft.
    for device in devices {
        let state = match device.status {
            DeviceStatus::Unverified => t("transfer.deviceUnverified"),
            DeviceStatus::IdentityChecked => t("transfer.deviceIdentityChecked"),
            DeviceStatus::Revoked => t("transfer.deviceRevoked"),
        };
        let name = details(&device.name);
        let description = hint(&format!("{} · {}", device.fingerprint, state));
        let labels = NewWidget::new(
            Flex::column().cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_fixed(name).with_fixed(description),
        );
        let mut entry = Flex::row().cross_axis_alignment(CrossAxisAlignment::Center)
            .with(labels, 1.0);
        if device.status == DeviceStatus::IdentityChecked {
            let button = NewWidget::new(TextButton::new(
                if device.selected { t("transfer.deviceSelected") } else { t("transfer.selectDevice") },
                Variant::Ghost,
            ));
            device_actions.push((button.id(), device.id, device.key));
            entry = entry.with_fixed(button);
        }
        info = info.with_fixed(NewWidget::new(entry).with_props(
            PropertySet::new().with(Padding::all(Length::px(size::SPACE_8))),
        ));
    }
    info = info.with_fixed(hint(&t("transfer.deviceSelectionOnly")));
    info = info.with_fixed(hint(&t("transfer.offline")));
    if let Some(error) = error {
        info = info.with_fixed(hint(error));
    }
    let info = NewWidget::new(info).with_props(PropertySet::new()
        .with(Gap::new(Length::px(size::SPACE_4)))
        .with(Padding::all(Length::px(size::DIALOG_PADDING))));
    (info, device_actions)
}
