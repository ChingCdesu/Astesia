use super::{CredentialVaultError, CredentialVaultErrorCode};

const CREDENTIAL_SERVICE: &str = "com.astesia.app.database";
const MASTER_CREDENTIAL_SERVICE: &str = "com.astesia.app.credential-vault";
const MASTER_CREDENTIAL_ACCOUNT: &str = "master-key-v1";
#[cfg(target_os = "macos")]
const PROTECTED_MASTER_CREDENTIAL_ACCOUNT: &str = "master-key-v2-user-presence";

pub(super) trait KeyringBackend: Send + Sync {
    fn get_master(&self) -> keyring::Result<Vec<u8>>;
    fn set_master(&self, secret: &[u8]) -> keyring::Result<()>;
    fn get_legacy(&self, reference: &str) -> keyring::Result<String>;
    fn delete_legacy(&self, reference: &str) -> keyring::Result<()>;
}

#[derive(Debug, Default)]
pub(super) struct PlatformKeyringBackend;

impl KeyringBackend for PlatformKeyringBackend {
    fn get_master(&self) -> keyring::Result<Vec<u8>> {
        #[cfg(target_os = "macos")]
        {
            match get_macos_protected_master() {
                Ok(secret) => Ok(secret),
                Err(keyring::Error::NoEntry) => {
                    // The classic shared item is a migration bridge because the App and
                    // independently signed sidecar have separate data-protection scopes.
                    let secret = master_platform_entry()?.get_secret()?;
                    if let Err(error) = set_macos_protected_master(&secret) {
                        if !macos_protected_keychain_missing_entitlement(&error) {
                            return Err(error);
                        }
                        log_macos_classic_keychain_fallback();
                    }
                    Ok(secret)
                }
                Err(error) if macos_protected_keychain_missing_entitlement(&error) => {
                    log_macos_classic_keychain_fallback();
                    master_platform_entry()?.get_secret()
                }
                Err(error) => Err(error),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            master_platform_entry()?.get_secret()
        }
    }

    fn set_master(&self, secret: &[u8]) -> keyring::Result<()> {
        #[cfg(target_os = "macos")]
        {
            // The classic item is written first so separately signed processes can import
            // the same master before their protected Keychain items exist.
            master_platform_entry()?.set_secret(secret)?;
            match set_macos_protected_master(secret) {
                Ok(()) => Ok(()),
                Err(error) if macos_protected_keychain_missing_entitlement(&error) => {
                    log_macos_classic_keychain_fallback();
                    Ok(())
                }
                Err(error) => Err(error),
            }
        }
        #[cfg(not(target_os = "macos"))]
        {
            master_platform_entry()?.set_secret(secret)
        }
    }

    fn get_legacy(&self, reference: &str) -> keyring::Result<String> {
        platform_entry(reference)?.get_password()
    }

    fn delete_legacy(&self, reference: &str) -> keyring::Result<()> {
        platform_entry(reference)?.delete_credential()
    }
}

pub(super) fn map_error(error: keyring::Error, operation: &str) -> CredentialVaultError {
    let code = match error {
        keyring::Error::NoEntry => CredentialVaultErrorCode::Missing,
        keyring::Error::NoStorageAccess(_) if no_storage_access_means_unavailable() => {
            CredentialVaultErrorCode::StoreUnavailable
        }
        keyring::Error::NoStorageAccess(_) => CredentialVaultErrorCode::AccessDenied,
        keyring::Error::PlatformFailure(platform_error)
            if platform_failure_is_access_denied(platform_error.as_ref()) =>
        {
            CredentialVaultErrorCode::AccessDenied
        }
        keyring::Error::PlatformFailure(_) => CredentialVaultErrorCode::StoreUnavailable,
        keyring::Error::BadEncoding(_) | keyring::Error::Ambiguous(_) => {
            CredentialVaultErrorCode::Corrupt
        }
        keyring::Error::TooLong(_, _) | keyring::Error::Invalid(_, _) => {
            CredentialVaultErrorCode::Invalid
        }
        _ => CredentialVaultErrorCode::StoreUnavailable,
    };
    CredentialVaultError::new(code, operation)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn platform_entry(reference: &str) -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(CREDENTIAL_SERVICE, reference)
}

#[cfg(any(target_os = "macos", target_os = "windows", target_os = "linux"))]
fn master_platform_entry() -> keyring::Result<keyring::Entry> {
    keyring::Entry::new(MASTER_CREDENTIAL_SERVICE, MASTER_CREDENTIAL_ACCOUNT)
}

#[cfg(target_os = "macos")]
fn get_macos_protected_master() -> keyring::Result<Vec<u8>> {
    use security_framework::passwords::{generic_password, PasswordOptions};

    let mut options = PasswordOptions::new_generic_password(
        MASTER_CREDENTIAL_SERVICE,
        PROTECTED_MASTER_CREDENTIAL_ACCOUNT,
    );
    options.use_protected_keychain();
    generic_password(options).map_err(map_macos_keychain_error)
}

#[cfg(target_os = "macos")]
fn set_macos_protected_master(secret: &[u8]) -> keyring::Result<()> {
    use security_framework::passwords::{set_generic_password_options, PasswordOptions};

    let mut options = PasswordOptions::new_generic_password(
        MASTER_CREDENTIAL_SERVICE,
        PROTECTED_MASTER_CREDENTIAL_ACCOUNT,
    );
    options.use_protected_keychain();
    options.set_access_control_options(macos_master_access_control());
    set_generic_password_options(secret, options).map_err(map_macos_keychain_error)
}

#[cfg(target_os = "macos")]
fn macos_master_access_control() -> security_framework::passwords::AccessControlOptions {
    use security_framework::passwords::AccessControlOptions;

    // `WATCH` remains the backwards-compatible flag for companion authentication.
    AccessControlOptions::BIOMETRY_ANY
        | AccessControlOptions::WATCH
        | AccessControlOptions::DEVICE_PASSCODE
        | AccessControlOptions::OR
}

#[cfg(target_os = "macos")]
fn map_macos_keychain_error(error: security_framework::base::Error) -> keyring::Error {
    match error.code() {
        -25_300 => keyring::Error::NoEntry,
        -25_291 | -34_018 => keyring::Error::NoStorageAccess(Box::new(error)),
        _ => keyring::Error::PlatformFailure(Box::new(error)),
    }
}

#[cfg(target_os = "macos")]
fn macos_protected_keychain_missing_entitlement(error: &keyring::Error) -> bool {
    match error {
        keyring::Error::NoStorageAccess(platform_error)
        | keyring::Error::PlatformFailure(platform_error) => platform_error
            .downcast_ref::<security_framework::base::Error>()
            .is_some_and(|error| error.code() == -34_018),
        _ => false,
    }
}

#[cfg(target_os = "macos")]
fn log_macos_classic_keychain_fallback() {
    log::warn!(
        "macOS data-protection Keychain is unavailable because the process lacks a \
         Keychain access entitlement; using the classic Keychain master key"
    );
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn platform_entry(_reference: &str) -> keyring::Result<keyring::Entry> {
    unsupported_platform_error()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn master_platform_entry() -> keyring::Result<keyring::Entry> {
    unsupported_platform_error()
}

#[cfg(not(any(target_os = "macos", target_os = "windows", target_os = "linux")))]
fn unsupported_platform_error<T>() -> keyring::Result<T> {
    Err(keyring::Error::NoStorageAccess(Box::new(
        std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "Astesia has no credential-store backend for this operating system",
        ),
    )))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
const fn no_storage_access_means_unavailable() -> bool {
    true
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
const fn no_storage_access_means_unavailable() -> bool {
    false
}

#[cfg(target_os = "macos")]
fn platform_failure_is_access_denied(
    error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    error
        .downcast_ref::<security_framework::base::Error>()
        .is_some_and(|error| matches!(error.code(), -128 | -25_243 | -25_293 | -25_308))
}

#[cfg(not(target_os = "macos"))]
fn platform_failure_is_access_denied(
    _error: &(dyn std::error::Error + Send + Sync + 'static),
) -> bool {
    false
}

pub(super) fn remediation() -> String {
    #[cfg(target_os = "linux")]
    {
        return "请安装并启动兼容 Secret Service 的服务（例如 GNOME Keyring、KWallet 或 KeePassXC Secret Service），确认会话 D-Bus 可用后重试。Astesia 不会回退为明文密码文件。"
            .to_string();
    }
    #[cfg(target_os = "windows")]
    {
        return "请确认当前 Windows 用户的 Credential Manager 可用；不受支持的旧版 Windows 需要升级系统。Astesia 不会回退为明文密码文件。"
            .to_string();
    }
    #[cfg(target_os = "macos")]
    {
        return "请解锁当前用户的 macOS Keychain，并通过系统提供的 Touch ID、Apple Watch 或本机密码验证。Astesia App 与 astesia-mcp 会在各自首次实际读取凭据时请求验证；无图形登录会话无法显示系统授权界面。Astesia 不会回退为明文密码文件。"
            .to_string();
    }
    #[allow(unreachable_code)]
    "此操作系统没有 Astesia 支持的系统凭据库；请在受支持的系统上使用。Astesia 不会回退为明文密码文件。"
        .to_string()
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use security_framework::{access_control::SecAccessControl, passwords::AccessControlOptions};

    #[test]
    fn macos_authorization_failures_are_reported_as_access_denied() {
        for status in [-128, -25_243, -25_293, -25_308] {
            let error = security_framework::base::Error::from_code(status);
            assert!(platform_failure_is_access_denied(&error), "{status}");
        }

        let unavailable = security_framework::base::Error::from_code(-25_291);
        assert!(!platform_failure_is_access_denied(&unavailable));
    }

    #[test]
    fn macos_master_accepts_biometry_watch_or_password() {
        let options = macos_master_access_control();
        assert!(options.contains(AccessControlOptions::BIOMETRY_ANY));
        assert!(options.contains(AccessControlOptions::WATCH));
        assert!(options.contains(AccessControlOptions::DEVICE_PASSCODE));
        assert!(options.contains(AccessControlOptions::OR));
        SecAccessControl::create_with_flags(options.bits())
            .expect("macOS must accept the Astesia master-key access control");
    }

    #[test]
    fn macos_missing_entitlement_uses_classic_keychain_fallback() {
        let missing_entitlement =
            map_macos_keychain_error(security_framework::base::Error::from_code(-34_018));
        assert!(macos_protected_keychain_missing_entitlement(
            &missing_entitlement
        ));

        let unavailable =
            map_macos_keychain_error(security_framework::base::Error::from_code(-25_291));
        assert!(!macos_protected_keychain_missing_entitlement(&unavailable));
    }
}
