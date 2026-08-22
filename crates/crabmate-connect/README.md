# crabmate-connect

桌面 / 移动 **Tauri 2** 壳共用的「连接远程 `crabmate serve`」逻辑（探测、Bearer hash 交接、钥匙串）。

本 crate 已迁入 **`crabmate-client`** 仓，由 `desktop-tauri` / `mobile-tauri` **本仓 path** 引用（禁止再 path 回 Server 主仓）。

## 版本与钉依赖

Server 侧契约发版策略（`api-contract` / `sse-protocol`）：  
[client_contract_versioning.md](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_contract_versioning.md)  
本仓钉法摘要：[contract_pin.md](../../docs/design/contract_pin.md)

- Crate **`version`**：本目录 `Cargo.toml`（semver）
- **`publish = false`**：随本仓发版；不强制 crates.io
- **Tauri**：可选 feature **`tauri`**（`tauri = "2"`）。默认关闭，probe / handoff / keyring 测试不编 GTK/WebKit。Desktop / Android 须 `features = ["tauri"]`

本仓壳：

```toml
crabmate-connect = { path = "../../crates/crabmate-connect", features = ["tauri"] }
```

## 能力边界

- 探测 `GET /health` 与受保护的 prefs；非空 Bearer 经 `#cm_web_api_bearer=` 交给前端
- 桌面：连接成功后非空 Bearer 写系统钥匙串；Android：钥匙串不可用时由壳 `SecureBearerStore`（AndroidKeyStore AES-GCM，连接页 Origin 桥）落盘，**不**再写明文 localStorage
- **不**实现聊天 / SSE；线协议版本错位由 UI 与 `serve` 按 `SSE_PROTOCOL_VERSION` 与稳定错误码处理

静态页：`assets/connect.html`（`bash scripts/sync-tauri-connect-page.sh` 同步进壳 `dist/`）
