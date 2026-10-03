//! Secret newtype：凭据单点防泄密（project-structure.md 决策 6）。
//!
//! 纪律：凭据只走环境变量 SD_AGENT_API_KEY，禁落盘、禁打印。
//! 本类型**故意不实现** Debug / Display / Serialize——编译期挡住
//! 误格式化、误打日志、误进事件轨迹。取值只有一条显式路径 expose()，
//! Drop 时清零内存。任何需要展示凭据的需求一律是 bug。

/// 包裹敏感字符串。不实现 Debug/Display/Serialize（防日志泄密）。
pub struct Secret {
    inner: String,
}

impl Secret {
    /// 用已有字符串构造（调用方须保证来源是环境变量读取链路）。
    pub fn new(value: String) -> Self {
        Self { inner: value }
    }

    /// 从环境变量读取凭据；未设置或空串返回 None（不区分语义之外的错误）。
    pub fn from_env(name: &str) -> Option<Self> {
        match std::env::var(name) {
            Ok(v) if !v.is_empty() => Some(Self { inner: v }),
            _ => None,
        }
    }

    /// 显式取用明文。调用点必须可审计（目前唯一消费点：model.rs 构造请求头）。
    pub fn expose(&self) -> &str {
        &self.inner
    }

    /// 仅用于存在性检查（doctor 等），不返回内容。
    pub fn is_empty(&self) -> bool {
        self.inner.is_empty()
    }
}

impl Drop for Secret {
    fn drop(&mut self) {
        // 尽力清零（Rust 不保证字符串内存不被复制，这是缓解不是绝对保证）。
        // SAFETY: self.inner 是合法 UTF-8 String，原地覆写为 0 字节后仍可安全 drop
        //（长度不变，String::drop 只按 ptr/len/cap 释放，不要求 UTF-8）。
        unsafe {
            let bytes = self.inner.as_mut_vec();
            for b in bytes.iter_mut() {
                std::ptr::write_volatile(b, 0);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Secret;

    #[test]
    fn expose_roundtrip() {
        let s = Secret::new("dummy-value".to_string());
        assert_eq!(s.expose(), "dummy-value");
        assert!(!s.is_empty());
    }

    #[test]
    fn empty_env_returns_none() {
        assert!(Secret::from_env("SD_AGENT_TEST_VAR_DEFINITELY_NOT_SET_9f3a").is_none());
    }
}
