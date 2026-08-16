//! Victauri 集成测试共用步骤。

use victauri_test::VictauriClient;
use victauri_test::locator::Locator;

/// 冷启动不再恢复 `active_session_id`；点开会话轨上的指定会话。
pub async fn open_session_in_rail(client: &mut VictauriClient, session_id: &str) {
    Locator::test_id("chat-composer-input")
        .expect(client)
        .to_be_visible()
        .await
        .ok();
    Locator::test_id(&format!("nav-session-{session_id}"))
        .click(client)
        .await
        .ok();
}
