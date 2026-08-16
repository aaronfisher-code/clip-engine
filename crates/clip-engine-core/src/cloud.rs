use anyhow::{bail, Context};
use aws_credential_types::Credentials;
use aws_sdk_s3::config::{Builder as S3ConfigBuilder, Region};
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::{CompletedMultipartUpload, CompletedPart};
use reqwest::{Method, StatusCode};
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use std::path::Path;
use tokio::io::AsyncReadExt;

const KEYRING_SERVICE: &str = "dev.dab.clip-engine";
const KEYRING_USER: &str = "device-token";
const KEYRING_REQUEST_USER: &str = "access-request-token";

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AdminUser {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
    #[serde(default)]
    pub device_count: i64,
    pub last_seen_at: Option<String>,
    #[serde(default)]
    pub active_clip_count: i64,
    #[serde(default)]
    pub active_bytes: i64,
    #[serde(default)]
    pub uploaded_bytes: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PasswordReset {
    pub token: String,
    pub url: String,
    pub username: String,
    pub purpose: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DeviceSession {
    pub token: String,
    pub user: CloudUser,
    pub device_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccessRequest {
    pub id: String,
    pub username: String,
    pub display_name: String,
    pub status: String,
    pub created_at: String,
    pub reviewed_at: Option<String>,
    #[serde(default)]
    pub request_token: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudClip {
    pub id: String,
    pub owner_id: String,
    pub owner_name: String,
    pub slug: String,
    pub title: String,
    pub status: String,
    pub published_at: Option<String>,
    pub expires_at: Option<String>,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub fps: f64,
    pub size: i64,
    pub url: Option<String>,
    pub media_url: Option<String>,
    pub thumbnail_url: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TemporaryCredentials {
    pub access_key_id: String,
    pub secret_access_key: String,
    pub session_token: String,
    pub endpoint: String,
    pub bucket: String,
    pub expires_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedUpload {
    pub upload_id: String,
    pub clip_id: String,
    pub video_key: String,
    pub thumbnail_key: String,
    pub credentials: TemporaryCredentials,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadIntent<'a> {
    pub title: &'a str,
    pub video_size: u64,
    pub thumbnail_size: u64,
    pub duration: f64,
    pub width: i64,
    pub height: i64,
    pub fps: f64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Completion {
    #[allow(dead_code)]
    pub complete: bool,
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Extension {
    pub expires_at: String,
}

#[derive(Debug, Deserialize)]
struct ApiError {
    error: String,
}

#[derive(Clone)]
pub struct CloudClient {
    base_url: String,
    client: reqwest::Client,
}

impl CloudClient {
    pub fn new(base_url: String) -> anyhow::Result<Self> {
        Ok(Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::builder()
                .user_agent(format!("clip-engine/{}", env!("CARGO_PKG_VERSION")))
                .build()?,
        })
    }

    fn entry() -> anyhow::Result<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_USER)
            .context("Could not open the operating-system credential vault")
    }

    fn request_entry() -> anyhow::Result<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, KEYRING_REQUEST_USER)
            .context("Could not open the account-request credential vault")
    }

    pub fn token(&self) -> anyhow::Result<Option<String>> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => {
                Err(error).context("Could not read the Clip Engine login from the credential vault")
            }
        }
    }

    pub fn authenticated(&self) -> bool {
        self.token().ok().flatten().is_some()
    }

    fn request_token(&self) -> anyhow::Result<Option<String>> {
        match Self::request_entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error).context("Could not read the pending account request"),
        }
    }

    pub fn pending_access_request(&self) -> bool {
        self.request_token().ok().flatten().is_some()
    }

    fn store_request_token(&self, token: &str) -> anyhow::Result<()> {
        Self::request_entry()?
            .set_password(token)
            .context("Could not save the pending account request")
    }

    pub fn clear_access_request(&self) -> anyhow::Result<()> {
        match Self::request_entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context("Could not remove the pending account request"),
        }
    }

    fn store_token(&self, token: &str) -> anyhow::Result<()> {
        Self::entry()?
            .set_password(token)
            .context("Could not save the Clip Engine login to the credential vault")
    }

    fn clear_token(&self) -> anyhow::Result<()> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context("Could not remove the Clip Engine login"),
        }
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        if self.authenticated() {
            let _ = self
                .send::<(), serde_json::Value>(Method::POST, "/v1/auth/logout", None, true)
                .await;
        }
        self.clear_token()?;
        self.clear_access_request()
    }

    async fn send<I: Serialize + ?Sized, O: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        input: Option<&I>,
        authenticated: bool,
    ) -> anyhow::Result<O> {
        let mut request = self
            .client
            .request(method, format!("{}{}", self.base_url, path));
        if let Some(input) = input {
            request = request.json(input);
        }
        if authenticated {
            let token = self
                .token()?
                .context("Sign in with an invitation before publishing")?;
            request = request.bearer_auth(token);
        }
        let response = request
            .send()
            .await
            .context("Could not reach the Clip Engine service")?;
        let status = response.status();
        if !status.is_success() {
            let error = response
                .json::<ApiError>()
                .await
                .map(|value| value.error)
                .unwrap_or_else(|_| format!("Cloud request failed ({status})"));
            if status == StatusCode::UNAUTHORIZED {
                bail!("Your Clip Engine login is no longer valid. Sign in again.");
            }
            bail!(error);
        }
        response
            .json()
            .await
            .context("The Clip Engine service returned an invalid response")
    }

    pub async fn redeem(
        &self,
        invite_token: &str,
        username: &str,
        credential_secret: &str,
        display_name: &str,
        device_name: &str,
    ) -> anyhow::Result<DeviceSession> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Input<'a> {
            invite_token: &'a str,
            username: &'a str,
            credential_secret: &'a str,
            display_name: &'a str,
            device_name: &'a str,
        }
        let session = self
            .send(
                Method::POST,
                "/v1/auth/redeem",
                Some(&Input {
                    invite_token,
                    username,
                    credential_secret,
                    display_name,
                    device_name,
                }),
                false,
            )
            .await?;
        self.store_session(session)
    }

    pub async fn login(
        &self,
        username: &str,
        credential_secret: &str,
        owner_token: Option<&str>,
        device_name: &str,
    ) -> anyhow::Result<DeviceSession> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Input<'a> {
            username: &'a str,
            credential_secret: &'a str,
            owner_token: Option<&'a str>,
            device_name: &'a str,
        }
        let session = self
            .send(
                Method::POST,
                "/v1/auth/login",
                Some(&Input {
                    username,
                    credential_secret,
                    owner_token,
                    device_name,
                }),
                false,
            )
            .await?;
        self.store_session(session)
    }

    pub async fn validate_password_reset(
        &self,
        invite_token: &str,
        username: &str,
    ) -> anyhow::Result<()> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Input<'a> {
            invite_token: &'a str,
            username: &'a str,
        }
        let _: serde_json::Value = self
            .send(
                Method::POST,
                "/v1/auth/password-reset/validate",
                Some(&Input {
                    invite_token,
                    username,
                }),
                false,
            )
            .await?;
        Ok(())
    }

    fn store_session(&self, session: DeviceSession) -> anyhow::Result<DeviceSession> {
        self.store_token(&session.token)?;
        let _ = self.clear_access_request();
        Ok(session)
    }

    pub async fn me(&self) -> anyhow::Result<CloudUser> {
        self.send::<(), _>(Method::GET, "/v1/me", None, true).await
    }

    pub async fn request_access(
        &self,
        username: &str,
        display_name: &str,
        credential_secret: &str,
    ) -> anyhow::Result<AccessRequest> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Input<'a> {
            username: &'a str,
            display_name: &'a str,
            credential_secret: &'a str,
        }
        let mut request: AccessRequest = self
            .send(
                Method::POST,
                "/v1/access-requests",
                Some(&Input {
                    username,
                    display_name,
                    credential_secret,
                }),
                false,
            )
            .await?;
        let token = request
            .request_token
            .take()
            .context("The service did not return an account-request token")?;
        self.store_request_token(&token)?;
        Ok(request)
    }

    pub async fn access_request_status(&self) -> anyhow::Result<AccessRequest> {
        let token = self
            .request_token()?
            .context("No pending account request is saved on this device")?;
        let response = self
            .client
            .get(format!("{}/v1/access-requests/me", self.base_url))
            .bearer_auth(token)
            .send()
            .await
            .context("Could not reach the Clip Engine service")?;
        let status = response.status();
        if !status.is_success() {
            let error = response
                .json::<ApiError>()
                .await
                .map(|value| value.error)
                .unwrap_or_else(|_| format!("Account request failed ({status})"));
            bail!(error);
        }
        response
            .json()
            .await
            .context("The Clip Engine service returned an invalid account request")
    }

    pub async fn access_requests(&self) -> anyhow::Result<Vec<AccessRequest>> {
        self.send::<(), _>(Method::GET, "/v1/access-requests", None, true)
            .await
    }

    pub async fn review_access_request(&self, id: &str, decision: &str) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Input<'a> {
            decision: &'a str,
        }
        let _: serde_json::Value = self
            .send(
                Method::PATCH,
                &format!("/v1/access-requests/{id}"),
                Some(&Input { decision }),
                true,
            )
            .await?;
        Ok(())
    }

    pub async fn create_password_reset(&self, id: &str) -> anyhow::Result<PasswordReset> {
        self.send(
            Method::POST,
            &format!("/v1/users/{id}/password-reset"),
            Some(&serde_json::json!({})),
            true,
        )
        .await
    }

    pub async fn users(&self) -> anyhow::Result<Vec<AdminUser>> {
        self.send::<(), _>(Method::GET, "/v1/users", None, true)
            .await
    }

    pub async fn set_user_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Input<'a> {
            status: &'a str,
        }
        let _: serde_json::Value = self
            .send(
                Method::PATCH,
                &format!("/v1/users/{id}"),
                Some(&Input { status }),
                true,
            )
            .await?;
        Ok(())
    }

    pub async fn clips(&self) -> anyhow::Result<Vec<CloudClip>> {
        self.send::<(), _>(Method::GET, "/v1/clips", None, true)
            .await
    }

    pub async fn create_upload(&self, intent: &UploadIntent<'_>) -> anyhow::Result<CreatedUpload> {
        self.send(Method::POST, "/v1/uploads", Some(intent), true)
            .await
    }

    pub async fn complete_upload(&self, upload_id: &str) -> anyhow::Result<Completion> {
        self.send(
            Method::POST,
            &format!("/v1/uploads/{upload_id}/complete"),
            Some(&serde_json::json!({})),
            true,
        )
        .await
    }

    pub async fn delete_clip(&self, clip_id: &str) -> anyhow::Result<()> {
        let _: serde_json::Value = self
            .send::<(), _>(Method::DELETE, &format!("/v1/clips/{clip_id}"), None, true)
            .await?;
        Ok(())
    }

    pub async fn extend_clip(&self, clip_id: &str) -> anyhow::Result<Extension> {
        self.send(
            Method::POST,
            &format!("/v1/clips/{clip_id}/extend"),
            Some(&serde_json::json!({})),
            true,
        )
        .await
    }
}

fn s3_client(credentials: &TemporaryCredentials) -> aws_sdk_s3::Client {
    let credentials_provider = Credentials::new(
        credentials.access_key_id.clone(),
        credentials.secret_access_key.clone(),
        Some(credentials.session_token.clone()),
        None,
        "clip-engine-temporary",
    );
    let config = S3ConfigBuilder::new()
        .behavior_version_latest()
        .region(Region::new("auto"))
        .endpoint_url(&credentials.endpoint)
        .credentials_provider(credentials_provider)
        .force_path_style(true)
        .build();
    aws_sdk_s3::Client::from_conf(config)
}

pub async fn upload_file<F>(
    credentials: &TemporaryCredentials,
    key: &str,
    path: &Path,
    content_type: &str,
    mut progress: F,
) -> anyhow::Result<String>
where
    F: FnMut(f64) + Send,
{
    const PART_SIZE: usize = 16 * 1024 * 1024;
    let client = s3_client(credentials);
    let size = tokio::fs::metadata(path).await?.len();
    if size <= PART_SIZE as u64 {
        let result = client
            .put_object()
            .bucket(&credentials.bucket)
            .key(key)
            .content_type(content_type)
            .body(ByteStream::from_path(path).await?)
            .send()
            .await?;
        progress(1.0);
        return Ok(result.e_tag.unwrap_or_default());
    }
    let created = client
        .create_multipart_upload()
        .bucket(&credentials.bucket)
        .key(key)
        .content_type(content_type)
        .send()
        .await?;
    let upload_id = created
        .upload_id
        .context("R2 did not return a multipart upload ID")?;
    let result: anyhow::Result<String> = async {
        let mut file = tokio::fs::File::open(path).await?;
        let mut completed = Vec::new();
        let mut uploaded = 0_u64;
        let mut part_number = 1;
        loop {
            let mut bytes = Vec::with_capacity(PART_SIZE);
            let read = (&mut file)
                .take(PART_SIZE as u64)
                .read_to_end(&mut bytes)
                .await?;
            if read == 0 {
                break;
            }
            let part = client
                .upload_part()
                .bucket(&credentials.bucket)
                .key(key)
                .upload_id(&upload_id)
                .part_number(part_number)
                .body(ByteStream::from(bytes))
                .send()
                .await?;
            completed.push(
                CompletedPart::builder()
                    .part_number(part_number)
                    .set_e_tag(part.e_tag)
                    .build(),
            );
            uploaded += read as u64;
            progress(uploaded as f64 / size as f64);
            part_number += 1;
        }
        let multipart = CompletedMultipartUpload::builder()
            .set_parts(Some(completed))
            .build();
        let result = client
            .complete_multipart_upload()
            .bucket(&credentials.bucket)
            .key(key)
            .upload_id(&upload_id)
            .multipart_upload(multipart)
            .send()
            .await
            .context("R2 could not finalize the multipart upload")?;
        Ok(result.e_tag.unwrap_or_default())
    }
    .await;
    if result.is_err() {
        let _ = client
            .abort_multipart_upload()
            .bucket(&credentials.bucket)
            .key(key)
            .upload_id(&upload_id)
            .send()
            .await;
    }
    result
}
