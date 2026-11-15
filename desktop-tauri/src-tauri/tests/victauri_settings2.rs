//! Victauri 版设置页**壳专属**测试：模型密钥 / Web Bearer 的落盘位置（#152）。
//!
//! 这两个用例用 `window.__TAURI__.core.invoke('get_llm_secret')` /
//! `invoke('get_connect_bearer')` 直读设备钥匙串，验证「密钥不进 localStorage」——
//! 浏览器无钥匙串，无等价物，故**留在 Victauri**。
//!
//! 原先的第三个用例 `import_mcp_json_adds_server_rows` 已迁到
//! `e2e/specs/mock-settings-page.spec.ts`（它只是页面行为）。

use victauri_test::e2e_test;
use victauri_test::locator::Locator;

async fn seed_settings_session(client: &mut victauri_test::VictauriClient, sid: &str) {
    let _ = client.eval_js("fetch('/user-data/prefs',{method:'PUT',headers:{'Content-Type':'application/json'},body:JSON.stringify({locale:'zh',theme:'crabmate-light',side_panel_view:'hidden',side_width:280,editor_layout_mode:false,status_bar_visible:true})})").await;
    let _ = client.eval_js(&format!("fetch('/user-data/workspaces/current/sessions',{{method:'PUT',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{sessions:[{{id:'{sid}',title:'E2E',draft:'',messages:[],updated_at:1,pinned:false,starred:false}}],active_session_id:'{sid}'}})}})")).await;
    let _ = client.eval_js("location.reload()").await;
    client
        .wait_for("network_idle", Some(""), Some(10000), Some(500))
        .await
        .ok();
}

async fn open_settings(client: &mut victauri_test::VictauriClient, section: &str) {
    Locator::test_id("settings-open")
        .click(client)
        .await
        .unwrap();
    client
        .wait_for(
            "selector",
            Some("[data-testid=\"settings-page\"].settings-page-visible"),
            Some(10000),
            Some(500),
        )
        .await
        .unwrap();
    Locator::test_id(&format!("settings-nav-{section}"))
        .click(client)
        .await
        .unwrap();
}

async fn close_settings(client: &mut victauri_test::VictauriClient) {
    Locator::test_id("settings-back")
        .click(client)
        .await
        .unwrap();
    client
        .wait_for(
            "selector_gone",
            Some("[data-testid=\"settings-page\"].settings-page-visible"),
            Some(10000),
            Some(200),
        )
        .await
        .unwrap();
}

e2e_test!(model_and_api_key_save, |client| async move {
    seed_settings_session(&mut client, "s_e2e_llm").await;
    open_settings(&mut client, "llm").await;
    Locator::test_id("settings-models-add")
        .click(&mut client)
        .await
        .unwrap();
    let _ = client.eval_js("(()=>{const set=(tid,val)=>{const el=document.querySelector('[data-testid=\"'+tid+'\"]');if(!el)return;const s=Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value').set;s.call(el,val);el.dispatchEvent(new Event('input',{bubbles:true}));};set('settings-models-new-label','E2E');set('settings-models-new-base','https://api.example.com/v1');set('settings-models-new-model','e2e-test-model');set('settings-models-new-key','E2E_STUB_KEY');})()").await;
    Locator::test_id("settings-models-add-submit")
        .click(&mut client)
        .await
        .unwrap();
    Locator::test_id("settings-save-all")
        .click(&mut client)
        .await
        .unwrap();
    client
        .wait_for("text", Some("已保存"), Some(10000), Some(200))
        .await
        .ok();
    close_settings(&mut client).await;
    let _ = client
        .eval_js(
            r#"(async()=>{window.__e2eLlmKeyOk=false;for(let i=0;i<40;i++){try{const inv=window.__TAURI__&&window.__TAURI__.core&&window.__TAURI__.core.invoke;if(typeof inv!=='function')return;const v=await inv('get_llm_secret',{slot:'client_llm'});if(v==='E2E_STUB_KEY'){window.__e2eLlmKeyOk=true;return;}}catch(e){}await new Promise(r=>setTimeout(r,100));}})()"#,
        )
        .await;
    let mut secret_ok = false;
    for _ in 0..50 {
        secret_ok = client
            .eval_js("window.__e2eLlmKeyOk===true")
            .await
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if secret_ok {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let ls_empty: bool = client
        .eval_js("!localStorage.getItem('crabmate-client-llm-api-key')")
        .await
        .unwrap()
        .as_bool()
        .unwrap_or(false);
    assert!(secret_ok, "expected client_llm in device keyring");
    assert!(ls_empty, "must not persist model API key in localStorage");
    open_settings(&mut client, "llm").await;
    // Dialog closed after submit; assert via saved list label.
    let listed: bool = client
        .eval_js("!!Array.from(document.querySelectorAll('.settings-saved-models-label')).find(el=>el.textContent==='E2E')")
        .await
        .unwrap()
        .as_bool()
        .unwrap_or(false);
    assert!(listed, "expected saved model row labeled E2E");
});

e2e_test!(web_api_bearer_not_in_local_storage, |client| async move {
    seed_settings_session(&mut client, "s_e2e_web_bearer").await;
    open_settings(&mut client, "connection").await;
    let _ = client
        .eval_js(
            r#"(()=>{const el=document.querySelector('[data-testid="settings-web-api-bearer-input"]');if(!el)return;const s=Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype,'value').set;s.call(el,'E2E_WEB_BEARER');el.dispatchEvent(new Event('input',{bubbles:true}));})()"#,
        )
        .await;
    Locator::test_id("settings-web-api-bearer-save")
        .click(&mut client)
        .await
        .unwrap();
    client
        .wait_for("text", Some("已保存"), Some(10000), Some(200))
        .await
        .ok();
    let _ = client
        .eval_js(
            r#"(async()=>{window.__e2eBearerOk=false;for(let i=0;i<40;i++){try{const inv=window.__TAURI__&&window.__TAURI__.core&&window.__TAURI__.core.invoke;if(typeof inv!=='function')return;const v=await inv('get_connect_bearer');if(v==='E2E_WEB_BEARER'){window.__e2eBearerOk=true;return;}}catch(e){}await new Promise(r=>setTimeout(r,100));}})()"#,
        )
        .await;
    let mut secret_ok = false;
    for _ in 0..50 {
        secret_ok = client
            .eval_js("window.__e2eBearerOk===true")
            .await
            .ok()
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        if secret_ok {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let ls_empty: bool = client
        .eval_js("!localStorage.getItem('crabmate-api-bearer-token')")
        .await
        .unwrap()
        .as_bool()
        .unwrap_or(false);
    assert!(secret_ok, "expected connect bearer in device keyring");
    assert!(ls_empty, "must not persist Web API Bearer in localStorage");
});
