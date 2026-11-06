//! Desktop public-client OAuth: browser + PKCE + random state + loopback callback.
//! All credential material is DPAPI-protected and never serialized to the UI.
use crate::model::now;
use anyhow::{bail, Context, Result};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine};
use omnisync_windows::{filesystem, security};
use rand::{rngs::OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    sync::Arc,
    time::Duration,
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
    sync::Mutex,
};
#[derive(Clone, Serialize, Deserialize)]
pub struct ClientConfig {
    pub client_id: String,
    #[serde(default)]
    pub client_secret: String,
}
#[derive(Clone, Serialize, Deserialize)]
struct Tokens {
    access_token: String,
    refresh_token: String,
    expires_at: i64,
}
#[derive(Clone, Serialize, Deserialize, Default)]
struct Secrets {
    client: Option<ClientConfig>,
    tokens: Option<Tokens>,
}
#[derive(Clone)]
pub struct Auth {
    path: PathBuf,
    secrets: Arc<Mutex<Secrets>>,
    http: reqwest::Client,
}
impl Auth {
    pub fn new(state: &Path) -> Result<Self> {
        let path = state.join("credentials.dpapi");
        let secrets = if path.exists() {
            serde_json::from_slice(&security::unprotect(&std::fs::read(&path)?)?)?
        } else {
            Secrets::default()
        };
        Ok(Self {
            path,
            secrets: Arc::new(Mutex::new(secrets)),
            http: reqwest::Client::builder()
                .timeout(Duration::from_secs(15))
                .build()?,
        })
    }
    pub async fn configured(&self) -> bool {
        self.secrets.lock().await.client.is_some()
    }
    pub async fn connected(&self) -> bool {
        self.secrets.lock().await.tokens.is_some()
    }
    fn save(&self, secrets: &Secrets) -> Result<()> {
        let bytes = security::protect(&serde_json::to_vec(secrets)?)?;
        let temp = self.path.with_extension("tmp");
        filesystem::write_durable(&temp, &bytes)?;
        filesystem::atomic_replace(&temp, &self.path)
    }
    pub async fn import_client(&self, path: &Path) -> Result<()> {
        let meta = std::fs::metadata(path)?;
        anyhow::ensure!(meta.len() < 1024 * 1024, "OAuth configuration is too large");
        let value: serde_json::Value = serde_json::from_slice(&std::fs::read(path)?)?;
        let config: ClientConfig = serde_json::from_value(
            value
                .get("installed")
                .context("Choose a Google OAuth Desktop app JSON file")?
                .clone(),
        )?;
        anyhow::ensure!(
            config.client_id.ends_with(".apps.googleusercontent.com"),
            "Invalid Google desktop client ID"
        );
        let mut secrets = self.secrets.lock().await;
        if secrets.tokens.is_some() {
            bail!("Disconnect the account before changing OAuth clients");
        }
        secrets.client = Some(config);
        self.save(&secrets)
    }
    pub async fn connect(&self) -> Result<()> {
        let client = self
            .secrets
            .lock()
            .await
            .client
            .clone()
            .context("Import Google Desktop OAuth client JSON in Settings first")?;
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let redirect = format!(
            "http://127.0.0.1:{}/callback",
            listener.local_addr()?.port()
        );
        let verifier = random();
        let state = random();
        let challenge = URL_SAFE_NO_PAD.encode(Sha256::digest(verifier.as_bytes()));
        let mut url = url::Url::parse("https://accounts.google.com/o/oauth2/v2/auth")?;
        url.query_pairs_mut().extend_pairs([
            ("client_id", client.client_id.as_str()),
            ("redirect_uri", &redirect),
            ("response_type", "code"),
            ("scope", "https://www.googleapis.com/auth/drive.file"),
            ("access_type", "offline"),
            ("prompt", "consent"),
            ("code_challenge", &challenge),
            ("code_challenge_method", "S256"),
            ("state", &state),
        ]);
        omnisync_windows::integration::open_url(url.as_str())?;
        let code=tokio::time::timeout(Duration::from_secs(300),async{
            loop {let(mut socket,peer)=listener.accept().await?;if !peer.ip().is_loopback(){continue;}
                let mut data=Vec::new();let mut chunk=[0u8;1024];
                loop{let n=tokio::time::timeout(Duration::from_secs(5),socket.read(&mut chunk)).await??;if n==0{break;}data.extend_from_slice(&chunk[..n]);if data.len()>8192{break;}if data.windows(4).any(|w|w==b"\r\n\r\n"){break;}}
                if data.len()>8192{continue;}let request=String::from_utf8_lossy(&data);let first=request.lines().next().unwrap_or("");let fields:Vec<_>=first.split_whitespace().collect();if fields.len()!=3||fields[0]!="GET"{continue;}
                let callback=url::Url::parse(&format!("http://127.0.0.1{}",fields[1]))?;let pairs:std::collections::HashMap<_,_>=callback.query_pairs().into_owned().collect();
                if callback.path()!="/callback"||pairs.get("state")!=Some(&state){let _=socket.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;continue;}
                let body="OmniSync received the sign-in response. You can close this tab.";let response=format!("HTTP/1.1 200 OK\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());socket.write_all(response.as_bytes()).await?;
                if pairs.contains_key("error"){bail!("Google sign-in was declined or canceled");}return pairs.get("code").cloned().context("Google did not return an authorization code");
            }
        }).await.context("Google sign-in timed out")??;
        let response = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "authorization_code"),
                ("client_id", &client.client_id),
                ("client_secret", &client.client_secret),
                ("code", &code),
                ("code_verifier", &verifier),
                ("redirect_uri", &redirect),
            ])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Cannot reach Google sign-in service"))?;
        let tokens = token_response(response, None).await?;
        let mut secrets = self.secrets.lock().await;
        secrets.tokens = Some(tokens);
        self.save(&secrets)
    }
    pub async fn access_token(&self, force: bool) -> Result<String> {
        let mut secrets = self.secrets.lock().await;
        let old = secrets
            .tokens
            .clone()
            .context("Connect Google Drive to begin backup")?;
        if !force && old.expires_at > now() + 60 {
            return Ok(old.access_token);
        }
        let client = secrets
            .client
            .clone()
            .context("OAuth configuration is missing")?;
        let response = self
            .http
            .post("https://oauth2.googleapis.com/token")
            .form(&[
                ("grant_type", "refresh_token"),
                ("client_id", &client.client_id),
                ("client_secret", &client.client_secret),
                ("refresh_token", &old.refresh_token),
            ])
            .send()
            .await
            .map_err(|_| anyhow::anyhow!("Cannot refresh Google authorization while offline"))?;
        let tokens = token_response(response, Some(old.refresh_token)).await?;
        let access = tokens.access_token.clone();
        secrets.tokens = Some(tokens);
        self.save(&secrets)?;
        Ok(access)
    }
    pub async fn disconnect(&self) -> Result<()> {
        let mut secrets = self.secrets.lock().await;
        if let Some(tokens) = &secrets.tokens {
            let _ = self
                .http
                .post("https://oauth2.googleapis.com/revoke")
                .form(&[("token", &tokens.refresh_token)])
                .send()
                .await;
        }
        secrets.tokens = None;
        self.save(&secrets)
    }
}
fn random() -> String {
    let mut bytes = [0u8; 32];
    OsRng.fill_bytes(&mut bytes);
    URL_SAFE_NO_PAD.encode(bytes)
}
async fn token_response(
    response: reqwest::Response,
    old_refresh: Option<String>,
) -> Result<Tokens> {
    if !response.status().is_success() {
        bail!("Google authorization failed. Reconnect the account and check the consent configuration.");
    }
    let json: serde_json::Value = response
        .json()
        .await
        .context("Invalid Google authorization response")?;
    Ok(Tokens {
        access_token: json["access_token"]
            .as_str()
            .context("Missing access token")?
            .into(),
        refresh_token: json["refresh_token"]
            .as_str()
            .map(String::from)
            .or(old_refresh)
            .context("Google did not grant offline access; reconnect and grant consent")?,
        expires_at: now() + json["expires_in"].as_i64().unwrap_or(3600),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn imported_client_is_durable_and_encrypted() {
        let temp = tempfile::tempdir().unwrap();
        let state = temp.path().join("state");
        security::private_directory(&state, false).unwrap();
        let path = temp.path().join("client.json");
        std::fs::write(&path,br#"{"installed":{"client_id":"unit-test.apps.googleusercontent.com","client_secret":"test"}}"#).unwrap();
        let auth = Auth::new(&state).unwrap();
        auth.import_client(&path).await.unwrap();
        assert!(auth.configured().await);
        drop(auth);
        let bytes = std::fs::read(state.join("credentials.dpapi")).unwrap();
        assert!(!String::from_utf8_lossy(&bytes).contains("unit-test.apps"));
        let reopened = Auth::new(&state).unwrap();
        assert!(reopened.configured().await);
        assert!(!reopened.connected().await);
    }
}
