---
name: Bug 报告 / Bug report
about: 描述缺陷与复现方式 / Describe the defect and how to reproduce it
title: ""
labels: ["bug"]
---

<!-- 标题格式：<type>(<optional scope>): <中文简述> / <English brief>，如
     fix(mobile): 键盘弹出后 composer 被抬高两次 / composer lifted twice when IME shows
     提交前请删除本注释。 -->

## 环境 / Environment

- 端 / Surface：<!-- Desktop Linux / Android / Web 浏览器 / TUI（crabmate-tui）；多端请分别注明 -->
- Client 版本 / 来源：<!-- `v0.5.x`，或 `git rev-parse --short HEAD` + 分支 -->
- 连接方式：<!-- 回环 / LAN / 个人云 HTTPS；是否启用 Web Bearer -->
- `crabmate serve` 版本：<!-- 远端 serve 的版本或契约 pin（`docs/design/contract_pin.md`） -->
- OS / 版本：
- 其他相关环境：<!-- 浏览器 + 版本 / WebKitGTK 版本 / Android API level + 机型 / 终端类型与列宽 -->

## 复现步骤 / Steps to reproduce

1.
2.

## 期望行为 / Expected

<!-- 无行为变化写「无 / none」不适用此节，写清你认为正确的行为。 -->

## 实际行为 / Actual

<!-- 区分「提示错误」与「静默失败」；涉及数据的状态（会话、工作区、偏好）请说明是否丢失。 -->

## 日志或截图 / Logs or screenshots

<!-- 浏览器 / 壳的 devtools 控制台、`RUST_LOG` 输出（TUI / connect / web-host）、录屏或截图。
     请勿粘贴 API_KEY、`CM_WEB_API_BEARER_TOKEN`、GitHub token 等敏感信息。 -->

## 可能相关模块（可选）/ Suspected modules (optional)

<!-- 如 frontend（chat / IDE / 设置 / 主题）、desktop-tauri、mobile-tauri、crabmate-connect、crabmate-tui、web-host、e2e。

     若问题在 serve 侧（HTTP / SSE / 错误码 / 工作区语义），请改在 Server 仓提：本仓只维护壳与 UI，
     契约级问题以 Server 仓为准。 / If the defect is in `crabmate serve` (HTTP / SSE / error codes /
     workspace semantics), please file it in the Server repo instead. -->

## 是否可稳定复现 / Reproducible

- [ ] 每次
- [ ] 偶发（约 __ 次中 1 次）
- [ ] 仅一次，未能复现
