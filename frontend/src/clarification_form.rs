//! Web 澄清问卷：直接复用共享层的待填状态（`crabmate_client_api::clarify_form`）。
//!
//! 这里只做改名转发。两侧字段与 `from_sse` 本来一字不差，各存一份的代价是下次契约
//! 加字段（例如题型将来带上候选列表）要记得改两处 —— 漏一处就是「Web 收不到新列」。
//! 必填校验与序列化早已共用同一份实现，结构却各写一遍只会让这种漂移更难发现。

pub use crabmate_client_api::clarify_form::PendingClarification as PendingClarificationForm;
