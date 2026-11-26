use crate::{
    model::Dashboard,
    publisher::{
        CLOUD_BASE, Cloud, Prepared, PublishOutcome, Result, Settings, lock, read_optional, save,
        send_prepared,
    },
};
use std::{
    fs,
    io::{BufRead, Write},
    path::Path,
};

pub fn configure(
    dir: &Path,
    cloud: &Cloud,
    dashboard: &Dashboard,
    input: &mut impl BufRead,
    output: &mut impl Write,
    save_key: impl FnOnce() -> Result<()>,
) -> Result<bool> {
    configure_with_clock(dir, cloud, dashboard, input, output, save_key, || {
        chrono::Local::now().fixed_offset()
    })
}

pub fn configure_with_clock(
    dir: &Path,
    cloud: &Cloud,
    dashboard: &Dashboard,
    input: &mut impl BufRead,
    output: &mut impl Write,
    save_key: impl FnOnce() -> Result<()>,
    mut now: impl FnMut() -> chrono::DateTime<chrono::FixedOffset>,
) -> Result<bool> {
    fs::create_dir_all(dir)?;
    // Two setup sessions must never present different images at the same preview path.
    let setup_lock = fs::File::options()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(dir.join("setup.lock"))?;
    setup_lock.try_lock().map_err(|_| "另一个 setup 正在运行")?;
    let previous = read_optional::<Settings>(&dir.join("settings.json"))?;
    let devices = cloud.devices()?;
    if devices.is_empty() {
        return Err("没有发现兼容的 NOTE4 设备".into());
    }
    for (i, device) in devices.iter().enumerate() {
        writeln!(output, "{}. {}", i + 1, device.alias)?;
    }
    let default = previous
        .as_ref()
        .and_then(|s| devices.iter().position(|d| d.device_id == s.device_id))
        .map_or(1, |index| index + 1);
    let index = choice(input, output, "选择设备", default, devices.len())? - 1;
    let page = choice(
        input,
        output,
        "选择持久页面 pageId",
        previous.as_ref().map_or(1, |s| usize::from(s.page_id)),
        5,
    )?;
    let settings = Settings {
        device_id: devices[index].device_id.clone(),
        page_id: page as u8,
    };
    let preview = dir.join("pending-preview.png");
    let mut current = dashboard.clone();
    current.now = now();
    let (_publisher_lock, outcome) = loop {
        let prepared = Prepared::new(&current)?;
        let png = prepared.png()?;
        save(&preview, &png)?;
        writeln!(output, "已生成预览 {}", preview.display())?;
        if !yes(input, output, "确认用该预览替换选定页面？", false)? {
            writeln!(output, "已取消上传。")?;
            return Ok(false);
        }
        let publisher_lock = lock(dir)?;
        current.now = now();
        let confirmed = Prepared::new(&current)?;
        if confirmed.png()? != png {
            writeln!(output, "时间已更新，请确认新预览。")?;
            continue;
        }
        break (
            publisher_lock,
            send_prepared(dir, cloud, &settings, &confirmed, current.now.timestamp())?,
        );
    };
    if let PublishOutcome::Deferred { until } = outcome {
        return Err(format!("上传间隔未满，请在 Unix 时间 {until} 后重试").into());
    }
    save_key()?;
    save(
        &dir.join("settings.json"),
        &serde_json::to_vec_pretty(&settings)?,
    )?;
    writeln!(output, "设置已保存。")?;
    Ok(true)
}

fn line(input: &mut impl BufRead, output: &mut impl Write, prompt: &str) -> Result<String> {
    write!(output, "{prompt} ")?;
    output.flush()?;
    let mut answer = String::new();
    input.read_line(&mut answer)?;
    Ok(answer.trim().into())
}

fn yes(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
    default: bool,
) -> Result<bool> {
    let suffix = if default { "[Y/n]" } else { "[y/N]" };
    match line(input, output, &format!("{prompt} {suffix}"))?
        .to_ascii_lowercase()
        .as_str()
    {
        "" => Ok(default),
        "y" | "yes" => Ok(true),
        "n" | "no" => Ok(false),
        _ => Err("请输入 y 或 n".into()),
    }
}

fn choice(
    input: &mut impl BufRead,
    output: &mut impl Write,
    prompt: &str,
    default: usize,
    max: usize,
) -> Result<usize> {
    let value = line(input, output, &format!("{prompt} (1–{max}) [{default}]"))?;
    let value = if value.is_empty() {
        default
    } else {
        value.parse()?
    };
    if !(1..=max).contains(&value) {
        return Err("选择超出范围".into());
    }
    Ok(value)
}

use std::{env, io, path::PathBuf};
use zeroize::Zeroizing;
const SERVICE: &str = "ai-quota-zectrix-dashboard";
const ACCOUNT: &str = "zectrix-api-key";

pub fn data_dir() -> Result<PathBuf> {
    if let Some(path) = env::var_os("AI_QUOTA_DATA_DIR") {
        return Ok(path.into());
    }
    let home = env::var_os("HOME").ok_or("无法确定用户主目录")?;
    Ok(PathBuf::from(home)
        .join("Library/Application Support")
        .join(SERVICE))
}

pub fn keychain_key() -> Result<Option<Zeroizing<String>>> {
    match security_framework::passwords::get_generic_password(SERVICE, ACCOUNT) {
        Ok(raw) => {
            let bytes = Zeroizing::new(raw);
            Ok(Some(Zeroizing::new(
                std::str::from_utf8(&bytes)?.to_owned(),
            )))
        }
        Err(e) if e.code() == -25300 => Ok(None),
        Err(e) => Err(e.into()),
    }
}

pub fn run_setup(dir: &Path, dashboard: &Dashboard) -> Result<()> {
    let mut input = io::stdin().lock();
    let mut output = io::stdout().lock();
    let existing = keychain_key()?;
    let (key, replace) = match existing {
        Some(key)
            if yes(
                &mut input,
                &mut output,
                "使用钥匙串中的现有 API Key？",
                true,
            )? =>
        {
            (key, false)
        }
        _ => {
            let key = Zeroizing::new(rpassword::prompt_password("ZECTRIX API Key（输入隐藏） ")?);
            if key.is_empty() {
                return Err("API Key 不能为空".into());
            }
            (key, true)
        }
    };
    let cloud = Cloud::new(CLOUD_BASE, &key)?;
    configure(dir, &cloud, dashboard, &mut input, &mut output, || {
        if replace {
            security_framework::passwords::set_generic_password(SERVICE, ACCOUNT, key.as_bytes())?;
        }
        Ok(())
    })?;
    Ok(())
}
