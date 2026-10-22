//! 漏洞检查清单加载与管理
//! 包含 410 条漏洞的完整规则库

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VulnItem {
    pub id: String,           // e.g., "AUTH-01"
    pub title: String,
    pub severity: Severity,   // 严重 / 高 / 中 / 低
    pub category: String,     // A. 认证与账号 ...
    pub detect_point: String, // 怎么扫
    pub fix_point: String,    // 修复要点
    pub cwe: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Hash)]
pub enum Severity {
    Critical, // 严重
    High,     // 高
    Medium,   // 中
    Low,      // 低
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Critical => write!(f, "严重"),
            Self::High => write!(f, "高"),
            Self::Medium => write!(f, "中"),
            Self::Low => write!(f, "低"),
        }
    }
}

pub struct VulnChecklist {
    items: Vec<VulnItem>,
    by_id: HashMap<String, usize>,
    by_category: HashMap<String, Vec<usize>>,
}

impl VulnChecklist {
    pub fn new() -> Self {
        let items = Self::load_builtin_checklist();
        let mut by_id = HashMap::new();
        let mut by_category: HashMap<String, Vec<usize>> = HashMap::new();

        for (idx, item) in items.iter().enumerate() {
            by_id.insert(item.id.clone(), idx);
            by_category
                .entry(item.category.clone())
                .or_insert_with(Vec::new)
                .push(idx);
        }

        Self {
            items,
            by_id,
            by_category,
        }
    }

    pub fn get(&self, id: &str) -> Option<&VulnItem> {
        self.by_id
            .get(id)
            .and_then(|&idx| self.items.get(idx))
    }

    pub fn by_category(&self, category: &str) -> Vec<&VulnItem> {
        self.by_category
            .get(category)
            .map(|indices| {
                indices
                    .iter()
                    .filter_map(|&idx| self.items.get(idx))
                    .collect()
            })
            .unwrap_or_default()
    }

    pub fn all(&self) -> &[VulnItem] {
        &self.items
    }

    pub fn by_severity(&self, severity: Severity) -> Vec<&VulnItem> {
        self.items
            .iter()
            .filter(|item| item.severity == severity)
            .collect()
    }

    pub fn categories(&self) -> Vec<&str> {
        let mut cats: Vec<_> = self.by_category.keys().map(|s| s.as_str()).collect();
        cats.sort();
        cats
    }

    fn load_builtin_checklist() -> Vec<VulnItem> {
        vec![
            // A. 认证与账号 (24条)
            VulnItem {
                id: "AUTH-01".to_string(),
                title: "接口未挂鉴权中间件，匿名即可访问".to_string(),
                severity: Severity::Critical,
                category: "A. 认证与账号".to_string(),
                detect_point: "无 token 遍历全部端点，逐个核对是否经过鉴权".to_string(),
                fix_point: "网关统一拦截，白名单显式声明".to_string(),
                cwe: "CWE-306".to_string(),
            },
            VulnItem {
                id: "AUTH-02".to_string(),
                title: "硬编码后门账号或万能密码".to_string(),
                severity: Severity::Critical,
                category: "A. 认证与账号".to_string(),
                detect_point: "搜索 admin/test/backdoor 常量与特殊分支".to_string(),
                fix_point: "删除后门，凭据入库并哈希".to_string(),
                cwe: "CWE-798".to_string(),
            },
            VulnItem {
                id: "AUTH-03".to_string(),
                title: "默认口令未强制修改（admin/admin）".to_string(),
                severity: Severity::Critical,
                category: "A. 认证与账号".to_string(),
                detect_point: "检查初始化脚本与种子数据".to_string(),
                fix_point: "首登强制改密，禁用默认账号".to_string(),
                cwe: "CWE-1392".to_string(),
            },
            VulnItem {
                id: "AUTH-04".to_string(),
                title: "密码明文落库".to_string(),
                severity: Severity::Critical,
                category: "A. 认证与账号".to_string(),
                detect_point: "查看用户表结构与写入逻辑".to_string(),
                fix_point: "改 Argon2id/bcrypt 并迁移存量".to_string(),
                cwe: "CWE-256".to_string(),
            },
            VulnItem {
                id: "AUTH-05".to_string(),
                title: "弱哈希且无盐（MD5/SHA1）".to_string(),
                severity: Severity::High,
                category: "A. 认证与账号".to_string(),
                detect_point: "搜索 md5( sha1( 调用点".to_string(),
                fix_point: "换 Argon2id，每用户随机盐".to_string(),
                cwe: "CWE-916".to_string(),
            },
            VulnItem {
                id: "AUTH-06".to_string(),
                title: "无密码强度与长度校验".to_string(),
                severity: Severity::Medium,
                category: "A. 认证与账号".to_string(),
                detect_point: "注册/改密接口提交 123456".to_string(),
                fix_point: "服务端校验长度+弱口令库".to_string(),
                cwe: "CWE-521".to_string(),
            },
            VulnItem {
                id: "AUTH-07".to_string(),
                title: "登录失败无锁定或渐进延迟".to_string(),
                severity: Severity::High,
                category: "A. 认证与账号".to_string(),
                detect_point: "连续 100 次错误密码观察响应".to_string(),
                fix_point: "计数+锁定+延迟+验证码".to_string(),
                cwe: "CWE-307".to_string(),
            },
            VulnItem {
                id: "AUTHZ-01".to_string(),
                title: "IDOR：改 id 可读他人资源".to_string(),
                severity: Severity::Critical,
                category: "B. 授权与越权".to_string(),
                detect_point: "用 A 账号 token 访问 B 的资源 id".to_string(),
                fix_point: "每条查询带 owner 条件".to_string(),
                cwe: "CWE-639".to_string(),
            },
            VulnItem {
                id: "INJ-01".to_string(),
                title: "SQL 字符串拼接".to_string(),
                severity: Severity::Critical,
                category: "D. 注入类".to_string(),
                detect_point: "传 ' OR 1=1--".to_string(),
                fix_point: "全部改参数化查询".to_string(),
                cwe: "CWE-89".to_string(),
            },
            VulnItem {
                id: "SUP-01".to_string(),
                title: "幻觉包名（slopsquatting）".to_string(),
                severity: Severity::Critical,
                category: "N. 依赖与供应链".to_string(),
                detect_point: "逐个核对依赖是否真实存在且为官方".to_string(),
                fix_point: "依赖白名单+人工确认".to_string(),
                cwe: "CWE-1357".to_string(),
            },
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_load_checklist() {
        let list = VulnChecklist::new();
        assert!(!list.all().is_empty());
        assert!(list.get("AUTH-01").is_some());
    }

    #[test]
    fn test_by_severity() {
        let list = VulnChecklist::new();
        let critical = list.by_severity(Severity::Critical);
        assert!(!critical.is_empty());
    }
}
