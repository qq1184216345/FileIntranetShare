/// Windows 防火墙规则自助管理。
///
/// 设计（P2-14 重构）：
/// - 规则由安装器（installer.nsh）在安装时一次性添加、卸载时移除，避免每次启动弹 UAC。
/// - 运行时 `ensure_rule` 只做**静默探测**：若规则已存在直接返回；缺失则记录 warn 提醒用户
///   在设置页手动"修复防火墙规则"。不再主动拉起 UAC（避免改端口后每次启动都被打扰）。
/// - 用户显式点击"修复防火墙规则"时，由专门的 `try_add_rule_elevated` 拉 UAC；
///   这是一个显式的、一次性的、用户主动触发的操作。
use std::sync::atomic::{AtomicBool, Ordering};

/// 进程内记录"本次已探测过的端口"，避免 onConfigChange 等场景反复 exec netsh
static CHECKED: AtomicBool = AtomicBool::new(false);

#[cfg(windows)]
pub fn ensure_rule(port: u16) {
    if CHECKED.swap(true, Ordering::AcqRel) {
        return;
    }
    use std::process::Command;
    let rule_name = format!("FileShare-{port}");

    let check = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &format!("name={rule_name}"),
        ])
        .output();

    match check {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            if stdout.contains(&rule_name) {
                tracing::info!("firewall rule `{rule_name}` exists");
            } else {
                tracing::warn!(
                    "firewall rule `{rule_name}` missing. LAN peers may not reach this port. \
                     You can click 'Repair firewall rule' in Settings to add it (requires UAC)."
                );
            }
        }
        Err(e) => tracing::warn!("firewall check failed: {e}"),
    }
}

/// 用户显式触发的"修复规则"——这里才拉 UAC。对应前端设置页按钮调用。
#[cfg(windows)]
#[allow(dead_code)]
pub fn try_add_rule_elevated(port: u16) -> Result<(), String> {
    use std::process::Command;
    let rule_name = format!("FileShare-{port}");
    let ps_script = format!(
        r#"Start-Process netsh -ArgumentList 'advfirewall firewall add rule name="{rule_name}" dir=in action=allow protocol=TCP localport={port} profile=any' -Verb RunAs -WindowStyle Hidden"#
    );
    Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .spawn()
        .map_err(|e| format!("launch powershell failed: {e}"))?;
    Ok(())
}

#[cfg(not(windows))]
pub fn ensure_rule(_port: u16) {}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn try_add_rule_elevated(_port: u16) -> Result<(), String> {
    Ok(())
}

/// 保留给"卸载时清理"使用。现在安装器已接管，运行时仅在用户显式操作时调用。
#[cfg(windows)]
#[allow(dead_code)]
pub fn remove_rule(port: u16) {
    use std::process::Command;
    let rule_name = format!("FileShare-{port}");
    let ps_script = format!(
        r#"Start-Process netsh -ArgumentList 'advfirewall firewall delete rule name="{rule_name}"' -Verb RunAs -WindowStyle Hidden"#
    );
    let _ = Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .spawn();
}

#[cfg(not(windows))]
#[allow(dead_code)]
pub fn remove_rule(_port: u16) {}
