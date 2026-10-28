//! Transfer dialog action dispatch; no network work occurs without a click.
use super::*;

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
            #[cfg(feature = "p2p-native-send-beta")]
            self.transfer.sender.cancel();
            #[cfg(feature = "p2p-native-receive-beta")]
            self.transfer.receiver.cancel();
            ui_dialog::close(root, dialog.layer);
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
        #[cfg(feature = "p2p-inbox-preview")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog
                .inbox_save_actions
                .iter()
                .find(|(button, _)| *button == id)
            {
                refresh = match command {
                    transfer_ui::InboxAction::Preview(receipt) => self.begin_inbox_text_preview(receipt),
                    transfer_ui::InboxAction::SaveCopy(receipt) => self.begin_inbox_save(receipt),
                    #[cfg(windows)]
                    transfer_ui::InboxAction::Import(receipt) => self.begin_inbox_import(receipt),
                };
            }
        }
        #[cfg(feature = "p2p-native-send-beta")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog
                .native_send_actions
                .iter()
                .find(|(button, _)| *button == id)
            {
                refresh = match command {
                    transfer_ui::NativeSendAction::Start(peer) => self.begin_native_send(peer),
                    transfer_ui::NativeSendAction::StartInternet(peer) => {
                        self.begin_native_send_internet(peer)
                    }
                    transfer_ui::NativeSendAction::Cancel => {
                        self.transfer.sender.cancel();
                        true
                    }
                };
            }
        }
        #[cfg(feature = "p2p-native-receive-beta")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog
                .native_receive_actions
                .iter()
                .find(|(button, _)| *button == id)
            {
                refresh = match command {
                    transfer_ui::NativeReceiveAction::Start(peer) => {
                        self.begin_native_receive(peer)
                    }
                    transfer_ui::NativeReceiveAction::StartInternet(peer) => {
                        self.begin_native_receive_internet(peer)
                    }
                    transfer_ui::NativeReceiveAction::Cancel => {
                        self.transfer.receiver.cancel();
                        true
                    }
                };
            }
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
        #[cfg(feature = "messaging-w4-pilot")]
        if pressed && !refresh {
            if let Some((_, command)) = dialog.w4_actions.iter().find(|(button, _)| *button == id) {
                self.transfer
                    .w4_pilot
                    .action(*command, Arc::clone(&self.workspace.wake));
                refresh = true;
            }
        }
        self.popup = Some(Popup::Transfers(dialog));
        if refresh {
            self.refresh_transfers(root);
        }
        Outcome::default()
    }
}
