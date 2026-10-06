//! 模型配置表单状态（契约文件：结构冻结，双方只读引用）。
//!
//! "+" 添加 / 编辑模型配置的弹窗表单（替代 prompt/confirm 连弹的纪律约定）。
//! 交互层（app）驱动字段编辑与保存；渲染层（ui/widgets）只读渲染。
//! 落盘走核心 Settings（upsert_profile + set_active + save）。

/// 表单字段（Tab/点击切换焦点）。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FormField {
    Label,
    BaseUrl,
    Model,
    ApiKey,
    Effort,
}

/// 表单字段顺序（Tab 轮转用）。
pub const FORM_FIELDS: [FormField; 5] = [
    FormField::Label,
    FormField::BaseUrl,
    FormField::Model,
    FormField::ApiKey,
    FormField::Effort,
];

/// 表单状态。
#[derive(Debug, Default)]
pub struct FormState {
    /// 配置名（唯一标识）。
    pub label: String,
    /// OpenAI 兼容端点。
    pub base_url: String,
    /// 模型名。
    pub model: String,
    /// API 密钥（展示打码；空=不改既有值）。
    pub api_key: String,
    /// 思考强度档位索引（0..7，对齐 crate::slash::EFFORTS）。
    pub effort_idx: usize,
    /// 当前聚焦字段。
    pub focus: FormField,
    /// 校验错误提示（保存时填）。
    pub error: Option<String>,
    /// 编辑既有配置时的原 label（None=新增）。
    pub edit_label: Option<String>,
}

impl Default for FormField {
    fn default() -> Self {
        FormField::Label
    }
}

impl FormState {
    /// 新增配置的空表单（默认端点占位、思考强度 medium=3）。
    pub fn add() -> Self {
        Self {
            base_url: "https://".into(),
            effort_idx: 3,
            focus: FormField::Label,
            ..Default::default()
        }
    }

    /// 编辑既有配置（api_key 留空=不改）。
    /// 编辑入口尚未接线（长按/右键 [待磨合]），保留契约方法。
    #[allow(dead_code)]
    pub fn edit(label: &str, base_url: &str, model: &str, effort_idx: usize) -> Self {
        Self {
            label: label.into(),
            base_url: base_url.into(),
            model: model.into(),
            api_key: String::new(),
            effort_idx,
            focus: FormField::Label,
            error: None,
            edit_label: Some(label.into()),
        }
    }

    /// 焦点字段的可编辑缓冲（Effort 是档位非文本，返回 None 走方向键）。
    pub fn field_mut(&mut self) -> Option<&mut String> {
        match self.focus {
            FormField::Label => Some(&mut self.label),
            FormField::BaseUrl => Some(&mut self.base_url),
            FormField::Model => Some(&mut self.model),
            FormField::ApiKey => Some(&mut self.api_key),
            FormField::Effort => None,
        }
    }

    /// Tab 下一字段。
    pub fn focus_next(&mut self) {
        let idx = FORM_FIELDS
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        self.focus = FORM_FIELDS[(idx + 1) % FORM_FIELDS.len()];
    }

    /// 校验（label/base_url/model 必填）。返回 Err(中文提示)。
    pub fn validate(&self) -> Result<(), String> {
        if self.label.trim().is_empty() {
            return Err("配置名不能为空".into());
        }
        if !self.base_url.trim().starts_with("http") {
            return Err("端点需以 http(s):// 开头".into());
        }
        if self.model.trim().is_empty() {
            return Err("模型名不能为空".into());
        }
        Ok(())
    }

    // ── 以下为交互层方法（新增，不动冻结字段） ──

    /// 反向字段轮转（Shift+Tab）。
    pub fn focus_prev(&mut self) {
        let idx = FORM_FIELDS
            .iter()
            .position(|f| *f == self.focus)
            .unwrap_or(0);
        self.focus = FORM_FIELDS[(idx + FORM_FIELDS.len() - 1) % FORM_FIELDS.len()];
    }

    /// 点击定位字段（鼠标命中行 → 字段）。
    pub fn focus_field(&mut self, f: FormField) {
        self.focus = f;
    }

    /// 字符插入聚焦字段（Effort 是档位，无可编辑文本，忽略）。
    /// 无字段内光标：一律尾部编辑（表单短文本，够用）。
    pub fn insert(&mut self, ch: char) {
        if let Some(buf) = self.field_mut() {
            buf.push(ch);
        }
    }

    /// 删尾字符。
    pub fn backspace(&mut self) {
        if let Some(buf) = self.field_mut() {
            buf.pop();
        }
    }

    /// 删除键：无独立光标，与 backspace 同义（尾部删除）。
    pub fn delete(&mut self) {
        self.backspace();
    }

    /// Effort 档位 -1（clamp 到 0）。
    pub fn effort_dec(&mut self) {
        self.effort_idx = self.effort_idx.saturating_sub(1);
    }

    /// Effort 档位 +1（clamp 到 EFFORTS 末档）。
    pub fn effort_inc(&mut self) {
        let max = crate::slash::EFFORTS.len().saturating_sub(1);
        if self.effort_idx < max {
            self.effort_idx += 1;
        }
    }

    /// 当前 Effort 档位名（对齐 crate::slash::EFFORTS；越界回落 medium）。
    pub fn effort_name(&self) -> &'static str {
        crate::slash::EFFORTS
            .get(self.effort_idx)
            .copied()
            .unwrap_or("medium")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn add_defaults() {
        let f = FormState::add();
        assert_eq!(f.focus, FormField::Label);
        assert_eq!(f.effort_idx, 3);
        assert!(f.base_url.starts_with("https://"));
        assert!(f.edit_label.is_none());
    }

    #[test]
    fn focus_wraps_both_directions() {
        let mut f = FormState::add();
        f.focus_prev();
        assert_eq!(f.focus, FormField::Effort);
        f.focus_next();
        assert_eq!(f.focus, FormField::Label);
        for _ in 0..FORM_FIELDS.len() {
            f.focus_next();
        }
        assert_eq!(f.focus, FormField::Label);
    }

    #[test]
    fn insert_backspace_on_focused_field() {
        let mut f = FormState::add();
        for c in "配置甲".chars() {
            f.insert(c);
        }
        assert_eq!(f.label, "配置甲");
        f.focus_next();
        f.insert('x');
        assert_eq!(f.base_url, "https://x");
        f.backspace();
        assert_eq!(f.base_url, "https://");
    }

    #[test]
    fn effort_field_edits_gear_not_text() {
        let mut f = FormState::add();
        f.focus_field(FormField::Effort);
        f.insert('x');
        f.backspace();
        assert!(f.field_mut().is_none());
        f.effort_inc();
        assert_eq!(f.effort_idx, 4);
        assert_eq!(f.effort_name(), "high");
        for _ in 0..10 {
            f.effort_inc();
        }
        assert_eq!(f.effort_idx, crate::slash::EFFORTS.len() - 1);
        for _ in 0..10 {
            f.effort_dec();
        }
        assert_eq!(f.effort_idx, 0);
    }

    #[test]
    fn validate_reports_first_missing() {
        let mut f = FormState::add();
        f.label.clear();
        assert_eq!(f.validate(), Err("配置名不能为空".into()));
        f.label = "a".into();
        f.base_url.clear();
        assert_eq!(f.validate(), Err("端点需以 http(s):// 开头".into()));
        f.base_url = "https://x".into();
        f.model.clear();
        assert_eq!(f.validate(), Err("模型名不能为空".into()));
        f.model = "m".into();
        assert!(f.validate().is_ok());
    }
}
