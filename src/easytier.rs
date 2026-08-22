// easytier VPN 生命周期管理（融合版）
// 启动时拉起 easytier-core，退出时停止；防止重复启动
use hbb_common::log;
use std::process::{Child, Command};
use std::sync::{Mutex, OnceLock};

static EASYTIER_PROC: OnceLock<Mutex<Option<Child>>> = OnceLock::new();

const EASYTIER_BIN: &str = "easytier-core.exe";
const EASYTIER_CONF: &str = "easytier.toml";

fn proc_lock() -> &'static Mutex<Option<Child>> {
    EASYTIER_PROC.get_or_init(|| Mutex::new(None))
}

/// 启动 easytier（在主程序初始化后调用）
pub fn start_easytier() {
    if is_easytier_running() {
        log::info!("[easytier] already running, skip");
        return;
    }
    let exe_dir = match std::env::current_exe() {
        Ok(p) => match p.parent() {
            Some(d) => d.to_path_buf(),
            None => {
                log::warn!("[easytier] cannot get exe dir");
                return;
            }
        },
        Err(_) => {
            log::warn!("[easytier] cannot get current exe");
            return;
        }
    };
    let bin_path = exe_dir.join(EASYTIER_BIN);
    let conf_path = exe_dir.join(EASYTIER_CONF);
    if !bin_path.exists() || !conf_path.exists() {
        log::warn!(
            "[easytier] not found: {} or {}",
            bin_path.display(),
            conf_path.display()
        );
        return;
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        match Command::new(&bin_path)
            .args(["-c", conf_path.to_str().unwrap_or("easytier.toml")])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
        {
            Ok(child) => {
                *proc_lock().lock().unwrap() = Some(child);
                log::info!("[easytier] started: {}", bin_path.display());
            }
            Err(e) => log::error!("[easytier] start failed: {}", e),
        }
    }
    #[cfg(not(windows))]
    {
        match Command::new(&bin_path)
            .args(["-c", conf_path.to_str().unwrap_or("easytier.toml")])
            .spawn()
        {
            Ok(child) => {
                *proc_lock().lock().unwrap() = Some(child);
                log::info!("[easytier] started: {}", bin_path.display());
            }
            Err(e) => log::error!("[easytier] start failed: {}", e),
        }
    }
}

/// 停止 easytier（主程序退出时调用）
pub fn stop_easytier() {
    if let Some(mut child) = proc_lock().lock().unwrap().take() {
        let _ = child.kill();
        let _ = child.wait();
        log::info!("[easytier] stopped (child)");
    }
    // 兜底：杀掉可能残留的进程
    #[cfg(windows)]
    {
        let _ = Command::new("taskkill")
            .args(["/F", "/IM", EASYTIER_BIN])
            .output();
        log::info!("[easytier] stopped (taskkill fallback)");
    }
}

/// 检查 easytier 是否已在运行
fn is_easytier_running() -> bool {
    #[cfg(windows)]
    {
        let out = Command::new("tasklist")
            .args(["/FI", "IMAGENAME eq easytier-core.exe", "/FO", "CSV", "/NH"])
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
            .unwrap_or_default();
        out.contains("easytier-core.exe")
    }
    #[cfg(not(windows))]
    {
        false
    }
}


/// XH60-FIX: 轻量方案 - 获取 VPN 在线节点列表（调用 easytier-cli peer，解析为 JSON）
/// 返回格式: [{"ipv4":"192.168.100.12","hostname":"yyt","cost":"p2p","lat":"5.92","tunnel":"tcp"}, ...]
pub fn get_vpn_nodes_json() -> String {
    use std::process::Command;
    let exe_dir = match std::env::current_exe() {
        Ok(p) => p.parent().map(|d| d.to_path_buf()).unwrap_or_default(),
        Err(_) => return "error:no-exe-dir".to_owned(),
    };
    let cli_path = exe_dir.join("easytier-cli.exe");
    if !cli_path.exists() {
        log::info!("[vpn-nodes] easytier-cli.exe not found, standalone version");
        return "[]".to_owned();
    }
    let out = match Command::new(&cli_path).args(["peer"]).output() {
        Ok(o) => o,
        Err(e) => {
            log::error!("[vpn-nodes] run easytier-cli failed: {}", e);
            return "error:cli-failed".to_owned();
        }
    };
    let text = String::from_utf8_lossy(&out.stdout).to_string();
    parse_peer_table(&text)
}

/// 解析 easytier-cli peer 的表格输出为 JSON 数组
fn parse_peer_table(text: &str) -> String {
    let mut nodes: Vec<serde_json::Value> = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('|') == false {
            continue;
        }
        // 跳过表头与分隔行
        if line.contains("ipv4") && line.contains("hostname") {
            continue;
        }
        if line.contains("-----") {
            continue;
        }
        // 解析 | ip | host | cost | lat | loss | rx | tx | tunnel | NAT | version |
        let cells: Vec<&str> = line
            .trim_matches('|')
            .split('|')
            .map(|s| s.trim())
            .collect();
        if cells.len() < 4 {
            continue;
        }
        let ipv4 = cells[0].trim_end_matches("/24").to_string();
        let hostname = cells[1].to_string();
        let cost = cells[2].to_string();
        let lat = cells[3].to_string();
        let tunnel = cells.get(7).map(|s| s.to_string()).unwrap_or_default();
        if ipv4.is_empty() || hostname.is_empty() {
            continue;
        }
        nodes.push(serde_json::json!({
            "ipv4": ipv4,
            "hostname": hostname,
            "cost": cost,
            "lat": lat,
            "tunnel": tunnel,
        }));
    }
    serde_json::to_string(&nodes).unwrap_or_else(|_| "[]".to_owned())
}
