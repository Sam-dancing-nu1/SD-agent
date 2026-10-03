//! 运行期资源分配 schema + 非差异型副作用登记表（硬约束 20：
//! 工作区只隔离受版本控制的文件；端口 / DB 实例 / 环境变量 / 依赖锁
//! 必须显式分配，非差异型副作用单独登记）。
//!
//! P0 口径：分配表为纯数据登记位（结构就位、无消费方），副作用登记表
//! 真实记录工具执行产生的非文件类副作用（如 bash 起了子进程）。

use std::sync::Mutex;

/// 一次非差异型副作用登记。
#[derive(Debug, Clone)]
pub struct SideEffectRecord {
    pub kind: String,
    pub detail: String,
    pub ts_unix_ms: u64,
}

/// 运行期资源分配 schema（P0 空表 + 登记点；P1 起接端口/DB/依赖锁）。
#[derive(Debug, Default)]
pub struct ResourceAllocation {
    /// 已分配端口（P0 恒空）。
    pub ports: Vec<u16>,
    /// 已登记数据库实例名（P0 恒空）。
    pub db_instances: Vec<String>,
    /// 显式声明的运行期环境变量名（只记名，不记值——值可能是凭据）。
    pub env_var_names: Vec<String>,
}

/// 非差异型副作用登记表（唯一登记入口，工具执行路径必须留痕）。
#[derive(Debug, Default)]
pub struct ResourceLedger {
    effects: Mutex<Vec<SideEffectRecord>>,
    pub allocation: ResourceAllocation,
}

impl ResourceLedger {
    pub fn new() -> Self {
        Self::default()
    }

    /// 登记一条副作用（追加式，硬约束 2：更新只许追加）。
    /// 锁中毒容错：持锁方 panic 后仍取回数据继续（登记表不因旁路 panic 报废）。
    pub fn register(&self, kind: &str, detail: &str, ts_unix_ms: u64) {
        let mut effects = self.effects.lock().unwrap_or_else(|p| p.into_inner());
        effects.push(SideEffectRecord {
            kind: kind.to_string(),
            detail: detail.to_string(),
            ts_unix_ms,
        });
    }

    /// 快照（诊断/审计用）。
    pub fn snapshot(&self) -> Vec<SideEffectRecord> {
        self.effects
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .clone()
    }

    pub fn len(&self) -> usize {
        self.effects.lock().unwrap_or_else(|p| p.into_inner()).len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_appends() {
        let ledger = ResourceLedger::new();
        ledger.register("subprocess", "bash probe", 1);
        ledger.register("subprocess", "bash probe 2", 2);
        assert_eq!(ledger.len(), 2);
    }
}
