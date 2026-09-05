//! TuiApp 的设置面板接线（`settings_panel.rs` 的子模块，压住面板文件行数门禁）：
//! 开/关面板、面板按键分发、每帧内容构建、保存请求下发与保存结果回写。

use crossterm::event::KeyEvent;
use ratatui::style::Color;

use crabmate_client_api::secrets::{KEYRING_SERVICE, SecretSlot};

use super::super::TuiApp;
use super::super::settings::{LlmSave, PersistedSettings, PrefsSave};
use super::super::state::LineKind;
use super::super::worker::WorkerJob;
use super::field_label;
use super::{FieldAction, FieldId, PanelContent, PanelCtx, PanelEffect, SaveGroup, SettingsPanel};

impl TuiApp<'_> {
    /// 打开设置面板（`/settings` / F2）。回合进行中打开为只读并提示；
    /// 持久层快照尚未拉到（如启动时拉取失败）时借机重试一次。
    pub(crate) fn open_settings(&mut self) {
        let read_only = self.st.running;
        let mut panel = SettingsPanel::new(read_only);
        // API 密钥"已设"判定：本进程 override（CLI/env）非空，或钥匙串该槽有值。
        let override_set = self
            .overrides
            .api_key
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        let key_set = override_set
            || (!self.no_keyring && keyring_slot_has(SecretSlot::ClientLlm.keyring_account()));
        panel.set_secret_set(key_set);
        self.panel = Some(panel);
        if self.persisted.is_none() {
            self.load_user_settings();
        }
        if read_only {
            self.st
                .push_line(LineKind::System, "回合进行中：设置面板只读，结束后自动解锁");
        }
    }

    /// 启动时 user-data 快照结果：成功更新持久层；失败仅当设置面板打开时提示
    /// （启动静默拉取失败不打扰；面板打开会重试加载）。
    pub(crate) fn on_user_data(&mut self, result: Result<PersistedSettings, String>) {
        match result {
            Ok(persisted) => self.persisted = Some(persisted),
            Err(e) => {
                if self.panel.is_some() {
                    self.st.push_line(
                        LineKind::System,
                        &format!("拉取 user-data 设置失败：{e}（回退 override 与 serve 默认）"),
                    );
                }
            }
        }
    }

    /// 面板打开期间的全部分发（在 approval / Ctrl+C 之后调用）。
    pub(crate) fn on_settings_key(&mut self, key: KeyEvent) {
        let effect = {
            let Some(panel) = self.panel.as_mut() else {
                return;
            };
            let ctx = PanelCtx {
                overrides: self.overrides,
                persisted: self.persisted.as_ref(),
                serve_defaults: self.st.serve_defaults.as_ref(),
            };
            panel.handle_key(&key, &ctx)
        };
        match effect {
            PanelEffect::None => {}
            PanelEffect::Close => {
                self.panel = None;
            }
            PanelEffect::Save { llm, prefs, secret } => {
                self.submit_settings_save(llm, prefs, secret)
            }
        }
    }

    /// 面板当前帧内容（供 render 画全屏浮层）；面板未打开返回 `None`。
    pub(crate) fn settings_content(&self, width: usize) -> Option<PanelContent> {
        let panel = self.panel.as_ref()?;
        let ctx = PanelCtx {
            overrides: self.overrides,
            persisted: self.persisted.as_ref(),
            serve_defaults: self.st.serve_defaults.as_ref(),
        };
        Some(panel.content(&ctx, width))
    }

    /// 保存请求：先同步落地 API 密钥（本机钥匙串，不进 serve user-data），再按 staged
    /// 分组下发 worker 任务（先 GET 再合并改自己管理的键再全量 PUT）。密钥环节失败
    /// （`--no-keyring` 或钥匙串不可用）时整单取消并复位在途标记，staged 保留可重试。
    fn submit_settings_save(&mut self, llm: LlmSave, prefs: PrefsSave, secret: FieldAction) {
        if !secret.is_write() {
            self.dispatch_user_data_saves(llm, prefs);
            return;
        }
        if self.no_keyring {
            self.cancel_save_for_secret(
                "「--no-keyring」下不能写钥匙串：API 密钥保存已取消（其余改动保留，可按 S 重试）",
            );
            return;
        }
        let value = match &secret {
            FieldAction::Write(v) => v.clone(),
            FieldAction::Skip => None,
        };
        let account = SecretSlot::ClientLlm.keyring_account();
        match keyring_write(account, value.as_deref()) {
            Ok(()) => {
                self.on_secret_written(value.map(|s| s.trim().to_string()));
                self.dispatch_user_data_saves(llm, prefs);
            }
            Err(e) => {
                self.cancel_save_for_secret(&format!(
                    "API 密钥保存失败：{e}（未写入钥匙串；改动保留，可按 S 重试）"
                ));
            }
        }
    }

    /// API 密钥钥匙串写成功：同步 in-process override（写 = 新值，清 = None）、
    /// 清面板 staged 并更新「已设」显示。
    fn on_secret_written(&mut self, value: Option<String>) {
        let now_set = value
            .as_deref()
            .map(str::trim)
            .is_some_and(|s| !s.is_empty());
        self.overrides.api_key = if now_set { value } else { None };
        if let Some(panel) = self.panel.as_mut() {
            panel.secret_saved(now_set);
        }
        if now_set {
            self.st.push_line(
                LineKind::System,
                "模型 API 密钥已写入本机钥匙串（随对话发送；serve 无服务端 API_KEY 时生效）",
            );
        } else {
            self.st.push_line(
                LineKind::System,
                "模型 API 密钥已清除（未设时将跟随 serve）",
            );
        }
    }

    /// 密钥环节失败：不发 user-data 任务，复位在途标记（staged 保留供重试）。
    fn cancel_save_for_secret(&mut self, msg: &str) {
        self.st.push_line(LineKind::System, msg);
        if let Some(panel) = self.panel.as_mut() {
            panel.save_group_result(SaveGroup::Llm, false);
            panel.save_group_result(SaveGroup::Prefs, false);
            panel.set_note(
                "保存取消：改动保留，可按 S 重试".to_string(),
                Color::LightRed,
            );
        }
    }

    /// 下发 llm/prefs 两组保存任务（密钥已先行落地）。worker 已退出导致发送失败时复位
    /// 在途标记，面板保留改动可重试，避免卡在"保存中"。
    fn dispatch_user_data_saves(&mut self, llm: LlmSave, prefs: PrefsSave) {
        let llm_pending = llm.any();
        let prefs_pending = prefs.any();
        let llm_ok = !llm_pending || self.job_tx.send(WorkerJob::SaveLlm(llm)).is_ok();
        let prefs_ok = !prefs_pending || self.job_tx.send(WorkerJob::SavePrefs(prefs)).is_ok();
        if !llm_ok || !prefs_ok {
            self.st.push_line(
                LineKind::System,
                "设置保存失败：worker 已退出（改动保留，可重试）",
            );
            if let Some(panel) = self.panel.as_mut() {
                if !llm_ok {
                    panel.save_group_result(SaveGroup::Llm, false);
                }
                if !prefs_ok {
                    panel.save_group_result(SaveGroup::Prefs, false);
                }
            }
            return;
        }
        if llm_pending || prefs_pending {
            self.st
                .push_line(LineKind::System, "正在保存设置到 serve user-data…");
        }
    }

    /// llm-overrides 保存结果：成功 → 内存持久层同样更新 + 清本地 override + 面板落地。
    pub(crate) fn on_settings_saved_llm(&mut self, result: Result<LlmSave, String>) {
        match result {
            Ok(save) => {
                self.persisted
                    .get_or_insert_with(PersistedSettings::default)
                    .apply_llm_saved(&save);
                if save.model.is_write() {
                    self.overrides.model = None;
                }
                if save.api_base.is_write() {
                    self.overrides.api_base = None;
                }
                let names = llm_saved_names(&save);
                self.st.push_line(
                    LineKind::System,
                    &format!(
                        "设置已保存：{}（serve user-data；本进程 override 已清除）",
                        names.join("、")
                    ),
                );
                if let Some(panel) = self.panel.as_mut() {
                    panel.save_group_result(SaveGroup::Llm, true);
                    panel.set_note("已保存到 serve user-data".to_string(), Color::LightGreen);
                }
            }
            Err(e) => {
                self.st.push_line(
                    LineKind::System,
                    &format!("保存模型设置失败：{e}（面板保留改动，可按 S 重试）"),
                );
                if let Some(panel) = self.panel.as_mut() {
                    panel.save_group_result(SaveGroup::Llm, false);
                    panel.set_note("保存失败：改动保留，按 S 重试".to_string(), Color::LightRed);
                }
            }
        }
    }

    /// prefs 保存结果（同 llm 侧语义）。
    pub(crate) fn on_settings_saved_prefs(&mut self, result: Result<PrefsSave, String>) {
        match result {
            Ok(save) => {
                self.persisted
                    .get_or_insert_with(PersistedSettings::default)
                    .apply_prefs_saved(&save);
                if save.role.is_write() {
                    self.overrides.agent_role = None;
                }
                if save.session_mode.is_write() {
                    self.overrides.session_mode = None;
                }
                let names = prefs_saved_names(&save);
                self.st.push_line(
                    LineKind::System,
                    &format!(
                        "设置已保存：{}（serve user-data；本进程 override 已清除）",
                        names.join("、")
                    ),
                );
                if let Some(panel) = self.panel.as_mut() {
                    panel.save_group_result(SaveGroup::Prefs, true);
                    panel.set_note("已保存到 serve user-data".to_string(), Color::LightGreen);
                }
            }
            Err(e) => {
                self.st.push_line(
                    LineKind::System,
                    &format!("保存会话设置失败：{e}（面板保留改动，可按 S 重试）"),
                );
                if let Some(panel) = self.panel.as_mut() {
                    panel.save_group_result(SaveGroup::Prefs, false);
                    panel.set_note("保存失败：改动保留，按 S 重试".to_string(), Color::LightRed);
                }
            }
        }
    }
}

fn llm_saved_names(save: &LlmSave) -> Vec<&'static str> {
    let mut v = Vec::new();
    if save.model.is_write() {
        v.push(field_label(FieldId::Model));
    }
    if save.api_base.is_write() {
        v.push(field_label(FieldId::ApiBase));
    }
    if save.temperature.is_write() {
        v.push(field_label(FieldId::Temperature));
    }
    if save.thinking.is_write() {
        v.push(field_label(FieldId::ThinkingMode));
    }
    v
}

fn prefs_saved_names(save: &PrefsSave) -> Vec<&'static str> {
    let mut v = Vec::new();
    if save.role.is_write() {
        v.push(field_label(FieldId::Role));
    }
    if save.session_mode.is_write() {
        v.push(field_label(FieldId::SessionMode));
    }
    v
}

/// 钥匙串槽位是否已有可读非空值（无默认 store / 无条目 / 空白都算"未设"）。
fn keyring_slot_has(account: &str) -> bool {
    keyring_read(account).is_some()
}

/// 读钥匙串条目（service 与桌面壳一致：`com.crabmate.credentials`）；不可用/空返回 `None`。
fn keyring_read(account: &str) -> Option<String> {
    let entry = keyring::Entry::new(KEYRING_SERVICE, account).ok()?;
    let raw = entry.get_password().ok()?;
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// 写 / 清钥匙串条目：`Some(v)` 写密码；`None` 删除（条目本就不存在视为成功清除）。
/// `Err`（钥匙串不可用等）交给调用方提示并保留 staged 供重试。
fn keyring_write(account: &str, value: Option<&str>) -> Result<(), String> {
    let entry =
        keyring::Entry::new(KEYRING_SERVICE, account).map_err(|e| format!("钥匙串不可用：{e}"))?;
    match value {
        Some(v) => entry.set_password(v).map_err(|e| format!("写入失败：{e}")),
        None => match entry.delete_credential() {
            Ok(()) => Ok(()),
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("删除失败：{e}")),
        },
    }
}
