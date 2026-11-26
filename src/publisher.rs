use reqwest::{
    Url,
    blocking::Client,
    header::{HeaderMap, HeaderValue},
    redirect::Policy,
};
use serde::Deserialize;
use std::{error::Error, time::Duration};

pub const CLOUD_BASE: &str = "https://cloud.zectrix.com";
pub type Result<T> = std::result::Result<T, Box<dyn Error>>;

pub struct Cloud {
    client: Client,
    base: Url,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Device {
    pub device_id: String,
    pub alias: String,
    board: String,
}

#[derive(Deserialize)]
struct Response {
    code: i64,
    #[serde(default)]
    data: serde_json::Value,
}

impl Cloud {
    pub fn new(base: &str, key: &str) -> Result<Self> {
        let mut headers = HeaderMap::new();
        let mut secret = HeaderValue::from_str(key)?;
        secret.set_sensitive(true);
        headers.insert("X-API-Key", secret);
        let client = Client::builder()
            .default_headers(headers)
            .redirect(Policy::none())
            .timeout(Duration::from_secs(30))
            .build()?;
        Ok(Self {
            client,
            base: Url::parse(base)?,
        })
    }

    pub fn devices(&self) -> Result<Vec<Device>> {
        let response: Response = self
            .client
            .get(self.base.join("/open/v1/devices")?)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|_| "ZECTRIX 设备发现失败")?;
        if response.code != 0 {
            return Err("ZECTRIX 设备发现失败".into());
        }
        let devices: Vec<Device> = serde_json::from_value(response.data)?;
        Ok(devices
            .into_iter()
            .filter(|d| {
                matches!(
                    d.board.as_str(),
                    "zectrix-s3-epaper-4.2" | "bread-compact-wifi" | "极趣 S3 墨水屏 4.2寸"
                )
            })
            .collect())
    }
}

use crate::{Frame, model::Dashboard, render};
use reqwest::blocking::multipart::{Form, Part};
use serde::{Serialize, de::DeserializeOwned};
use sha2::{Digest, Sha256};
use std::{fs, io, path::Path};

#[derive(Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Settings {
    pub device_id: String,
    pub page_id: u8,
}
impl Settings {
    pub fn load(dir: &Path) -> Result<Self> {
        let settings: Self = serde_json::from_slice(&fs::read(dir.join("settings.json"))?)?;
        settings.validate()?;
        Ok(settings)
    }

    fn validate(&self) -> Result<()> {
        if self.device_id.is_empty() || !(1..=5).contains(&self.page_id) {
            return Err("ZECTRIX 设备或页面配置无效".into());
        }
        Ok(())
    }
}

pub(crate) fn read_optional<T: DeserializeOwned>(path: &Path) -> Result<Option<T>> {
    match fs::read(path) {
        Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
        Err(e) if e.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub(crate) fn save(path: &Path, bytes: &[u8]) -> Result<()> {
    use io::Write;
    let mut tmp = tempfile::NamedTempFile::new_in(path.parent().ok_or("缺少父目录")?)?;
    tmp.write_all(bytes)?;
    tmp.as_file().sync_all()?;
    tmp.persist(path)?;
    Ok(())
}

impl Cloud {
    fn upload(&self, settings: &Settings, image: Vec<u8>) -> Result<()> {
        let mut url = self.base.join("/open/v1/devices")?;
        url.path_segments_mut()
            .map_err(|_| "ZECTRIX 地址无效")?
            .extend([settings.device_id.as_str(), "display", "image"]);
        let form = Form::new()
            .text("dither", "false")
            .text("pageId", settings.page_id.to_string())
            .part(
                "images",
                Part::bytes(image)
                    .file_name("dashboard.png")
                    .mime_str("image/png")?,
            );
        let response: Response = self
            .client
            .post(url)
            .multipart(form)
            .send()
            .and_then(|r| r.error_for_status())
            .and_then(|r| r.json())
            .map_err(|_| "ZECTRIX 未确认图像上传")?;
        if response.code != 0 {
            return Err("ZECTRIX 拒绝图像上传".into());
        }
        Ok(())
    }
}

#[derive(Debug, PartialEq, Eq)]
pub enum PublishOutcome {
    Published,
    Unchanged,
    Deferred { until: i64 },
}

#[derive(Default, Serialize, Deserialize)]
struct State {
    next_attempt: i64,
    destination: String,
    visible_hash: String,
    frame_hash: String,
    last_success: Option<i64>,
}

pub fn last_success(dir: &Path) -> Result<Option<i64>> {
    Ok(read_optional::<State>(&dir.join("publisher.json"))?.and_then(|s| s.last_success))
}

pub(crate) struct Prepared {
    visible_hash: String,
    frame: Frame,
}

impl Prepared {
    pub(crate) fn new(dashboard: &Dashboard) -> Result<Self> {
        // The upload time describes the update; it must never trigger one by itself.
        let mut d = dashboard.clone();
        d.screen_updated_at = None;
        let visible_hash = render(&d)?.sha256();
        d.screen_updated_at = Some(d.now);
        Ok(Self {
            visible_hash,
            frame: render(&d)?,
        })
    }
    pub(crate) fn png(&self) -> Result<Vec<u8>> {
        Ok(self.frame.png_bytes()?)
    }
}

pub(crate) fn lock(dir: &Path) -> Result<fs::File> {
    fs::create_dir_all(dir)?;
    let lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("publisher.lock"))?;
    lock.lock()?;
    Ok(lock)
}

pub(crate) fn send_prepared(
    dir: &Path,
    cloud: &Cloud,
    settings: &Settings,
    prepared: &Prepared,
    now: i64,
) -> Result<PublishOutcome> {
    settings.validate()?;
    let path = dir.join("publisher.json");
    let mut state: State = read_optional(&path)?.unwrap_or_default();
    let destination = format!("{:x}", Sha256::digest(serde_json::to_vec(settings)?));
    if destination == state.destination && state.visible_hash == prepared.visible_hash {
        return Ok(PublishOutcome::Unchanged);
    }
    if now < state.next_attempt {
        return Ok(PublishOutcome::Deferred {
            until: state.next_attempt,
        });
    }
    state.next_attempt = now.saturating_add(60);
    save(&path, &serde_json::to_vec(&state)?)?;
    cloud.upload(settings, prepared.png()?)?;
    state.visible_hash = prepared.visible_hash.clone();
    state.frame_hash = prepared.frame.sha256();
    state.destination = destination;
    state.last_success = Some(now);
    save(&path, &serde_json::to_vec(&state)?)?;
    Ok(PublishOutcome::Published)
}

pub fn publish_changed(
    dir: &Path,
    cloud: &Cloud,
    settings: &Settings,
    dashboard: &Dashboard,
) -> Result<PublishOutcome> {
    let _lock = lock(dir)?;
    if let Some(current) = read_optional::<Settings>(&dir.join("settings.json"))?
        && current != *settings
    {
        return Err("设备或页面已变更，请重新读取设置".into());
    }
    send_prepared(
        dir,
        cloud,
        settings,
        &Prepared::new(dashboard)?,
        dashboard.now.timestamp(),
    )
}
