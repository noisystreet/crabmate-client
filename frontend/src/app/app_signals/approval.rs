//! 审批条、澄清问卷与思维迹日志。

use leptos::prelude::*;

use crate::clarification_form::PendingClarificationForm;
use crate::sse_dispatch::ThinkingTraceInfo;

#[derive(Clone, Copy)]
pub struct ApprovalSignals {
    pub pending_approval: RwSignal<Option<(String, String, String)>>,
    pub approval_expanded: RwSignal<bool>,
    pub last_approval_sid: RwSignal<String>,
    pub pending_clarification: RwSignal<Option<PendingClarificationForm>>,
    pub thinking_trace_log: RwSignal<Vec<ThinkingTraceInfo>>,
    /// 审批决定提交中：提交期间按钮禁用，避免连击重复请求。
    pub approval_busy: RwSignal<bool>,
    /// 审批决定提交失败（弹窗保留，允许重试）。
    pub approval_error: RwSignal<Option<String>>,
}

impl ApprovalSignals {
    pub fn new() -> Self {
        Self {
            pending_approval: RwSignal::new(None),
            approval_expanded: RwSignal::new(false),
            last_approval_sid: RwSignal::new(String::new()),
            pending_clarification: RwSignal::new(None),
            thinking_trace_log: RwSignal::new(Vec::new()),
            approval_busy: RwSignal::new(false),
            approval_error: RwSignal::new(None),
        }
    }
}

impl Default for ApprovalSignals {
    fn default() -> Self {
        Self::new()
    }
}
