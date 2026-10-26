//! Async adapter: GUI -> explicit *Akula native* --w4-pilot CLI -> pinned WOW.
 //! There is no automatic listener or trust-on-first-use. In-app MVP only
//! sends a fixed test letter after the user has explicitly reviewed a key.

use std::{process::{Command,Stdio},sync::{Arc,mpsc::{channel,Receiver,Sender}}};
use crate::ui::transfer::{PilotAction,PilotView};

#[derive(Clone,Copy,Debug)]
enum Work { Status, InitSender, InitRecipient, Send, Resend }

struct Finished {
    version:u64, action:Work, good:bool,
    stdout:String, stderr:String,
}

pub(super) struct PilotUi {
    pub(super) view:PilotView,
    generation:u64,
    tx:Sender<Finished>,
    rx:Receiver<Finished>,
}
impl Default for PilotUi {
    fn default()->Self {
        let (tx,rx)=channel();
        Self{view:PilotView::default(),generation:0,tx,rx}
    }
}
fn hex(input:&str,size:usize)->bool {
    input.len()==size && input.bytes().all(|b|b.is_ascii_hexdigit())
}
fn key_line(text:&str,name:&str,size:usize)->Option<String> {
    let mut hits=text.lines().filter_map(|line|line.strip_prefix(name));
    let first=hits.next()?;
    if hits.next().is_some() || !hex(first,size) {return None;}
    Some(first.to_ascii_lowercase())
}
fn copy(text:&str)->bool {
    arboard::Clipboard::new().and_then(|mut c|c.set_text(text.to_owned())).is_ok()
}
fn task_args(work:Work,invite:Option<String>,id:Option<String>)->Option<Vec<String>> {
    Some(match work {
        Work::Status=>vec!["status".into()],
        Work::InitSender=>vec!["init-sender".into()],
        Work::InitRecipient=>vec!["init-recipient".into()],
        Work::Send=>vec!["send-test".into(),invite.filter(|s|hex(s,160))?],
        Work::Resend=>vec!["resend".into(),invite.filter(|s|hex(s,160))?,id.filter(|s|hex(s,32))?],
    })
}
impl PilotUi {
    pub(super) fn action(&mut self,action:PilotAction,wake:Arc<dyn Fn()+Send+Sync>) {
        if action==PilotAction::Enable {
            if !self.view.enabled {self.view.enabled=true;self.view.notice=Some("Режим включён только в этом окне. Сверяйте полный ключ вручную.".into());}
            return;
        }
        if self.view.busy || !self.view.enabled {return;}
        if action==PilotAction::Disable {
            self.view=PilotView::default();
            return;
        }
        match action {
            PilotAction::CopySender=>{
                self.view.notice=Some(match self.view.sender_public.as_deref(){
                    Some(s) if hex(s,64) && copy(s)=>"Открытый ключ скопирован.",
                    _=>"Нет проверенного открытого ключа для копирования.",
                }.into());
                return;
            },
            PilotAction::CopyRecipient=>{
                self.view.notice=Some(match self.view.recipient_invite.as_deref(){
                    Some(s) if hex(s,160) && copy(s)=>"Приглашение скопировано.",
                    _=>"Нет проверенного приглашения для копирования.",
                }.into());
                return;
            },
            PilotAction::PasteInvitation=>{
                let data=arboard::Clipboard::new().and_then(|mut c|c.get_text());
                match data {
                    Ok(v) if hex(v.trim(),160) => {
                        self.view.recipient_to_confirm=Some(v.trim().to_ascii_lowercase());
                        self.view.last_message=None;
                        self.view.notice=Some("Приглашение загружено. Сверьте открытый ключ получателя и нажмите подтверждение.".into());
                    },
                    _=>{
                        self.view.recipient_to_confirm=None;
                        self.view.last_message=None;
                        self.view.notice=Some("Ожидается строго 160 шестнадцатеричных символов.".into());
                    }
                }
                return;
            },
            _=>{},
        }
        let (work,invite,id)=match action {
            PilotAction::Check=>(Work::Status,None,None),
            PilotAction::InitSender=>(Work::InitSender,None,None),
            PilotAction::InitRecipient=>(Work::InitRecipient,None,None),
            PilotAction::ConfirmSend=>{
                let Some(invite)=self.view.recipient_to_confirm.clone() else{return;};
                (Work::Send,Some(invite),None)
            },
            PilotAction::Retry=>{
                let (Some(invite),Some(id))=(
                    self.view.recipient_to_confirm.clone(),
                    self.view.last_message.clone(),
                )else{return;};
                (Work::Resend,Some(invite),Some(id))
            },
            _=>return,
        };
        let Some(args)=task_args(work,invite,id) else{return;};
        self.generation=self.generation.wrapping_add(1);
        let version=self.generation;
        let tx=self.tx.clone();
        self.view.busy=true;
        self.view.notice=None;
        std::thread::spawn(move || {
            // Launch only the main Akula executable with a finite known CLI,
            // not an arbitrary path or untrusted shell and not a GUI window.
            let output=std::env::current_exe()
                .map_err(|e|e.to_string())
                .and_then(|exe|Command::new(exe)
                    .arg("--w4-pilot").args(&args)
                    .stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped())
                    .output().map_err(|e|e.to_string()));
            let completed=match output {
                Ok(out)=>Finished{
                    version,action:work,good:out.status.success(),
                    stdout:String::from_utf8_lossy(&out.stdout[..out.stdout.len().min(4096)]).into(),
                    stderr:String::from_utf8_lossy(&out.stderr[..out.stderr.len().min(1024)]).into(),
                },
                Err(e)=>Finished{version,action:work,good:false,stdout:String::new(),stderr:e},
            };
            if tx.send(completed).is_ok(){wake();}
        });
    }
    pub(super) fn poll(&mut self)->bool {
        let mut changed=false;
        while let Ok(done)=self.rx.try_recv(){
            if done.version!=self.generation {continue;}
            self.view.busy=false;
            changed=true;
            match done.action {
                Work::Status=>self.view.notice=Some(if done.good && done.stdout.contains("AKULA_W4_PILOT_READY") {
                    "Проверено: встроенная Акула нашла исправный W4. Подключение не открывалось.".into()
                }else{"Компонент W4 отсутствует или не прошёл проверку контрольной суммы.".into()}),
                Work::InitSender=>{
                    if done.good {
                        self.view.sender_public=key_line(&done.stdout,"SENDER_VERIFY=",64);
                    }
                    self.view.notice=Some(if self.view.sender_public.is_some() {
                        "Новые ключи отправителя созданы; перенесите только открытый ключ.".into()
                    }else{"Не удалось создать ключи. Существующие ключи не перезаписаны.".into()});
                },
                Work::InitRecipient=>{
                    if done.good {
                        self.view.recipient_invite=key_line(&done.stdout,"RECIPIENT_INVITE=",160);
                    }
                    self.view.notice=Some(if self.view.recipient_invite.is_some(){
                        "Приглашение получателя готово. На другом ПК нужна команда listen.".into()
                    }else{"Ключи получателя не созданы. Существующие ключи сохранены.".into()});
                },
                Work::Send=>{
                    if let Some(id)=key_line(&done.stdout,"MESSAGE_ID=",32) {
                        self.view.last_message=Some(id);
                    }
                    let confirmed=done.good && done.stdout.contains("RECIPIENT_CONFIRMED ");
                    self.view.notice=Some(if confirmed {
                        "Адресат записал письмо и вернул настоящую подписанную W4.".into()
                    }else if self.view.last_message.is_some(){
                        "Подтверждение не получено. Письмо осталось в SQLite: можно повторить отправку.".into()
                    }else{"Отправка не началась. Проверьте W4 и SSH-туннель.".into()});
                },
                Work::Resend=>{
                    self.view.notice=Some(if done.good && done.stdout.contains("RECIPIENT_CONFIRMED ") {
                        self.view.last_message=None;
                        "Повтор успешно подтверждён W4; новые копии больше не отправляются.".into()
                    }else{"Повтор не подтверждён. Проверьте соединение, срок письма и доверие.".into()});
                },
            }
            // Never show full stdout/stderr: they may contain uncontrolled
            // terminal escape sequences or operational details.
            let _=&done.stderr;
        }
        changed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn invalid_clipboard_invitation_can_never_form_send_command() {
        assert!(task_args(Work::Send,Some("a".repeat(159)),None).is_none());
        assert!(task_args(Work::Send,Some("a".repeat(160)),None).is_some());
        assert!(task_args(Work::Resend,Some("a".repeat(160)),Some("f".repeat(31))).is_none());
    }
    #[test]
    fn stale_generation_and_disabled_state_do_not_initiate_network(){
        let mut p=PilotUi::default();
        p.action(PilotAction::ConfirmSend,Arc::new(||{}));
        assert!(!p.view.busy);
        p.action(PilotAction::Enable,Arc::new(||{}));
        p.action(PilotAction::Disable,Arc::new(||{}));
        assert!(!p.view.enabled);
    }
}
