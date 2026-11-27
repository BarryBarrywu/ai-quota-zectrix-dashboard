use crate::{
    publisher::{Result, save},
    setup::data_dir,
};
use clap::Subcommand;
use serde::Serialize;
use std::{
    collections::BTreeMap,
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
};
const LABEL: &str = "io.github.barrybarrywu.ai-quota-zectrix-dashboard";
const NAME: &str = "ai-quota-zectrix-dashboard";

#[derive(Subcommand)]
pub enum AgentAction {
    /// Print the LaunchAgent definition.
    Plist {
        #[arg(long, default_value = "codex")]
        codex_program: PathBuf,
    },
    /// Copy the companion to Application Support and start it at login.
    Install {
        #[arg(long, default_value = "codex")]
        codex_program: PathBuf,
    },
    /// Stop and remove the managed agent and executable.
    Remove,
}

#[derive(Serialize)]
#[serde(rename_all = "PascalCase")]
struct Agent {
    label: &'static str,
    program_arguments: Vec<PathBuf>,
    environment_variables: BTreeMap<&'static str, PathBuf>,
    run_at_load: bool,
    keep_alive: bool,
    throttle_interval: u32,
    exit_time_out: u32,
}
fn definition(binary: &Path, dir: &Path, codex: &Path) -> Result<Vec<u8>> {
    let codex = if codex.components().count() == 1 {
        env::split_paths(&env::var_os("PATH").unwrap_or_default())
            .map(|dir| dir.join(codex))
            .find(|p| p.is_file())
            .ok_or("找不到 Codex，请使用 --codex-program 指定可执行文件")?
    } else {
        codex.to_owned()
    };
    let mut vars = BTreeMap::from([("AI_QUOTA_DATA_DIR", dir.to_owned())]);
    if let Some(path) = env::var_os("AI_QUOTA_STATUSLINE_PATH") {
        vars.insert("AI_QUOTA_STATUSLINE_PATH", std::path::absolute(path)?);
    }
    if let Some(path) = crate::antigravity::cli_path() {
        vars.insert("AI_QUOTA_AGY_BIN", std::path::absolute(path)?);
    }
    if let Some(proxy) = env::var("AI_QUOTA_AGY_PROXY").ok().or_else(|| {
        ["HTTPS_PROXY", "https_proxy", "ALL_PROXY", "all_proxy"]
            .into_iter()
            .find_map(|k| env::var(k).ok())
            .filter(|v| !v.trim().is_empty())
    }) {
        let url = reqwest::Url::parse(&proxy).map_err(|_| "invalid Antigravity proxy URL")?;
        if !matches!(url.scheme(), "http" | "https" | "socks5" | "socks5h")
            || url.host().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/")
        {
            return Err("use a proxy address without credentials, path, query or fragment".into());
        }
        vars.insert("AI_QUOTA_AGY_PROXY", proxy.into());
    }
    let agent = Agent {
        label: LABEL,
        program_arguments: vec![
            binary.into(),
            "run".into(),
            "--codex-program".into(),
            std::path::absolute(codex)?,
        ],
        environment_variables: vars,
        run_at_load: true,
        keep_alive: true,
        throttle_interval: 30,
        exit_time_out: 60,
    };
    let mut buf = vec![];
    plist::to_writer_xml(&mut buf, &agent)?;
    Ok(buf)
}
fn launchctl(args: &[&std::ffi::OsStr]) -> Result<bool> {
    Ok(Command::new("/bin/launchctl")
        .args(args)
        .output()?
        .status
        .success())
}

pub fn manage(action: AgentAction) -> Result<()> {
    let dir = std::path::absolute(data_dir()?)?;
    let binary = dir.join("bin").join(NAME);
    let plist = PathBuf::from(env::var_os("HOME").ok_or("缺少用户主目录")?)
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"));
    let domain = format!("gui/{}", unsafe { libc::getuid() });
    let service = format!("{domain}/{LABEL}");
    let stop = || -> Result<()> {
        if launchctl(&["print".as_ref(), service.as_ref()])? {
            if !launchctl(&["bootout".as_ref(), service.as_ref()])? {
                return Err("launchctl bootout 失败".into());
            }
            for _ in 0..30 {
                if !launchctl(&["print".as_ref(), service.as_ref()])? {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
        }
        Ok(())
    };
    match action {
        AgentAction::Plist { codex_program } => {
            use std::io::Write;
            io::stdout().write_all(&definition(&binary, &dir, &codex_program)?)?;
        }
        AgentAction::Install { codex_program } => {
            let content = definition(&binary, &dir, &codex_program)?;
            let bin_dir = binary.parent().unwrap();
            fs::create_dir_all(bin_dir)?;
            fs::create_dir_all(plist.parent().unwrap())?;
            let tmp = tempfile::NamedTempFile::new_in(bin_dir)?;
            fs::copy(env::current_exe()?, tmp.path())?;
            tmp.as_file().sync_all()?;
            stop()?;
            tmp.persist(binary)?;
            save(&plist, &content)?;
            let mut bootstrapped = false;
            for _ in 0..5 {
                if launchctl(&["bootstrap".as_ref(), domain.as_ref(), plist.as_os_str()])? {
                    bootstrapped = true;
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(200));
            }
            if !bootstrapped {
                return Err("launchctl bootstrap 失败".into());
            }
        }
        AgentAction::Remove => {
            stop()?;
            for path in [plist, binary] {
                match fs::remove_file(path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => return Err(e.into()),
                    _ => (),
                }
            }
        }
    }
    Ok(())
}
