//! TuiApp 的设置面板接线（`settings_panel.rs` 的子模块，压住面板文件行数门禁）：
//! 开/关面板、面板按键分发、每帧内容构建、保存请求下发与保存结果回写。

use crossterm::event::KeyEvent;
use ratatui::style::Color;

use super::super::TuiApp;
use super::super::settings::{LlmSave, PersistedSettings, PrefsSave};
use super::super::state::LineKind;
use super::super::worker::WorkerJob;
use super::field_label;
use super::{FieldId, PanelContent, PanelCtx, PanelEffect, SaveGroup, SettingsPanel};

impl TuiApp<'_> {
    /// 打开设置面板（`/settings` / F2）。回合进行中打开为只读并提示；
    /// 持久层快照尚未拉到（如启动时拉取失败）时借机重试一次。
    pub(crate) fn open_settings(&mut self) {
        let read_only = self.st.running;
        self.panel = Some(SettingsPanel::new(read_only));
        if self.persisted.is_none() {
            self.load_user_settings();
        }
        if read_only {
            self.st.push_line(
                LineKind::System,
                "回合进行中：设置面板为只读（可浏览；结束回合后自动解锁）",
            );
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
            PanelEffect::Save { llm, prefs } => self.submit_settings_save(llm, prefs),
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

    /// 保存请求：staged 分组 → worker 任务（先 GET 再合并改自己管理的键再全量 PUT）。
    /// worker 已退出导致发送失败时复位在途标记，面板保留改动可重试，避免卡在"保存中"。
    fn submit_settings_save(&mut self, llm: LlmSave, prefs: PrefsSave) {
        let llm_ok = !llm.any() || self.job_tx.send(WorkerJob::SaveLlm(llm)).is_ok();
        let prefs_ok = !prefs.any() || self.job_tx.send(WorkerJob::SavePrefs(prefs)).is_ok();
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
        self.st
            .push_line(LineKind::System, "正在保存设置到 serve user-data…");
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
