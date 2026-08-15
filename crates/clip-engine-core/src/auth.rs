use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use pbkdf2::pbkdf2_hmac_array;
use sha2::Sha256;

const ITERATIONS: u32 = 600_000;

pub fn credential_secret(username: &str, password: &str) -> String {
    let normalized = username.trim().to_lowercase();
    let salt = format!("clip-engine-password-v1:{normalized}");
    let hash = pbkdf2_hmac_array::<Sha256, 32>(password.as_bytes(), salt.as_bytes(), ITERATIONS);
    URL_SAFE_NO_PAD.encode(hash)
}

pub fn invite_token(invite: &str) -> String {
    invite
        .trim()
        .split('/')
        .rfind(|part| !part.is_empty())
        .unwrap_or(invite.trim())
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn derived_secret_is_url_safe_without_padding() {
        let secret = credential_secret("Ada", "super-secret-password");
        assert_eq!(secret.len(), 43);
        assert!(secret
            .chars()
            .all(|character| character.is_ascii_alphanumeric()
                || character == '-'
                || character == '_'));
        assert_eq!(secret, credential_secret("ada", "super-secret-password"));
    }

    #[test]
    fn invite_tokens_accept_full_urls() {
        assert_eq!(
            invite_token("https://clips.dab.dev/invite/abc123"),
            "abc123"
        );
    }
}
