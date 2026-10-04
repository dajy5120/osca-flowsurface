//! 【zh】 聚合（aggregation）模块的入口：声明 `ticks` 与 `time` 两个子模块，
//! 【zh】 并定义按成交笔数（tick）聚合时使用的 `TickCount` 类型。
//! 【zh】 从子模块的命名看，推测分别对应按笔数与按时间的聚合方式。
pub mod ticks;
pub mod time;

use serde::{Deserialize, Serialize};

// 【zh】 按笔数聚合时，每根 K 线所包含的成交笔数，内部是一个 `u16`。
// 【zh】 derive 了 Copy、克隆（Clone）、序列化（Serialize）与反序列化（Deserialize），
// 【zh】 因此可以按值传递，也能写入配置并还原。
// 【zh】 `ALL` 给出 10 到 1000 的预设档位；不在其中的值由 `is_custom` 判定为自定义值。
// 【zh】 `u16` 的上限为 65535，自定义值也不能超过这个范围。
// 【zh】 `Display` 输出形如 `100T` 的文本，用作界面标签。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TickCount(pub u16);

impl TickCount {
    pub const ALL: [TickCount; 10] = [
        TickCount(10),
        TickCount(20),
        TickCount(50),
        TickCount(100),
        TickCount(200),
        TickCount(500),
        TickCount(1000),
        TickCount(2000),
        TickCount(5000),
        TickCount(10000),
    ];

    pub fn is_custom(&self) -> bool {
        !Self::ALL.contains(self)
    }
}

impl std::fmt::Display for TickCount {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        write!(f, "{}T", self.0)
    }
}
