//! 体检项与体检报告（渲染为终端文本，全中文）。

/// 体检项结果（title/detail/hint/fix 全中文；fix 只在不通过时填写）。
#[derive(Debug, Clone)]
pub struct DoctorItem {
    /// 英文键（程序判断用，稳定不变）。
    pub name: &'static str,
    /// 中文短标题，如“模型凭据”。
    pub title: String,
    /// 是否通过。
    pub ok: bool,
    /// 中文一句话状态，如“已配置（内容隐藏）”。
    pub detail: String,
    /// 这一项是干什么的，一句大白话。
    pub hint: String,
    /// 不通过时怎么修（中文具体步骤；通过时为空串）。
    pub fix: String,
}

#[derive(Debug, Clone)]
pub struct DoctorReport {
    pub items: Vec<DoctorItem>,
}

impl DoctorReport {
    pub fn all_green(&self) -> bool {
        self.items.iter().all(|i| i.ok)
    }

    /// 渲染为终端文本：每项三行（状态 / 说明 / 失败时修法），全中文。
    pub fn render(&self) -> String {
        let mut out = String::new();
        out.push_str("sd-agent doctor —— 环境体检（六项）\n\n");
        for item in &self.items {
            out.push_str(&format!(
                "{} {}\n",
                if item.ok { "✅" } else { "❌" },
                item.title
            ));
            out.push_str(&format!("   状态：{}\n", item.detail));
            out.push_str(&format!("   说明：{}\n", item.hint));
            if !item.ok && !item.fix.is_empty() {
                out.push_str(&format!("   修法：{}\n", item.fix));
            }
            out.push('\n');
        }
        out.push_str(&format!(
            "总体结论：{}\n",
            if self.all_green() {
                "全部通过 ✅"
            } else {
                "有问题 ❌（照上面“修法”逐条处理）"
            }
        ));
        out
    }
}
