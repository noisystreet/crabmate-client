# 个人云：壳对接纯 API（Client）

> **读者**：用 Desktop / Android 壳连接自己的 VPS `serve`。  
> **Server 运维权威**：主仓 [`个人VPS部署指南.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/个人VPS部署指南.md)（systemd、Caddy、TLS、Bearer）。  
> **运行时拆分**：主仓 [`client_ui_runtime_split.md`](https://github.com/noisystreet/CrabMate/blob/main/docs/design/client_ui_runtime_split.md) §4。

## 目标拓扑

```text
公网 DNS:  api.example.com  → Caddy(TLS) → 127.0.0.1:8080  crabmate serve
                                                      ↑ 默认纯 API（不要 --with-web）
壳本机:    包内 frontend/dist（tauri://localhost / http://tauri.localhost）
           → HTTPS + Web Bearer → api.example.com
```

| 角色 | 做什么 |
|------|--------|
| 公网 | **只**反代 API；**不要**给业务 UI 配公网 A 记录 |
| `serve` | 听回环；`CM_WEB_API_REQUIRE_BEARER=1` + 非空 `CM_WEB_API_BEARER_TOKEN`；**Server ≥ v0.2.0** 默认 CORS 已含壳 Origin |
| 壳 | 连接页填 `https://api.example.com/` + 同一 Bearer；加载**包内** UI，API 基址指向该 URL |

过渡期若必须在 VPS 上用浏览器打开同机 SPA，才加 `--with-web` + `CM_WEB_STATIC_DIR`；那不是个人云推荐路径。

## Client 侧步骤

1. 安装或构建壳（`make desktop-dev` / `make apk` 等）；确保 `make frontend` + `prepare-sidecar` / `prepare-mobile` 已同步包内 UI。
2. 连接页：
   - **服务器地址**：`https://api.example.com/`（须 HTTPS；公网明文 HTTP 会被壳拒绝）
   - **Web API 共享密钥**：与 VPS 上 `CM_WEB_API_BEARER_TOKEN` **完全一致**（不是模型 `API_KEY`）
3. 探测通过后进入包内 UI，发一轮对话。

可选预填：`CM_DESKTOP_SUGGESTED_URL=https://api.example.com/`。

## 冒烟勾选

- [ ] `GET https://api…/health` 在壳外可用（curl；可不带 Bearer）
- [ ] 连接页 HTTPS + Bearer 探测成功，进入包内 UI
- [ ] 一轮对话 / SSE 正常（状态栏可达「就绪」）
- [ ] 未把 `0.0.0.0` 填进连接地址；未对公网使用 `http://`

## 常见失败

| 现象 | 处理 |
|------|------|
| 明文 HTTP 被拒 | 公网改 HTTPS；或仅在 LAN 用 `http://192.168…` |
| 401 / 缺少 Web API 凭证 | Bearer 与服务器不一致；设置页 / 连接页重填 |
| CORS / Load failed | 确认 Server ≥ **v0.2.0**（默认壳 Origin）；若显式清空了 `CM_WEB_CORS_ALLOWED_ORIGINS` 请 `unset` |
| 连上但 `/` 404 | 正常：纯 API 不挂 SPA；壳应加载包内 UI，不要用浏览器当主入口打开 `api.` |

## 与本机开发的差异

| | 本机 | 个人云 |
|--|------|--------|
| URL | `http://127.0.0.1:8080/` | `https://api.…/` |
| Bearer | 常可省略 | **应强制**（`web_api_require_bearer`） |
| UI | 包内（推荐） | **仅**包内 |
| CORS env | 通常不必（v0.2.0+） | 通常不必；额外浏览器 Origin 再扩白名单 |
