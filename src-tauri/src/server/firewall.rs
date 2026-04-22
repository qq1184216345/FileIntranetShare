/// Windows 防火墙规则自助管理：首次启动时若入站规则不存在，则尝试通过 UAC 提权添加。
/// 失败不影响服务启动（在 127.0.0.1 上仍可用；LAN 访问将由系统默认弹窗兜底）。
#[cfg(windows)]
pub fn ensure_rule(port: u16) {
    use std::process::Command;
    let rule_name = format!("FileShare-{port}");

    // 先检查是否已存在
    let check = Command::new("netsh")
        .args([
            "advfirewall",
            "firewall",
            "show",
            "rule",
            &format!("name={rule_name}"),
        ])
        .output();

    if let Ok(out) = check {
        // netsh 找到规则时 stdout 有 "Rule Name" / "规则名称" 字段
        let stdout = String::from_utf8_lossy(&out.stdout);
        if stdout.contains(&rule_name) {
            tracing::info!("firewall rule `{rule_name}` already exists, skip");
            return;
        }
    }

    // 通过 PowerShell Start-Process -Verb RunAs 触发 UAC 弹窗
    // 添加 TCP 入站允许规则（IPv4 + IPv6）
    let ps_script = format!(
        r#"Start-Process netsh -ArgumentList 'advfirewall firewall add rule name="{rule_name}" dir=in action=allow protocol=TCP localport={port} profile=any' -Verb RunAs -WindowStyle Hidden"#
    );
    match Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps_script])
        .spawn()
    {
        Ok(_) => tracing::info!("requested firewall rule for port {port} (UAC)"),
        Err(e) => tracing::warn!("failed to request firewall rule: {e}"),
    }
}

#[cfg(not(windows))]
pub fn ensure_rule(_port: u16) {}

/// 保留给后续"卸载时清理"或托盘菜单"移除防火墙规则"功能调用。
/// 当前主流程未使用，但主动暴露比悄悄删除更稳妥 —— 避免以后要清理时找不到入口。
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
