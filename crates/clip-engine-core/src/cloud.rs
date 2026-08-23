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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CloudUser {
    pub id: String,
    pub display_name: String,
    pub role: String,
    pub status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Licence {
    pub id: String,
    pub key_hint: String,
    pub label: String,
    pub status: String,
    pub user_id: Option<String>,
    pub display_name: Option<String>,
    #[serde(default)]
    pub device_count: i64,
    #[serde(default)]
    pub active_clip_count: i64,
    #[serde(default)]
    pub active_bytes: i64,
    #[serde(default)]
    pub uploaded_bytes: i64,
    pub created_at: String,
    pub last_activated_at: Option<String>,
    pub revoked_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CreatedLicence {
    #[serde(flatten)]
    pub licence: Licence,
    pub key: String,
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

    pub fn token(&self) -> anyhow::Result<Option<String>> {
        match Self::entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(error) => Err(error)
                .context("Could not read the Clip Engine licence from the credential vault"),
        }
    }

    pub fn authenticated(&self) -> bool {
        self.token().ok().flatten().is_some()
    }

    fn store_token(&self, token: &str) -> anyhow::Result<()> {
        Self::entry()?
            .set_password(token)
            .context("Could not save the Clip Engine licence to the credential vault")
    }

    fn clear_token(&self) -> anyhow::Result<()> {
        match Self::entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(error) => Err(error).context("Could not remove the Clip Engine licence"),
        }
    }

    pub async fn logout(&self) -> anyhow::Result<()> {
        if self.authenticated() {
            let _ = self
                .send::<(), serde_json::Value>(Method::POST, "/v1/auth/logout", None, true)
                .await;
        }
        self.clear_token()
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
                .context("Enter a valid licence key before publishing")?;
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
                let _ = self.clear_token();
                bail!("Your publishing licence is no longer valid. Enter a licence key again.");
            }
            bail!(error);
        }
        response
            .json()
            .await
            .context("The Clip Engine service returned an invalid response")
    }

    pub async fn activate_licence(
        &self,
        licence_key: &str,
        display_name: &str,
        device_name: &str,
    ) -> anyhow::Result<DeviceSession> {
        #[derive(Serialize)]
        #[serde(rename_all = "camelCase")]
        struct Input<'a> {
            licence_key: &'a str,
            display_name: &'a str,
            device_name: &'a str,
        }
        let session = self
            .send(
                Method::POST,
                "/v1/auth/licence",
                Some(&Input {
                    licence_key,
                    display_name,
                    device_name,
                }),
                false,
            )
            .await?;
        self.store_session(session)
    }

    fn store_session(&self, session: DeviceSession) -> anyhow::Result<DeviceSession> {
        self.store_token(&session.token)?;
        Ok(session)
    }

    pub async fn me(&self) -> anyhow::Result<CloudUser> {
        self.send::<(), _>(Method::GET, "/v1/me", None, true).await
    }

    pub async fn licences(&self) -> anyhow::Result<Vec<Licence>> {
        self.send::<(), _>(Method::GET, "/v1/licences", None, true)
            .await
    }

    pub async fn create_licence(&self, label: &str) -> anyhow::Result<CreatedLicence> {
        #[derive(Serialize)]
        struct Input<'a> {
            label: &'a str,
        }
        self.send(Method::POST, "/v1/licences", Some(&Input { label }), true)
            .await
    }

    pub async fn set_licence_status(&self, id: &str, status: &str) -> anyhow::Result<()> {
        #[derive(Serialize)]
        struct Input<'a> {
            status: &'a str,
        }
        let _: serde_json::Value = self
            .send(
                Method::PATCH,
                &format!("/v1/licences/{id}"),
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
