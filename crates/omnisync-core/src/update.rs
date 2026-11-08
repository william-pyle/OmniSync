use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    path::{Path, PathBuf},
    time::Duration,
};
use tokio::io::AsyncWriteExt;

const API: &str = "https://api.github.com/repos/william-pyle/OmniSync/releases/latest";
const RELEASES: &str = "https://github.com/william-pyle/OmniSync/releases/download/";
const MAX_INSTALLER: u64 = 512 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Update {
    pub version: String,
    pub notes: String,
    pub url: String,
    pub checksum_url: String,
    pub size: u64,
    pub digest: Option<String>,
}
fn version(value: &str) -> Result<(u64, u64, u64)> {
    let parts: Vec<_> = value.split('.').collect();
    anyhow::ensure!(
        parts.len() == 3
            && parts.iter().all(|p| !p.is_empty()
                && p.bytes().all(|c| c.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))),
        "Unsupported release version"
    );
    Ok((parts[0].parse()?, parts[1].parse()?, parts[2].parse()?))
}
fn client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .user_agent(concat!("OmniSync/", env!("CARGO_PKG_VERSION")))
        .connect_timeout(Duration::from_secs(10))
        .timeout(Duration::from_secs(600))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() >= 5 {
                attempt.error("Too many update redirects")
            } else if url.scheme() == "https"
                && matches!(
                    url.host_str(),
                    Some(
                        "github.com"
                            | "api.github.com"
                            | "release-assets.githubusercontent.com"
                            | "objects.githubusercontent.com"
                    )
                )
            {
                attempt.follow()
            } else {
                attempt.error("Unexpected update host")
            }
        }))
        .build()?)
}
pub fn parse_release(value: &serde_json::Value, current: &str) -> Result<Option<Update>> {
    if value["draft"] == true || value["prerelease"] == true {
        return Ok(None);
    }
    let tag = value["tag_name"]
        .as_str()
        .context("Release has no version")?;
    let number = tag
        .strip_prefix('v')
        .context("Release tag must start with v")?;
    if version(number)? <= version(current)? {
        return Ok(None);
    }
    let name = format!("OmniSync-Setup-{number}-x64.exe");
    let assets = value["assets"]
        .as_array()
        .context("Release has no installer")?;
    let asset = assets
        .iter()
        .find(|a| a["name"] == name)
        .context("Release has no Windows x64 installer")?;
    let checksum = assets
        .iter()
        .find(|a| a["name"] == format!("{name}.sha256"))
        .context("Release has no installer checksum")?;
    let url = asset["browser_download_url"]
        .as_str()
        .context("Installer URL missing")?;
    let checksum_url = checksum["browser_download_url"]
        .as_str()
        .context("Checksum URL missing")?;
    anyhow::ensure!(
        url == format!("{RELEASES}{tag}/{name}") && checksum_url == format!("{url}.sha256"),
        "Unexpected release download URL"
    );
    let size = asset["size"].as_u64().context("Installer size missing")?;
    anyhow::ensure!(
        size > 0 && size <= MAX_INSTALLER,
        "Unsupported installer size"
    );
    let digest = asset["digest"].as_str().map(str::to_owned);
    if let Some(digest) = &digest {
        validate_hash(
            digest
                .strip_prefix("sha256:")
                .context("Unsupported release digest")?,
        )?;
    }
    Ok(Some(Update {
        version: number.into(),
        notes: value["body"]
            .as_str()
            .unwrap_or("")
            .chars()
            .take(12000)
            .collect(),
        url: url.into(),
        checksum_url: checksum_url.into(),
        size,
        digest,
    }))
}
fn validate_hash(hash: &str) -> Result<()> {
    anyhow::ensure!(
        hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
        "Invalid SHA-256 checksum"
    );
    Ok(())
}
async fn bounded_bytes(response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    let mut stream = response.error_for_status()?.bytes_stream();
    let mut data = vec![];
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        anyhow::ensure!(
            data.len() + chunk.len() <= limit,
            "Update response too large"
        );
        data.extend_from_slice(&chunk);
    }
    Ok(data)
}
pub async fn check(current: &str) -> Result<Option<Update>> {
    let response = client()?
        .get(API)
        .timeout(Duration::from_secs(20))
        .header("Accept", "application/vnd.github+json")
        .send()
        .await?;
    if response.status() == reqwest::StatusCode::NOT_FOUND {
        return Ok(None);
    }
    parse_release(
        &serde_json::from_slice(&bounded_bytes(response, 1024 * 1024).await?)?,
        current,
    )
}
pub async fn download(update: &Update, state: &Path) -> Result<PathBuf> {
    // Update is held in native state, never accepted from web content.
    let client = client()?;
    let checksum = bounded_bytes(
        client
            .get(&update.checksum_url)
            .timeout(Duration::from_secs(30))
            .send()
            .await?,
        4096,
    )
    .await?;
    let checksum = String::from_utf8(checksum)?;
    let mut fields = checksum.split_whitespace();
    let hash = fields
        .next()
        .context("Empty installer checksum")?
        .to_ascii_lowercase();
    validate_hash(&hash)?;
    anyhow::ensure!(
        fields.next() == Some(format!("OmniSync-Setup-{}-x64.exe", update.version).as_str())
            && fields.next().is_none(),
        "Checksum filename does not match installer"
    );
    if let Some(digest) = &update.digest {
        anyhow::ensure!(
            digest == &format!("sha256:{hash}"),
            "Release digest and checksum disagree"
        );
    }
    let directory = state.join("updates").join(crate::model::id());
    omnisync_windows::security::private_directory(&directory, false)?;
    let target = directory.join(format!("OmniSync-Setup-{}-x64.exe", update.version));
    let partial = target.with_extension("partial");
    let result = async {
        let response = client.get(&update.url).send().await?.error_for_status()?;
        if let Some(size) = response.content_length() {
            anyhow::ensure!(size == update.size, "Installer download size changed");
        }
        let mut stream = response.bytes_stream();
        let mut output = tokio::fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .open(&partial)
            .await?;
        let mut sha = Sha256::new();
        let mut size = 0;
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            size += chunk.len() as u64;
            anyhow::ensure!(
                size <= update.size && size <= MAX_INSTALLER,
                "Installer exceeds expected size"
            );
            sha.update(&chunk);
            output.write_all(&chunk).await?;
        }
        output.sync_all().await?;
        drop(output);
        anyhow::ensure!(
            size == update.size && hex::encode(sha.finalize()) == hash,
            "Installer checksum verification failed"
        );
        verify_download(&partial, &hash, update.size)?;
        omnisync_windows::filesystem::move_new(&partial, &target)?;
        Ok(target)
    }
    .await;
    let _ = tokio::fs::remove_file(partial).await;
    result
}
pub fn verify_download(path: &Path, expected: &str, expected_size: u64) -> Result<()> {
    use std::io::Read;
    let mut file = std::fs::File::open(path)?;
    let mut magic = [0; 2];
    file.read_exact(&mut magic)?;
    anyhow::ensure!(&magic == b"MZ", "Update is not a Windows executable");
    let (hash, _, size) = crate::staging::hashes(std::fs::File::open(path)?)?;
    anyhow::ensure!(
        hash == expected && size == expected_size,
        "Downloaded installer was modified; download again"
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn release(number: &str) -> serde_json::Value {
        let name = format!("OmniSync-Setup-{number}-x64.exe");
        let url = format!("{RELEASES}v{number}/{name}");
        serde_json::json!({"tag_name":format!("v{number}"),"draft":false,"prerelease":false,"assets":[{"name":name,"browser_download_url":url,"size":100},{"name":format!("{name}.sha256"),"browser_download_url":format!("{url}.sha256")}]})
    }
    #[test]
    fn only_newer_stable_releases_from_this_repo() {
        assert!(parse_release(&release("0.10.0"), "0.2.0")
            .unwrap()
            .is_some());
        for number in ["0.1.0", "0.2.0"] {
            assert!(parse_release(&release(number), "0.2.0").unwrap().is_none());
        }
        let mut r = release("0.3.0");
        r["prerelease"] = true.into();
        assert!(parse_release(&r, "0.2.0").unwrap().is_none());
        r["prerelease"] = false.into();
        r["assets"][0]["browser_download_url"] = "https://example.com/install.exe".into();
        assert!(parse_release(&r, "0.2.0").is_err());
        for number in ["../evil", "0.3.0-beta", "1.2", "01.2.3"] {
            assert!(parse_release(&release(number), "0.2.0").is_err());
        }
    }
    #[test]
    fn tampered_or_non_executable_download_is_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("setup.exe");
        std::fs::write(&path, b"MZinstaller").unwrap();
        let hash = hex::encode(Sha256::digest(b"MZinstaller"));
        verify_download(&path, &hash, 11).unwrap();
        std::fs::write(&path, b"MZtampered!").unwrap();
        assert!(verify_download(&path, &hash, 11).is_err());
        std::fs::write(&path, b"<html>oops</html>").unwrap();
        assert!(verify_download(&path, &hash, 11).is_err());
    }
}
