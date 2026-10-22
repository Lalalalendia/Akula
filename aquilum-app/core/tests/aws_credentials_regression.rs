//! Offline identity-provider compatibility checks for the modern AWS TLS stack.
//! Real AWS IAM Identity Center login needs a user; no live SSO claim is made.
//! This tests that SSO remains compiled and credential_process works in the default chain.

#![cfg(all(feature = "sync-s3", unix))]

use aws_sdk_s3::config::ProvideCredentials;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

const CHILD: &str = "AKULA_AWS_PROOF_CHILD";
const ACCESS: &str = "AKULA_TEST_PROCESS_ACCESS";
const SECRET: &str = "AKULA_TEST_PROCESS_SECRET";

#[test]
fn aws_sso_provider_remains_compiled_in_opt_in_build() {
    // This type is unavailable when aws-config/sso is disabled.
    let name = std::any::type_name::<aws_config::sso::SsoCredentialsProvider>();
    assert!(name.contains("SsoCredentialsProvider"));
}

#[test]
fn aws_credential_process_is_selected_by_default_chain() {
    if std::env::var_os(CHILD).is_some() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("Tokio runtime");
        let creds = runtime.block_on(async {
            let sdk = aws_config::defaults(aws_config::BehaviorVersion::latest())
                .region(aws_config::Region::new("us-east-1"))
                .load()
                .await;
            let provider = sdk.credentials_provider().expect("credential chain");
            provider
                .provide_credentials()
                .await
                .expect("process credentials")
        });
        assert_eq!(creds.access_key_id(), ACCESS);
        assert_eq!(creds.secret_access_key(), SECRET);
        return;
    }

    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("time")
        .as_nanos();
    let root = std::env::temp_dir().join(format!(
        "akula-aws-provider-proof-{}-{stamp}",
        std::process::id()
    ));
    fs::create_dir(&root).expect("temporary isolated AWS profile");
    let script = root.join("credentials.sh");
    let config = root.join("config");
    let empty = root.join("credentials");
    let json = format!(r#"{{"Version":1,"AccessKeyId":"{ACCESS}","SecretAccessKey":"{SECRET}"}}"#);
    fs::write(&script, format!("#!/bin/sh\nprintf '%s\\n' '{}'\n", json))
        .expect("write synthetic process output");
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))
        .expect("private process credentials script");
    fs::write(
        &config,
        format!(
            "[profile akula-process-proof]\nregion = us-east-1\ncredential_process = {}\n",
            script.display()
        ),
    )
    .expect("write private AWS profile");
    fs::write(&empty, "").expect("empty shared AWS credentials file");

    // Independent child avoids mutating global env while other tests run.
    let output = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "aws_credential_process_is_selected_by_default_chain",
            "--nocapture",
        ])
        .env(CHILD, "1")
        .env("AWS_PROFILE", "akula-process-proof")
        .env("AWS_CONFIG_FILE", &config)
        .env("AWS_SHARED_CREDENTIALS_FILE", &empty)
        .env("AWS_EC2_METADATA_DISABLED", "true")
        .env_remove("AWS_ACCESS_KEY_ID")
        .env_remove("AWS_SECRET_ACCESS_KEY")
        .env_remove("AWS_SESSION_TOKEN")
        .env_remove("AWS_WEB_IDENTITY_TOKEN_FILE")
        .env_remove("AWS_ROLE_ARN")
        .env_remove("AWS_CONTAINER_CREDENTIALS_RELATIVE_URI")
        .env_remove("AWS_CONTAINER_CREDENTIALS_FULL_URI")
        .output()
        .expect("run isolated credential provider chain test");

    // Never print credentials, even synthetic keys, in logs.
    assert!(
        output.status.success(),
        "credential-process child failed (exit status {:?})",
        output.status.code()
    );
    let _ = fs::remove_dir_all(root);
}
