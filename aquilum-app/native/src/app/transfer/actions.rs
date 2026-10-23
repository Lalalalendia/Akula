//! UI-only transfer dispatch. Never creates a QUIC connection or file grant.
use super::*;
use crate::app::Outcome;
use crate::ui::dialog::{self, DialogAnswer};
use crate::ui::transfer::QueueAction;
use crate::ui::widgets::Pressed;
use masonry::app::RenderRoot;
use masonry::core::{ErasedAction, WidgetId};

impl App {
    pub(in crate::app) fn on_transfer_action(
        &mut self,
        root: &mut RenderRoot,
        id: WidgetId,
        action: &ErasedAction,
    ) -> Outcome {
        let Some(Popup::Transfers(dialog)) = self.popup.take() else {
            return Outcome::default();
        };
        let pressed = action.downcast_ref::<Pressed>().is_some();
        let dismiss = (id == dialog.layer
            && matches!(
                action.downcast_ref::<DialogAnswer>(),
                Some(DialogAnswer::Cancel)
            ))
            || (id == dialog.close && pressed);
        if dismiss {
            dialog::close(root, dialog.layer);
            return Outcome::default();
        }
        let mut refresh = false;
        if id == dialog.choose && pressed {
            self.choose_transfer_files();
            refresh = true;
        } else if pressed
            && let Some((_, index, command)) =
                dialog.actions.iter().find(|(button, _, _)| *button == id)
        {
            match command {
                QueueAction::Toggle => self.transfer.toggle(*index),
                QueueAction::Remove => self.transfer.remove(*index),
            }
            refresh = true;
        } else if pressed
            && let Some((_, device_id, key)) = dialog
                .device_actions
                .iter()
                .find(|(button, _, _)| *button == id)
        {
            self.transfer.choose_peer((*device_id, *key));
            refresh = true;
        }
        #[cfg(feature = "p2p-device-ui")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog
                .identity_actions
                .iter()
                .find(|(button, _)| *button == id)
            {
                self.transfer
                    .local_identity
                    .action(*command, Arc::clone(&self.workspace.wake));
                refresh = true;
            }
        }
        #[cfg(feature = "p2p-device-ui")]
        if !refresh {
            if let Some(controls) = &dialog.peer_controls {
                refresh = self.transfer.peer_pair.handle(
                    controls,
                    id,
                    action,
                    Arc::clone(&self.workspace.wake),
                );
            }
        }
        self.popup = Some(Popup::Transfers(dialog));
        if refresh {
            self.refresh_transfers(root);
        }
        Outcome::default()
    }
}
