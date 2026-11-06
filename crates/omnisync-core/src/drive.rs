//! Google Drive v3 protocol. No bearer tokens or resumable URLs cross the UI boundary.
use crate::{model::Version, oauth::Auth};
use anyhow::{bail, Context, Result};
use reqwest::{Method, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};
use tokio::io::AsyncWriteExt;
pub const CHUNK: usize = 8 * 1024 * 1024;
const API: &str = "https://www.googleapis.com/drive/v3";
const UPLOAD: &str = "https://www.googleapis.com/upload/drive/v3";
const FIELDS: &str =
    "id,name,size,md5Checksum,sha256Checksum,trashed,mimeType,parents,appProperties,version";
#[derive(Clone)]
pub struct Drive {
    http: reqwest::Client,
    auth: Auth,
}
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Metadata {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub size: Option<String>,
    #[serde(default)]
    pub md5_checksum: Option<String>,
    #[serde(default)]
    pub sha256_checksum: Option<String>,
    #[serde(default)]
    pub trashed: bool,
    #[serde(default)]
    pub mime_type: String,
    #[serde(default)]
    pub parents: Vec<String>,
    #[serde(default)]
    pub app_properties: std::collections::HashMap<String, String>,
    #[serde(default)]
    pub version: String,
}
impl Metadata {
    pub fn matches(&self, sha: &str, md5: &str, size: u64) -> bool {
        !self.trashed
            && self.size.as_deref().and_then(|s| s.parse::<u64>().ok()) == Some(size)
            && self
                .sha256_checksum
                .as_deref()
                .map(|s| s == sha)
                .unwrap_or_else(|| self.md5_checksum.as_deref() == Some(md5))
    }
    pub fn matches_version(&self, v: &Version) -> bool {
        self.id == v.remote_id && self.matches(&v.sha256, &v.md5, v.size)
    }
}
pub enum UploadReply {
    Progress(u64),
    Complete(Metadata),
    Expired,
}
impl Drive {
    pub fn new(auth: Auth) -> Result<Self> {
        Ok(Self {
            auth,
            http: reqwest::Client::builder()
                .connect_timeout(Duration::from_secs(10))
                .timeout(Duration::from_secs(90))
                .redirect(reqwest::redirect::Policy::none())
                .build()?,
        })
    }
    async fn request(&self, method: Method, url: &str, body: Option<Value>) -> Result<Response> {
        for attempt in 0..2 {
            let token = self.auth.access_token(attempt == 1).await?;
            let mut req = self.http.request(method.clone(), url).bearer_auth(token);
            if let Some(b) = &body {
                req = req.json(b);
            }
            let res = req
                .send()
                .await
                .map_err(|_| anyhow::anyhow!("Cannot reach Google Drive. Backup will retry."))?;
            if res.status() != StatusCode::UNAUTHORIZED || attempt == 1 {
                return Ok(res);
            }
        }
        unreachable!()
    }
    pub async fn about(&self) -> Result<Value> {
        checked(
            self.request(
                Method::GET,
                &format!(
                    "{API}/about?fields=user(displayName,emailAddress,permissionId),storageQuota"
                ),
                None,
            )
            .await?,
        )
        .await?
        .json()
        .await
        .context("Invalid Drive account response")
    }
    pub async fn generate_id(&self) -> Result<String> {
        let v: Value = checked(
            self.request(
                Method::GET,
                &format!("{API}/files/generateIds?count=1&space=drive&type=files"),
                None,
            )
            .await?,
        )
        .await?
        .json()
        .await?;
        Ok(v["ids"][0]
            .as_str()
            .context("Drive did not allocate an ID")?
            .into())
    }
    pub async fn metadata(&self, id: &str) -> Result<Option<Metadata>> {
        valid_id(id)?;
        let res = self
            .request(
                Method::GET,
                &format!("{API}/files/{id}?fields={FIELDS}"),
                None,
            )
            .await?;
        if res.status() == StatusCode::NOT_FOUND {
            return Ok(None);
        }
        Ok(Some(checked(res).await?.json().await?))
    }
    pub async fn folder(
        &self,
        id: &str,
        name: &str,
        parent: Option<&str>,
        device: &str,
        key: &str,
    ) -> Result<()> {
        if let Some(existing) = self.metadata(id).await? {
            anyhow::ensure!(
                !existing.trashed
                    && existing.mime_type == "application/vnd.google-apps.folder"
                    && existing.app_properties.get("omniKey").map(String::as_str) == Some(key)
                    && existing
                        .app_properties
                        .get("omniDevice")
                        .map(String::as_str)
                        == Some(device),
                "Managed Drive folder changed or belongs to another device. Backup stopped."
            );
            return Ok(());
        }
        let mut body = json!({"id":id,"name":name,"mimeType":"application/vnd.google-apps.folder","appProperties":{"omniDevice":device,"omniKey":key}});
        if let Some(parent) = parent {
            body["parents"] = json!([parent]);
        }
        let res = self
            .request(Method::POST, &format!("{API}/files?fields=id"), Some(body))
            .await?;
        if res.status() != StatusCode::CONFLICT {
            checked(res).await?;
        }
        Ok(())
    }
    pub async fn copy_version(
        &self,
        source: &str,
        target: &str,
        parent: &str,
        name: &str,
        device: &str,
        item: &str,
    ) -> Result<Metadata> {
        if let Some(m) = self.metadata(target).await? {
            return Ok(m);
        }
        valid_id(source)?;
        let res=self.request(Method::POST,&format!("{API}/files/{source}/copy?fields={FIELDS}"),Some(json!({"id":target,"name":name,"parents":[parent],"appProperties":{"omniDevice":device,"omniItem":item,"omniRole":"history"}}))).await?;
        if res.status() == StatusCode::CONFLICT {
            return self
                .metadata(target)
                .await?
                .context("History copy could not be reconciled");
        }
        Ok(checked(res).await?.json().await?)
    }
    pub async fn start_upload(
        &self,
        target: &str,
        existing: bool,
        name: &str,
        parent: &str,
        size: u64,
        device: &str,
        item: &str,
    ) -> Result<String> {
        valid_id(target)?;
        let url = if existing {
            format!("{UPLOAD}/files/{target}?uploadType=resumable&fields={FIELDS}")
        } else {
            format!("{UPLOAD}/files?uploadType=resumable&fields={FIELDS}")
        };
        let mut body = json!({"name":name,"appProperties":{"omniDevice":device,"omniItem":item,"omniRole":"current"}});
        if !existing {
            body["id"] = json!(target);
            body["parents"] = json!([parent]);
        }
        let token = self.auth.access_token(false).await?;
        let res = self
            .http
            .request(
                if existing {
                    Method::PATCH
                } else {
                    Method::POST
                },
                url,
            )
            .bearer_auth(token)
            .header("X-Upload-Content-Type", "application/octet-stream")
            .header("X-Upload-Content-Length", size)
            .json(&body)
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Upload session could not be started"))?;
        let res = checked(res).await?;
        let location = res
            .headers()
            .get("Location")
            .context("Drive did not return an upload session")?
            .to_str()?
            .to_owned();
        valid_session(&location)?;
        Ok(location)
    }
    pub async fn upload(
        &self,
        session: &str,
        offset: u64,
        total: u64,
        bytes: Option<Vec<u8>>,
    ) -> Result<UploadReply> {
        valid_session(session)?;
        let token = self.auth.access_token(false).await?;
        let range = match &bytes {
            Some(b) if !b.is_empty() => {
                format!("bytes {offset}-{}/{total}", offset + b.len() as u64 - 1)
            }
            _ => format!("bytes */{total}"),
        };
        let res = self
            .http
            .put(session)
            .bearer_auth(token)
            .header("Content-Range", range)
            .body(bytes.unwrap_or_default())
            .send()
            .await
            .map_err(|_| {
                anyhow::anyhow!(
                    "Upload interrupted. Its saved session will be checked before retry."
                )
            })?;
        match res.status().as_u16() {
            308 => Ok(UploadReply::Progress(acknowledged(
                res.headers().get("Range").and_then(|v| v.to_str().ok()),
                total,
            )?)),
            404 | 410 => Ok(UploadReply::Expired),
            200 | 201 => Ok(UploadReply::Complete(res.json().await?)),
            _ => {
                checked(res).await?;
                bail!("Unexpected upload response")
            }
        }
    }
    pub async fn download(&self, id: &str, path: &Path, maximum: u64) -> Result<()> {
        valid_id(id)?;
        // A large restore has an idle timeout rather than a total 90-second limit.
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let token = self.auth.access_token(false).await?;
        let res = checked(
            tokio::time::timeout(
                Duration::from_secs(90),
                client
                    .get(format!("{API}/files/{id}?alt=media"))
                    .bearer_auth(token)
                    .send(),
            )
            .await
            .context("Drive download did not respond")?
            .map_err(|_| anyhow::anyhow!("Drive download could not start"))?,
        )
        .await?;
        use futures_util::StreamExt;
        let mut stream = res.bytes_stream();
        let mut file = tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .await?;
        let mut received = 0u64;
        while let Some(bytes) = tokio::time::timeout(Duration::from_secs(90), stream.next())
            .await
            .context("Drive download stalled")?
        {
            let bytes = bytes.map_err(|_| anyhow::anyhow!("Download interrupted"))?;
            received = received
                .checked_add(bytes.len() as u64)
                .context("Download size overflow")?;
            anyhow::ensure!(
                received <= maximum,
                "Cloud object grew beyond the reviewed restore size"
            );
            file.write_all(&bytes).await?;
        }
        file.sync_all().await?;
        Ok(())
    }
    pub async fn list_catalogs(&self) -> Result<Vec<Metadata>> {
        let mut result = Vec::new();
        let mut token = String::new();
        loop {
            let mut url = url::Url::parse(&format!("{API}/files"))?;
            url.query_pairs_mut()
                .append_pair(
                    "q",
                    "trashed = false and appProperties has { key='omniRole' and value='catalog' }",
                )
                .append_pair("fields", &format!("nextPageToken,files({FIELDS})"))
                .append_pair("pageSize", "100");
            if !token.is_empty() {
                url.query_pairs_mut().append_pair("pageToken", &token);
            }
            let v: Value = checked(self.request(Method::GET, url.as_str(), None).await?)
                .await?
                .json()
                .await?;
            result.extend(serde_json::from_value::<Vec<Metadata>>(v["files"].clone())?);
            token = v["nextPageToken"].as_str().unwrap_or_default().into();
            if token.is_empty() {
                break;
            }
            anyhow::ensure!(result.len() < 10000, "Too many catalogs to list");
        }
        Ok(result)
    }
    pub async fn mark_catalog(&self, id: &str) -> Result<()> {
        checked(
            self.request(
                Method::PATCH,
                &format!("{API}/files/{id}"),
                Some(json!({"appProperties":{"omniRole":"catalog"}})),
            )
            .await?,
        )
        .await?;
        Ok(())
    }
    pub async fn empty_file(
        &self,
        id: &str,
        name: &str,
        parent: &str,
        device: &str,
        item: &str,
    ) -> Result<Metadata> {
        let res=self.request(Method::POST,&format!("{API}/files?fields={FIELDS}"),Some(json!({"id":id,"name":name,"parents":[parent],"mimeType":"application/octet-stream","appProperties":{"omniDevice":device,"omniItem":item,"omniRole":"current"}}))).await?;
        if res.status() == StatusCode::CONFLICT {
            return self
                .metadata(id)
                .await?
                .context("Empty-file creation could not be reconciled");
        }
        Ok(checked(res).await?.json().await?)
    }
}
pub fn acknowledged(range: Option<&str>, total: u64) -> Result<u64> {
    match range {
        None => Ok(0),
        Some(s) => {
            let end = s
                .strip_prefix("bytes=0-")
                .context("Invalid upload acknowledgement")?
                .parse::<u64>()?
                .checked_add(1)
                .context("Upload acknowledgement overflow")?;
            anyhow::ensure!(
                end <= total,
                "Drive acknowledged more bytes than the file contains"
            );
            Ok(end)
        }
    }
}
fn valid_id(id: &str) -> Result<()> {
    anyhow::ensure!(
        !id.is_empty()
            && id.len() < 256
            && id
                .bytes()
                .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_'),
        "Invalid Drive file ID"
    );
    Ok(())
}
fn valid_session(value: &str) -> Result<()> {
    let url = url::Url::parse(value)?;
    anyhow::ensure!(
        url.scheme() == "https"
            && url.host_str() == Some("www.googleapis.com")
            && url.port_or_known_default() == Some(443)
            && url.username().is_empty()
            && url.password().is_none()
            && url.path().starts_with("/upload/drive/"),
        "Unexpected upload destination"
    );
    Ok(())
}
async fn checked(res: Response) -> Result<Response> {
    if res.status().is_success() {
        return Ok(res);
    }
    let status = res.status().as_u16();
    bail!(match status {
        401 => "Google authorization expired; reconnect in Settings",
        403 => "Google Drive denied access or its rate limit was reached",
        404 => "A Drive object is missing or no longer accessible",
        409 => "Drive object already exists; reconciliation is required",
        412 => "Drive content changed during backup; review the conflict",
        429 => "Google Drive rate limit reached; backup will retry",
        500..=599 => "Google Drive is temporarily unavailable; backup will retry",
        _ => "Google Drive rejected this request",
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn acknowledgements_are_bounded() {
        assert_eq!(acknowledged(None, 10).unwrap(), 0);
        assert_eq!(acknowledged(Some("bytes=0-7"), 10).unwrap(), 8);
        assert!(acknowledged(Some("bytes=0-10"), 10).is_err());
        assert!(acknowledged(Some("bytes=5-7"), 10).is_err());
    }
    #[test]
    fn sessions_cannot_redirect_credentials() {
        for bad in [
            "http://www.googleapis.com/upload/drive/v3",
            "https://evil.example/upload/drive/",
            "https://www.googleapis.com.evil.example/upload/drive/",
            "https://user@www.googleapis.com/upload/drive/",
        ] {
            assert!(valid_session(bad).is_err());
        }
        assert!(
            valid_session("https://www.googleapis.com/upload/drive/v3/files?upload_id=123").is_ok()
        );
    }
    #[test]
    fn green_requires_hash_and_size() {
        let m = Metadata {
            size: Some("3".into()),
            md5_checksum: Some("good".into()),
            ..Default::default()
        };
        assert!(m.matches("unused", "good", 3));
        assert!(!m.matches("unused", "bad", 3));
        assert!(!m.matches("unused", "good", 4));
    }
}
