; FileShare NSIS 自定义安装/卸载片段（Tauri bundler 会把它注入到 .nsi 对应 Hook 点）
; 目标：把 Windows 防火墙入站规则的添加/删除从运行时迁移到一次性安装/卸载，
;      避免每次用户启动服务时弹 UAC（P2-14）。
;
; 规则命名：FileShare-18888（默认端口）；用户改端口后需在设置页手动修复。
; 采用 currentUser 安装模式时 NSIS 不会默认以管理员身份运行；这里用 UAC 插件亦非必须，
; 因为 netsh advfirewall 在 Win10/11 下对 currentUser 可写入 "私有/公用" 本地规则。
; 若失败不阻塞安装过程（/IGNORE）。

!macro NSIS_HOOK_POSTINSTALL
  DetailPrint "添加 Windows 防火墙入站规则 (TCP 18888)..."
  nsExec::ExecToLog 'netsh advfirewall firewall add rule name="FileShare-18888" dir=in action=allow protocol=TCP localport=18888 profile=any'
  Pop $0
  ${If} $0 != 0
    DetailPrint "防火墙规则添加失败（可能需要管理员权限），FileShare 仍会正常运行；如需 LAN 访问，请在 FileShare 设置页点击"修复防火墙规则"。"
  ${EndIf}
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  DetailPrint "移除 Windows 防火墙入站规则..."
  nsExec::ExecToLog 'netsh advfirewall firewall delete rule name="FileShare-18888"'
  Pop $0
!macroend
