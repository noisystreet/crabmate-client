# 桌面壳调试手册（无头环境拿证据）

真桌面（Tauri + WebKitGTK）上「点了没反应」这类症状，`make e2e-playwright` 与全部 Chromium 用例照样全绿。
这份手册记的是：**怎么在没有图形会话的情况下，把真壳里的证据（panic 原文、状态是否变化、元素是否被遮挡）抓出来**。
2026-11-22 查 #569「连两条 lane 后整屏不响应」就是靠它定位的两条根因（§5）。

## 0. 先立判据：三种证据强度

| 手段 | 能看到 | 漏掉什么 |
|---|---|---|
| Playwright + 假壳（`e2e/specs/mock-*.spec.ts`） | DOM / 网络 / console，秒级、可断言 | **引擎是 Chromium**：owner 生命周期、微任务时序、`display:contents` 等真 WebKit 行为都不在覆盖内 |
| 真壳 + xvfb + 自检探针（§2） | 真 WebKitGTK + 真 Tauri IPC + 真 serve | 要自己造观测通道；不能当常规回归 |
| 已安装的 `.deb`（从图标/gnome-shell 启动） | 最接近用户路径 | **日志读不到**：`/proc/<pid>/fd/2` 是 gnome-shell 的 socket，没有 devtools |

判断「要不要上 §2」的标准：**Chromium 复现不了、但用户在桌面上说不对**。本次两条根因都属这一类。

## 1. 一条硬结论先记住

**wasm32 上 panic 是 trap，不是 unwind。** 浏览器里报的是 `RuntimeError: Unreachable code should not be executed`（或 Playwright 的 `pageerror: unreachable`），
Rust 栈不展开、`Drop` 全跳过 —— 于是 `js-sys` 任务里的 `RefCell` 借用**永久不还**，该任务此后每次唤醒都再 panic，
`spawn_local` 那条链一起失效。

所以「静态页面还画着、点什么都没反应、CPU 却闲着」这个组合，几乎总是**反应式系统已死**，而不是死循环或渲染卡住。
先用 `/proc/<pid>/io` 的 `rchar` 增量 + `/proc/<pid>/stat` 的 utime/stime 3 秒采样确认「它没在干活」，再去找 panic。

## 2. 一条命令：`scripts/probe-desktop-shell.sh`

```bash
bash scripts/probe-desktop-shell.sh                          # 最小探针：panic + 存活 + lane 计数
bash scripts/probe-desktop-shell.sh --probe /tmp/p.js        # 换成自己的交互探针
bash scripts/probe-desktop-shell.sh --with-ctl --settle 60   # 左栏「可连实例」用真 ctl
```

它一条命令做完下面五件事（§2.1–§2.5 是它内部的顺序，手工重跑或改探针时按这个来）：

1. 起一台**独立** serve（自带已知 Bearer，不碰用户自己那几台的实例锁）；
2. 起本地收集端，**先探活**再往下走（收集端悄悄死掉是本次踩过的坑）；
3. 把探针内联进 `desktop-tauri/dist/index.html`，`touch build.rs` 重编壳（资产内嵌，不重编不生效）；
4. 在 xvfb 里启动壳，独立 `XDG_RUNTIME_DIR` / `XDG_DATA_HOME` / `XDG_CACHE_HOME`；
5. 观察窗口结束 → 打印 panic 条数与探针日志 → **还原 dist**（除非 `--no-restore`）。

环境变量与同名参数：`CM_PROBE_SETTLE_SECS`(45) / `CM_PROBE_SERVE_PORT`(18080) /
`CM_PROBE_COLLECT_PORT`(4177) / `CM_PROBE_BEARER`(probe-token)；`CM_DESKTOP_BACKEND_BIN` 指定 serve 二进制。
**退出码**：探针里出现 panic → 1，否则 0（可以当冒烟断言用）。日志路径每次都会打印。

输出长这样（`ALIVE` 每 10 s 一条，是「页面还活着、前台是哪条」的读数）：

```text
==================== 探针结果 ====================
panic（CONSOLE-ERR）: 0
-------------------------------------------------
ALIVE slots=0 activeLane=none composer=false t=0s
ALIVE slots=1 activeLane=0 composer=true t=10s
=================================================
```

改探针时只要保证：幂等（重复执行不重复点击）、只在实际业务 UI 上跑、用 `fetch(..., {mode:"no-cors"})` POST 到
`http://127.0.0.1:<收集端端口>/r`。

### 2.1 起一台独立 serve（带已知 bearer）

```bash
CM_SERVE_DISABLE_INSTANCE_LOCK=1 CM_WEB_API_BEARER_TOKEN=probe-token no_proxy=127.0.0.1,localhost \
  /usr/bin/crabmate serve --host 127.0.0.1 --port 18080
# 探活（注意这个 serve 要 Bearer，裸 curl 是 401）
curl -s -o /dev/null -w '%{http_code}\n' -H 'Authorization: Bearer probe-token' \
  http://127.0.0.1:18080/workspace/projects    # 期望 200
```

`CM_SERVE_DISABLE_INSTANCE_LOCK=1` 是为了不和用户自己那几台 serve 抢同一工作区的实例锁。
`CM_WEB_API_BEARER_TOKEN` 就设一个已知值 —— 壳侧也用同一个（§2.3 的 env），否则两边对不上、页面全是 401。

### 2.2 起一个收集端（页面把结果 POST 过来）

```python
# 临时脚本，放在 agent_space/（gitignored），跑完删
from http.server import BaseHTTPRequestHandler, ThreadingHTTPServer
LOG = "/tmp/cm-probe.log"

class H(BaseHTTPRequestHandler):
    def do_POST(self):
        n = int(self.headers.get("Content-Length") or 0)
        body = self.rfile.read(n).decode("utf-8", "replace") if n else ""
        open(LOG, "a", encoding="utf-8").write(body.replace("\n", " | ") + "\n")
        self.send_response(204); self.send_header("Access-Control-Allow-Origin", "*"); self.end_headers()
    def log_message(self, *a): pass

ThreadingHTTPServer(("127.0.0.1", 4177), H).serve_forever()
```

页面侧用 `fetch(url, {method:"POST", mode:"no-cors", body})` 发（`no-cors` 不会触发预检，也不需要读响应）。
**开跑前一定先 `curl -X POST --data ping http://127.0.0.1:4177/ping` 探活** —— 收集端被后台任务回收时不会报错，
症状是「几轮下来 probe 日志一直是空的」（本次踩过，白跑两轮）。

### 2.3 注入探针（脚本走 dist + 重编；手工更省事的是壳侧 `initialization_script`）

壳侧本来就有 `initialization_script`（注入 `__CM_CONN_KEY__` 的那条），临时再加一条：

```rust
// desktop-tauri/src-tauri/src/desktop_main_window.rs（**临时**，验完立刻删）
.initialization_script(connection_scope_init_script(label))
.initialization_script(
    std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../agent_space/probe.js"))
        .unwrap_or_default(),
);
```

好处：不改打包资产、不依赖内嵌刷新、真机与 xvfb 都能用；坏处：要重编壳（`cargo build --bin crabmate-desktop`，约 7 s）。

### 2.4 在 xvfb 里启动，并绕开单实例插件

```bash
xvfb-run --auto-servernum --server-args='-screen 0 1280x900x24' \
  env -u WAYLAND_DISPLAY -u WAYLAND_SOCKET -u GDK_BACKEND \
      WINIT_UNIX_BACKEND=x11 LIBGL_ALWAYS_SOFTWARE=1 \
      XDG_RUNTIME_DIR=/tmp/cm-rt XDG_DATA_HOME=/tmp/cm-data XDG_CACHE_HOME=/tmp/cm-cache \
      CM_DESKTOP_SKIP_CONNECT=1 CM_DESKTOP_SERVE_URL=http://127.0.0.1:18080/ \
      CM_WEB_API_BEARER_TOKEN=probe-token \
      ./desktop-tauri/src-tauri/target/debug/crabmate-desktop
```

- `CM_DESKTOP_SKIP_CONNECT=1` + `CM_DESKTOP_SERVE_URL=...` = 跳过连接页，直接进业务 UI 并指向这台 serve。
- **必须给独立的 `XDG_RUNTIME_DIR`**：否则 `tauri-plugin-single-instance` 会让第二个进程**静默 exit 0**（无任何输出）。
- 想要左栏「可连实例」是真的，就把 ctl socket 链进去：`ln -sfn /run/user/1000/crabmate /tmp/cm-rt/crabmate`。
- `XDG_DATA_HOME` / `XDG_CACHE_HOME` **每轮换新目录**（§3.4）。

### 2.5 读结果

```bash
grep -c CONSOLE-ERR /tmp/cm-probe.log      # panic 条数：修复前后对比就是主判据
grep -e BOOT -e AFTER -e SWITCHED /tmp/cm-probe.log
```

## 3. 六个坑（都是本次实踩）

### 3.1 第二个进程 exit 0、什么都不打印

单实例插件按 app id 查重，直接在 `main` 里退出。**不是**崩，也不写日志。
→ 独立 `XDG_RUNTIME_DIR`（§2.4）。不要为此杀掉用户正在跑的那个实例。

### 3.2 改了 `desktop-tauri/dist` 却不生效

debug 壳把资产**内嵌**进二进制（`grep` 二进制搜不到明文是因为压缩存放，别据此判断「没内嵌」）。
改完 dist 必须 `touch desktop-tauri/src-tauri/build.rs` 再 `cargo build`，否则 embed 不刷新。
→ 要么按这个顺序做（`scripts/probe-desktop-shell.sh` 就是这么干的），要么手工时干脆走壳侧
`initialization_script`（§2.3）—— 那条不碰资产，改完直接重编即可。

### 3.3 `SyntaxError: Unexpected token '<'`

把**旧的** `index.html` 当注入基线：`make frontend` 会换掉 asset 哈希，旧 index 引用的 `crabmate-web-*.js` 已不存在，
拿到的是 404 页面（HTML）当 JS。→ 注入基线永远取**当前** `frontend/dist/index.html`。

### 3.4 明明换了代码，行为还是上一轮的

WebKit 的数据目录缓存了旧 `index.html`。→ 每轮 `rm -rf` 并给新的 `XDG_DATA_HOME` / `XDG_CACHE_HOME`。

### 3.5 探针双跑，读数全是 `false`

`initialization_script` 在**每次导航**时都执行；如果同时在 `dist/index.html` 里也留了一份探针，
两份会互相把菜单「开了又关」，于是 `dd=false`（看起来像 bug，其实是探针自己）。
→ 探针要有幂等 + 路径守卫（只在实际业务 UI 上跑一次），并确认 dist 里没残留探针。

### 3.6 把「元素存在」当「用户看得见」

`document.querySelector` 找得到 ≠ 在屏幕上可点。本次菜单的判定改成三件事一起看：
`getBoundingClientRect()`（位置尺寸）+ `getComputedStyle` 的 `display/visibility/z-index` +
`document.elementFromPoint(中心)` 是否落在菜单自己身上（判被遮挡）。

## 4. 探针该测什么

- **panic 计数**：hook `console.error`（`console_error_panic_hook` 走这里）+ `window.onerror` + `unhandledrejection`。
- **状态，不截图**：`data-lane`（前台是哪条）、元素存在与否、`isVisible`、几何、`elementFromPoint`。
- **一次点击 → 一个可断言的布尔**（例如「菜单里那一项出现了吗」），比「看起来对不对」可靠。
- 探针里的等待用轮询 + 上限，不要用固定 `sleep` 赌时序。

## 5. 本次两个真实根因（模板）

两条都属同一类错误：**长寿命的反应式值建在了短寿命的 owner 里**。`reactive_graph` 的两条事实决定了这个坑：

- `Owner::new()`：**注册为「当前 owner」的子节点**（不是 root）；
- `Effect` / `Memo` 每次 run 都 `with_cleanup` —— 本次 run 里建出来的节点，下一轮 / dispose 时被清掉。

| 案例 | 建错的地方 | 症状 | 修法 |
|---|---|---|---|
| A：应用级单例（连接名单 / 已登记标记 / 可连实例） | 首次读取发生在 `connection_report` 的 `Memo` 里 → 建在 memo 的 computation 作用域 | 启动即 panic，`already disposed`；上报链全死 | `bootstrap_app_shell` 里建在 `App` 的 owner 上（`init_app_signal_singletons`） |
| B：新 lane 的整棵子树 | `mount_to` 的 owner 挂到「点『连接』那颗按钮所在的 DOM 分支」下 | 建第二条 lane 那一刻起成串 panic；切 lane 不动、点「项目」不弹、点会话没反应 | 抓下 `App` 的 owner，挂载跑在 `owner.with(...)` 里 |

结论一句话：**凡是「全局 / 与页面同寿」的反应式值，必须在启动期、稳定的 owner 下创建**；
凡是「从事件回调里新建一棵子树」，显式指定它挂在哪一层 owner 下。

## 6. 边界

- 这套东西**不替代** Playwright 回归。反过来：Chromium 能复现的，就别上真壳。
- 探针代码与临时 `initialization_script` **不要提交**；验完 `bash desktop-tauri/scripts/prepare-sidecar.sh` 还原 dist。
- `CM_DESKTOP_SKIP_CONNECT` 是调试/E2E 入口，不要让它出现在产品路径上。
- 用户正在跑的那个实例不要杀；用独立运行时目录并行跑你自己的。
