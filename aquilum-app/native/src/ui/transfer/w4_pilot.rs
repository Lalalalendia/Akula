//! Pilot messaging controls inside Akula's existing Transfer dialog.
//! Explicit opt-in only. No background listener, auto-enrollment or Vault grant.
use masonry::core::{NewWidget,PropertySet,WidgetId};
use masonry::layout::Length;
use masonry::properties::types::CrossAxisAlignment;
use masonry::properties::{Gap,Padding};
use masonry::widgets::Flex;

use super::{details,hint};
use crate::ui::tokens::size;
use crate::ui::widgets::{TextButton,Variant};

#[derive(Clone,Copy,Debug,PartialEq,Eq)]
pub enum PilotAction {
    Enable, Disable, Check, InitSender, InitRecipient,
    CopySender, CopyRecipient, PasteInvitation, ConfirmSend, Retry,
}

#[derive(Clone,Debug,Default)]
pub struct PilotView {
    pub enabled: bool,
    pub busy: bool,
    pub notice: Option<String>,
    pub sender_public: Option<String>,
    pub recipient_invite: Option<String>,
    pub recipient_to_confirm: Option<String>,
    pub last_message: Option<String>,
}

fn button(a:&mut Vec<(WidgetId,PilotAction)>,label:&str,act:PilotAction)
    ->NewWidget<TextButton> {
    let w=NewWidget::new(TextButton::new(label,Variant::Ghost));
    a.push((w.id(),act));
    w
}

pub(super) fn render(view:&PilotView)->(NewWidget<Flex>,Vec<(WidgetId,PilotAction)>){
    let mut actions=Vec::new();
    let mut card=Flex::column()
        .cross_axis_alignment(CrossAxisAlignment::Stretch)
        .with_fixed(details("Акула · Зашифрованная переписка (испытания)"))
        .with_fixed(hint("Экспериментальная W3/W4: без доступа к заметкам, без автоматического подключения."))
        .with_fixed(hint("Для доставки через Интернет заранее откройте доверенный SSH-туннель на 127.0.0.1:42043."));
    if !view.enabled {
        card=card.with_fixed(hint("Функция отключена до вашего явного включения."))
            .with_fixed(button(&mut actions,"Включить испытательный режим",PilotAction::Enable));
    } else if view.busy {
        card=card.with_fixed(hint("Выполняется защищённая операция..."));
    } else {
        card=card.with_fixed(button(&mut actions,"Проверить W4",PilotAction::Check))
            .with_fixed(button(&mut actions,"Создать ключи отправителя",PilotAction::InitSender))
            .with_fixed(button(&mut actions,"Создать ключи получателя",PilotAction::InitRecipient));
        if let Some(key)=&view.sender_public {
            card=card.with_fixed(hint(&format!("Открытый ключ отправителя: {key}")))
                .with_fixed(button(&mut actions,"Скопировать ключ",PilotAction::CopySender));
        }
        if view.recipient_invite.is_some() {
            card=card.with_fixed(hint("Приглашение получателя создано (только открытые данные)."))
                .with_fixed(button(&mut actions,"Скопировать приглашение",PilotAction::CopyRecipient));
        }
        card=card.with_fixed(button(&mut actions,"Взять приглашение из буфера",PilotAction::PasteInvitation));
        if let Some(invite)=&view.recipient_to_confirm {
            card=card.with_fixed(hint(&format!("Сверьте ключ получателя (первые 64 символа): {}",&invite[..64])))
                .with_fixed(hint("Отправка начнётся только после второго нажатия."))
                .with_fixed(button(&mut actions,"Подтверждаю получателя — отправить 1 КиБ",PilotAction::ConfirmSend));
        }
        if let Some(message)=&view.last_message {
            card=card.with_fixed(hint(&format!("Последнее письмо: {message}")))
                .with_fixed(button(&mut actions,"Повторить то же письмо",PilotAction::Retry));
        }
        card=card.with_fixed(button(&mut actions,"Выключить режим",PilotAction::Disable));
    }
    if let Some(notice)=&view.notice {
        card=card.with_fixed(hint(notice));
    }
    (NewWidget::new(card).with_props(
        PropertySet::new().with(Gap::new(Length::px(size::SPACE_4)))
            .with(Padding::all(Length::px(size::DIALOG_PADDING)))
    ),actions)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn disabled_has_only_explicit_enable_action(){
        let (_,actions)=render(&PilotView::default());
        assert_eq!(actions.len(),1);
        assert_eq!(actions[0].1,PilotAction::Enable);
    }
    #[test]
    fn cannot_send_without_staged_invitation(){
        let view=PilotView{enabled:true,..Default::default()};
        let (_,actions)=render(&view);
        assert!(!actions.iter().any(|(_,a)|*a==PilotAction::ConfirmSend));
        assert!(!actions.iter().any(|(_,a)|*a==PilotAction::Retry));
    }
    #[test]
    fn explicit_staging_adds_send_but_does_not_start_network(){
        let view=PilotView{enabled:true,recipient_to_confirm:Some("a".repeat(160)),..Default::default()};
        let (_,actions)=render(&view);
        assert!(actions.iter().any(|(_,a)|*a==PilotAction::ConfirmSend));
    }
}
