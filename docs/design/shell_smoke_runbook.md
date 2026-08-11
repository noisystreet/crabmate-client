# Client 壳冒烟（Desktop / Android）

> **状态**：人工勾选；默认不进 pre-commit。  
> **Server 侧全量三端清单**（CLI/TUI/Web/契约）：主仓 [`client_turn_smoke_runbook.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_turn_smoke_runbook.md)。  
> **本文件**：只覆盖 **壳仓** 步骤（对应主仓 runbook §4.4 / §4.5 / Client 矩阵 C3–C5）。

## 1. 前置

| 项 | 说明 |
|----|------|
| `serve` | 已安装或已编译的 **`crabmate serve`**（勿 path 编译回主开发树做正式验收；开发期可用同级 `../crabmate_agent`） |
| 业务 UI | 本仓 `make frontend` → `prepare-sidecar` 进 `desktop-tauri/dist`；连接后壳加载包内 `index.html` |
| Bearer | 连接页填 **Web API 共享密钥**，不是模型 `API_KEY` |
| 提示词 | `用一句话介绍你自己` → 助手终答或流式结束 |

## 2. Desktop

```bash
# 终端 A — 纯 API serve（壳加载包内 UI；Server 默认 CORS 已含壳 Origin）
crabmate serve --host 127.0.0.1 --port 8080
# 或: cd ../crabmate_agent && cargo run -- serve --host 127.0.0.1 --port 8080

# 终端 B（本仓）
make frontend && make prepare-sidecar
cd desktop-tauri/src-tauri
# 可选: CM_DESKTOP_SUGGESTED_URL=http://127.0.0.1:8080/
cargo tauri dev
```

- [x] 连接页预填本机 URL → 探测成功进入 UI（2026-08-08 验收用 `CM_DESKTOP_SKIP_CONNECT` + `CM_DESKTOP_SERVE_URL` 跳过连接页直达 UI）  
- [x] 一轮对话（同日：干净克隆 release 壳对接本仓/主仓 `:18080` UI/`serve`；`POST /chat/stream` v2 得助手终答）  
- [ ] （可选）改填 LAN 上另一台 `serve`；非回环时桌面 IPC 受限符合预期  

跳过连接页（E2E）：`CM_E2E_FIXTURES=1` 或 `CM_DESKTOP_SKIP_CONNECT=1`，且必须 `CM_DESKTOP_SERVE_URL=…`。

## 2b. 个人云（远程纯 API）

见 [`personal_cloud_runbook.md`](./personal_cloud_runbook.md)。摘要：

```text
连接页：https://api.example.com/  +  与 VPS CM_WEB_API_BEARER_TOKEN 相同的 Bearer
（包内 UI；serve 默认纯 API，不要 --with-web）
```

- [ ] HTTPS + Bearer 探测成功并进入包内 UI  
- [ ] 一轮对话  

## 3. Mobile 或「桌面当远程壳」

**A. Android**：`./mobile-tauri/scripts/build-apk.sh` → 连接页填 `http://<LAN-IP>:8080/` + Web Bearer → 一轮对话。

**B. 无真机**：Desktop 连接页指向另一 `serve`（或本机第二端口）。

- [ ] 连接成功，hash Bearer 交接后能发消息  
- [ ] 侧栏「断开」/ `?manual=1` 不立刻误重连  
- [ ] Android：连上后系统返回键弹出确认框（非静默退回连接页并自动登录）；选「返回连接页」带 `?manual=1` 且不立刻重连  

## 4. 自动化（Victauri）

```bash
# 须能解析到 crabmate 二进制（优先级见脚本头注释）
./scripts/victauri-e2e.sh all
# 真实 LLM：REAL_LLM_E2E=1 ./scripts/victauri-e2e.sh real_llm
```

CI 分层：

- PR：Playwright Chromium 覆盖工具后 SSE 提前 EOF → `stream_resume` → 终答恢复。
- PR：构建并上传 Android aarch64 APK，验证完整 Android/Tauri 打包链。
- 定时或手动：`.github/workflows/android-emulator-smoke.yml` 构建 x86_64 debug APK，在 API 35 模拟器安装并启动包 `edu.crabmate`。

详见 [`docs/TESTING.md`](../TESTING.md)。

## 5. 执行记录（可选）

```text
日期：
serve 来源：release | PATH | ../crabmate_agent
Desktop / Mobile = pass|fail
备注：
```

勿提交含密钥的日志。
