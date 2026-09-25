//! Victauri 版 Phase 3 E2E：通过 `eval_js` 注入 `window.fetch` 拦截器存根 SSE。
//!
//! ## 核心技术
//!
//! Victauri 在 webview 内覆盖 `window.fetch`，对 `POST /chat/stream` 返回 fixture SSE 正文：
//!
//! ```javascript
//! client.eval_js("window.__originalFetch = window.fetch;
//!   window.fetch = (url, opts) => {
//!     if (url.includes('/chat/stream') && opts.method==='POST')
//!       return Promise.resolve(new Response(sseBody, {status:200,headers:{'content-type':'text/event-stream'}}));
//!     return window.__originalFetch(url, opts);
//!   }");
//! ```
//!
//! 前置条件：
//!   1. Tauri 桌面应用 debug 模式运行
//!   2. `CM_E2E_FIXTURES=1` 启用 E2E fixture 路由
//!   3. `VICTAURI_E2E=1 cargo test --test victauri_sse_stub`

use victauri_test::e2e_test;

/// 播种默认会话并重新加载。
async fn seed_and_goto(client: &mut victauri_test::VictauriClient, session_id: &str) {
    let _ = client
        .eval_js("fetch('/user-data/prefs',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({locale:'zh',theme:'crabmate-light',side_panel_view:'hidden',side_width:280,editor_layout_mode:false,status_bar_visible:true})})")
        .await;

    let _ = client
        .eval_js(&format!(
            "fetch('/user-data/workspaces/current/sessions',{{method:'PUT',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{sessions:[{{id:'{session_id}',title:'E2E smoke',draft:'',messages:[],updated_at:1,pinned:false,starred:false}}],active_session_id:'{session_id}'}})}})"
        ))
        .await;

    let _ = client.eval_js("location.reload()").await;
    client
        .wait_for("network_idle", Some(""), Some(10000), Some(500))
        .await
        .ok();
}

// ---------------------------------------------------------------------------
// 测试 1：流错误存根 → 状态栏显示失败
// ---------------------------------------------------------------------------
e2e_test!(
    stream_error_shows_failure_in_status_bar,
    |client| async move {
        // Phase 3: 注入错误流存根
        let sse_body = concat!(
            "id: 1\ndata: {\"sse_capabilities\":{\"supported_sse_v\":1}}\n\n",
            "id: 2\ndata: {\"v\":1}\n\n",
            "id: 3\ndata: {\"error\":\"e2e intentional failure\",\"code\":\"E2E_STREAM_FAIL\"}\n\n",
            "id: 4\ndata: {\"stream_ended\":{\"reason\":\"error\"}}\n\n",
        );

        let _ = client
        .eval_js(&format!(
            "(()=>{{const body=`{sse_body}`;\
             window.__originalFetch=window.fetch;\
             window.fetch=(url,opts)=>{{if(typeof url==='string'&&url.includes('/chat/stream')&&opts&&opts.method==='POST')\
             return Promise.resolve(new Response(body,{{status:200,headers:{{'content-type':'text/event-stream'}}}}));\
             return window.__originalFetch(url,opts);}};}})()"
        ))
        .await;

        seed_and_goto(&mut client, "s_e2e_err").await;

        // 发送消息
        let _ = client
        .eval_js(
            "(()=>{const el=document.querySelector('[data-testid=\"chat-composer-input\"]');\
             if(!el)return;el.focus();\
             const s=Object.getOwnPropertyDescriptor(window.HTMLTextAreaElement.prototype,'value').set;\
             s.call(el,'e2e error test');el.dispatchEvent(new Event('input',{{bubbles:true}}));})()"
        )
        .await;

        client.press_key("Enter").await.unwrap();

        // 等待状态栏显示 fetch-error 样式
        client
            .wait_for(
                "selector",
                Some("[data-testid=\"status-bar\"].status-bar-fetch-error"),
                Some(15000),
                Some(200),
            )
            .await
            .unwrap();
    }
);
