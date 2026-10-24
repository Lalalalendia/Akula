    use super::*;
    use akula_dropbox_auth_lab::begin_readonly;
    use std::sync::Mutex;
    use url::Url;

    fn code() -> TokenExchange {
        let start = begin_readonly("app_key_123",
            "http://127.0.0.1:47219/dropbox/callback").unwrap();
        let state = Url::parse(&start.authorize_url).unwrap().query_pairs()
            .find(|(key, _)| key == "state").unwrap().1.into_owned();
        start.complete(&format!(
            "http://127.0.0.1:47219/dropbox/callback?state={state}&code=once",
        )).unwrap()
    }
    #[derive(Default)]
    struct Fake {
        calls: Mutex<Vec<&'static str>>,
        other_account: bool,
        bad_namespace: bool,
        read_failed: bool,
        store_failed: bool,
    }
    impl Fake {
        fn calls(&self) -> Vec<&'static str> { self.calls.lock().unwrap().clone() }
    }
    impl AccountPorts for Fake {
        fn exchange(&self, code: TokenExchange) -> Result<SessionTokens, AccountLoginError> {
            self.calls.lock().unwrap().push("exchange");
            let form = code.into_form_body();
            assert!(form.contains("code=once") && !form.contains("client_secret"));
            Ok(SessionTokens {
                account_id: "dbid:alice".into(),
                access: Zeroizing::new("synthetic-access".into()),
                refresh: Zeroizing::new("synthetic-refresh".into()),
            })
        }
        fn get_account(&self, access: &str) -> Result<AccountIdentity, AccountLoginError> {
            self.calls.lock().unwrap().push("get_current_account");
            assert_eq!(access, "synthetic-access");
            if self.read_failed { return Err(AccountLoginError::NetworkUnavailable); }
            Ok(AccountIdentity {
                account_id: if self.other_account { "dbid:bob" } else { "dbid:alice" }.into(),
                root_namespace_id: if self.bad_namespace { "../bad" } else { "12345" }.into(),
            })
        }
        fn save(&self, _: &AccountCredentialScope, refresh: &str)
            -> Result<(), AccountLoginError>
        {
            self.calls.lock().unwrap().push("save_os");
            assert_eq!(refresh, "synthetic-refresh");
            if self.store_failed {
                return Err(AccountLoginError::SecretStore(SecretError::StoreUnavailable));
            }
            Ok(())
        }
    }

    fn fake_account_response(
        status: u16, response_body: String,
    ) -> (String, std::thread::JoinHandle<()>) {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut received = Vec::new();
            let mut chunk = [0u8; 4096];
            while !received.windows(4).any(|b| b == b"\r\n\r\n") {
                let n = stream.read(&mut chunk).unwrap();
                if n == 0 || received.len() > 8192 { panic!("invalid fake RPC request"); }
                received.extend_from_slice(&chunk[..n]);
            }
            let header = String::from_utf8_lossy(&received).to_ascii_lowercase();
            assert!(header.starts_with("post /2/users/get_current_account http/1.1"));
            assert!(header.contains("authorization: bearer synthetic-access"));
            let reason = if status == 200 { "OK" } else { "Unauthorized" };
            write!(stream,
                "HTTP/1.1 {status} {reason}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                response_body.len(), response_body,
            ).unwrap();
        });
        (format!("http://127.0.0.1:{port}/2/users/get_current_account"), worker)
    }

    #[test]
    fn wire_account_lookup_rejects_malformed_oversize_and_unauthorized() {
        let valid = serde_json::json!({
            "account_id": "dbid:alice",
            "root_info": {".tag":"user","root_namespace_id":"12345","home_namespace_id":"12345"}
        }).to_string();
        let (url, task) = fake_account_response(200, valid);
        let result = lookup_account_at(&url, "synthetic-access").unwrap();
        task.join().unwrap();
        assert_eq!(result.account_id, "dbid:alice");
        assert_eq!(result.root_namespace_id, "12345");
        assert!(result.validate("app_key_123").is_ok());

        for (status, body, expected) in [
            (200, r#"{"account_id":"dbid:alice"}"#.to_owned(),
                AccountLoginError::InvalidAccount),
            (200, "x".repeat(MAX_ACCOUNT_REPLY as usize + 1),
                AccountLoginError::InvalidAccount),
            (401, r#"{"error":"expired"}"#.to_owned(),
                AccountLoginError::NetworkUnavailable),
        ] {
            let (url, task) = fake_account_response(status, body);
            assert_eq!(lookup_account_at(&url, "synthetic-access"), Err(expected));
            task.join().unwrap();
        }
    }

    #[test]
    fn diagnostic_codes_distinguish_console_callback_scope_and_keyring() {
        let cases = [
            (AccountLoginError::Callback(CallbackError::PortUnavailable),
                "DROPBOX_CALLBACK_PORT_UNAVAILABLE"),
            (AccountLoginError::BrowserUnavailable, "DROPBOX_BROWSER_UNAVAILABLE"),
            (AccountLoginError::Callback(CallbackError::Authorization(AuthError::Denied)),
                "DROPBOX_CONSENT_DENIED"),
            (AccountLoginError::Exchange(ExchangeError::UnexpectedScope),
                "DROPBOX_READ_SCOPES_REJECTED"),
            (AccountLoginError::Exchange(ExchangeError::Rejected),
                "DROPBOX_OAUTH_APP_OR_REDIRECT_REJECTED"),
            (AccountLoginError::Refresh(ResumeError::RefreshRejected),
                "DROPBOX_SAVED_CONSENT_REVOKED"),
            (AccountLoginError::SecretStore(SecretError::StoreUnavailable),
                "DROPBOX_OS_KEYRING_UNAVAILABLE"),
            (AccountLoginError::AccountMismatch,
                "DROPBOX_ACCOUNT_OR_ROOT_MISMATCH"),
            (AccountLoginError::MissingSavedConsent, "DROPBOX_NO_SAVED_CONSENT"),
        ];
        for (err, expected) in cases {
            let status = err.public_status();
            assert!(status.starts_with(expected), "{status}");
            assert!(status.ends_with(" / SYNC_DISABLED"));
            assert!(!status.contains("dbid:") && !status.contains("Bearer "));
        }
        assert_ne!(
            AccountLoginError::Exchange(ExchangeError::UnexpectedScope).public_status(),
            AccountLoginError::Callback(CallbackError::PortUnavailable).public_status());
    }

    #[test]
    fn account_consent_uses_no_vault_and_only_stores_verified_identity() {
        let fake = Fake::default();
        let account = connect_with(&fake, code(), "app_key_123", None).unwrap();
        assert_eq!(account.account_id, "dbid:alice");
        assert_eq!(account.root_namespace_id, "12345");
        assert_eq!(fake.calls(), vec!["exchange", "get_current_account", "save_os"]);
    }

    #[test]
    fn mismatched_account_and_invalid_root_never_store_tokens() {
        for fake in [
            Fake { other_account: true, ..Default::default() },
            Fake { bad_namespace: true, ..Default::default() },
            Fake { read_failed: true, ..Default::default() },
        ] {
            assert!(connect_with(&fake, code(), "app_key_123", None).is_err());
            assert_eq!(fake.calls(), vec!["exchange", "get_current_account"]);
        }
    }

    #[test]
    fn unavailable_keyring_fails_closed_and_never_reports_logged_in() {
        let fake = Fake { store_failed: true, ..Default::default() };
        assert_eq!(connect_with(&fake, code(), "app_key_123", None),
            Err(AccountLoginError::SecretStore(SecretError::StoreUnavailable)));
        assert_eq!(fake.calls(), vec!["exchange", "get_current_account", "save_os"]);
    }

    #[derive(Default)]
    struct FakeRecovery {
        calls: Mutex<Vec<&'static str>>,
        missing: bool,
        locked: bool,
        refresh_denied: bool,
        network_down: bool,
        other_account: bool,
        other_root: bool,
    }
    impl FakeRecovery {
        fn calls(&self) -> Vec<&'static str> {
            self.calls.lock().unwrap().clone()
        }
    }
    impl AccountResumePorts for FakeRecovery {
        fn load(&self, _: &AccountCredentialScope)
            -> Result<Option<RefreshToken>, AccountLoginError>
        {
            self.calls.lock().unwrap().push("load_keyring");
            if self.locked {
                return Err(AccountLoginError::SecretStore(SecretError::StoreUnavailable));
            }
            if self.missing { return Ok(None); }
            Ok(Some(RefreshToken::new("synthetic-saved-refresh".into()).unwrap()))
        }
        fn refresh(&self, app: &str, saved: &RefreshToken)
            -> Result<Zeroizing<String>, AccountLoginError>
        {
            self.calls.lock().unwrap().push("refresh_once");
            assert_eq!(app, "app_key_123");
            assert_eq!(saved.as_str(), "synthetic-saved-refresh");
            if self.refresh_denied {
                return Err(AccountLoginError::Refresh(ResumeError::RefreshRejected));
            }
            Ok(Zeroizing::new("synthetic-new-access".into()))
        }
        fn get_account(&self, access: &str)
            -> Result<AccountIdentity, AccountLoginError>
        {
            self.calls.lock().unwrap().push("get_current_account");
            assert_eq!(access, "synthetic-new-access");
            if self.network_down { return Err(AccountLoginError::NetworkUnavailable); }
            Ok(AccountIdentity {
                account_id: if self.other_account { "dbid:bob" }
                    else { "dbid:alice" }.into(),
                root_namespace_id: if self.other_root { "98765" }
                    else { "12345" }.into(),
            })
        }
    }
    fn expected_account() -> AccountIdentity {
        AccountIdentity {
            account_id: "dbid:alice".into(),
            root_namespace_id: "12345".into(),
        }
    }

    #[test]
    fn restart_recovers_exact_account_without_browser_or_vault() {
        let ports = FakeRecovery::default();
        assert_eq!(resume_account_with(&ports, "app_key_123", &expected_account()),
            Ok(expected_account()));
        assert_eq!(ports.calls(),
            vec!["load_keyring", "refresh_once", "get_current_account"]);
    }

    #[test]
    fn restart_rejects_missing_locked_revoked_or_offline_consent() {
        for ports in [
            FakeRecovery { missing: true, ..Default::default() },
            FakeRecovery { locked: true, ..Default::default() },
            FakeRecovery { refresh_denied: true, ..Default::default() },
            FakeRecovery { network_down: true, ..Default::default() },
        ] {
            assert!(resume_account_with(&ports, "app_key_123", &expected_account()).is_err());
            assert!(!ports.calls().is_empty());
        }
    }

    #[test]
    fn restart_cannot_claim_foreign_account_or_changed_namespace() {
        for ports in [
            FakeRecovery { other_account: true, ..Default::default() },
            FakeRecovery { other_root: true, ..Default::default() },
        ] {
            assert_eq!(
                resume_account_with(&ports, "app_key_123", &expected_account()),
                Err(AccountLoginError::AccountMismatch));
            assert_eq!(ports.calls(),
                vec!["load_keyring", "refresh_once", "get_current_account"]);
        }
    }

    #[test]
    fn forged_saved_app_or_account_fails_before_keyring_or_network() {
        let ports = FakeRecovery::default();
        assert_eq!(resume_account_with(&ports, "../wrong", &expected_account()),
            Err(AccountLoginError::InvalidAccount));
        assert_eq!(resume_account_with(&ports, "app_key_123", &AccountIdentity {
            account_id: "other".into(), root_namespace_id: "12345".into(),
        }), Err(AccountLoginError::InvalidAccount));
        assert!(ports.calls().is_empty());
    }

    #[test]
    fn malformed_app_id_rejected_before_oauth_exchange() {
        let fake = Fake::default();
        assert!(matches!(connect_with(&fake, code(), "../bad_app", None),
            Err(AccountLoginError::InvalidAccount)));
        assert!(fake.calls().is_empty());
    }

    #[test]
    fn renewed_consent_cannot_orphan_old_account_keyring_scope() {
        // The previous verified OS scope remains discoverable only when
        // switching accounts requires an explicit Forget operation first.
        let previous = expected_account();
        let fake = Fake::default();
        assert_eq!(
            connect_with(&fake, code(), "app_key_123", Some(&previous)),
            Ok(previous.clone()),
        );
        assert_eq!(fake.calls(), vec!["exchange", "get_current_account", "save_os"]);

        for expected in [
            AccountIdentity { account_id: "dbid:bob".into(),
                root_namespace_id: "12345".into() },
            AccountIdentity { account_id: "dbid:alice".into(),
                root_namespace_id: "98765".into() },
        ] {
            let fake = Fake::default();
            assert_eq!(connect_with(&fake, code(), "app_key_123", Some(&expected)),
                Err(AccountLoginError::AccountMismatch));
            assert_eq!(fake.calls(), vec!["exchange", "get_current_account"],
                "a different account/root must never persist a new refresh token");
        }
    }
