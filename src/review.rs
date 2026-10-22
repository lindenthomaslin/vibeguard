//! 逐条确认修复的流程，以及用于预览界面的演示数据。

use crate::finding::{Finding, Fix, Severity, sorted};
use crate::{scan_ui, ui};
use anyhow::{Result, bail};
use dialoguer::Select;
use dialoguer::theme::ColorfulTheme;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    Apply,
    Skip,
    /// 应用当前这条以及之后所有的修复
    ApplyRest,
    /// 结束确认，已选中的保留，其余不修复
    Quit,
}

/// 逐条确认的状态机：按严重级别从高到低依次给出待确认项，记录用户的选择。
/// 持有自己的数据、不碰终端，阻塞式（`review_with`）和按键驱动式（全屏界面）都基于它。
pub struct Reviewer {
    pairs: Vec<(Finding, Fix)>,
    next: usize,
    accepted: Vec<Fix>,
}

impl Reviewer {
    pub fn new(findings: &[Finding], fixes: &[Fix]) -> Self {
        // 只处理既有发现又有修复方案的条目
        let pairs = sorted(findings)
            .into_iter()
            .filter_map(|f| {
                fixes
                    .iter()
                    .find(|x| x.finding_id == f.id)
                    .map(|x| (f.clone(), x.clone()))
            })
            .collect();
        Self {
            pairs,
            next: 0,
            accepted: Vec::new(),
        }
    }

    pub fn total(&self) -> usize {
        self.pairs.len()
    }

    /// 当前待确认项：（发现, 修复, 序号（从 1 开始）, 总数）；全部处理完返回 `None`
    pub fn current(&self) -> Option<(&Finding, &Fix, usize, usize)> {
        self.pairs
            .get(self.next)
            .map(|(f, x)| (f, x, self.next + 1, self.total()))
    }

    pub fn answer(&mut self, decision: Decision) {
        let Some((_, fix)) = self.pairs.get(self.next) else {
            return;
        };
        match decision {
            Decision::Apply => {
                self.accepted.push(fix.clone());
                self.next += 1;
            }
            Decision::Skip => self.next += 1,
            Decision::ApplyRest => {
                let rest = self.pairs[self.next..].iter().map(|(_, x)| x.clone());
                self.accepted.extend(rest);
                self.next = self.total();
            }
            Decision::Quit => self.next = self.total(),
        }
    }

    #[cfg(test)]
    pub fn accepted(&self) -> &[Fix] {
        &self.accepted
    }

    pub fn into_accepted(self) -> Vec<Fix> {
        self.accepted
    }
}

/// 阻塞式逐条询问，返回被接受的修复。
/// `ask` 负责展示与提问，传入的是（发现, 修复, 序号, 总数）。
pub fn review_with(
    findings: &[Finding],
    fixes: &[Fix],
    mut ask: impl FnMut(&Finding, &Fix, usize, usize) -> Result<Decision>,
) -> Result<Vec<Fix>> {
    let mut reviewer = Reviewer::new(findings, fixes);
    while let Some((finding, fix, index, total)) = reviewer.current() {
        let decision = ask(finding, fix, index, total)?;
        reviewer.answer(decision);
    }
    Ok(reviewer.into_accepted())
}

/// 终端里的交互版本：先画卡片和 diff，再用选择菜单询问
pub fn review_interactive(findings: &[Finding], fixes: &[Fix]) -> Result<Vec<Fix>> {
    if !console::user_attended() {
        bail!("逐条确认需要在交互式终端中运行");
    }
    review_with(findings, fixes, |finding, fix, index, total| {
        let width = ui::term_width();
        println!();
        for line in scan_ui::finding_card(finding, index, total, width) {
            println!("{line}");
        }
        println!();
        for line in scan_ui::diff_lines(fix, 3, width) {
            println!("{line}");
        }
        println!();
        let choice = Select::with_theme(&ColorfulTheme::default())
            .with_prompt("应用这个修复？")
            .items(&["应用", "跳过", "应用此条及之后全部", "结束确认"])
            .default(0)
            .interact()?;
        Ok(match choice {
            0 => Decision::Apply,
            1 => Decision::Skip,
            2 => Decision::ApplyRest,
            _ => Decision::Quit,
        })
    })
}

/// 演示用的假数据，仅用于预览界面样式
pub fn demo_data() -> (Vec<Finding>, Vec<Fix>) {
    let finding = |id: &str, severity, title: &str, cwe: &str, file: &str, line, desc: &str, sug: &str| {
        Finding {
            id: id.into(),
            severity,
            title: title.into(),
            cwe: Some(cwe.into()),
            file: file.into(),
            line,
            description: desc.into(),
            suggestion: sug.into(),
        }
    };
    let findings = vec![
        finding(
            "VG-001",
            Severity::Critical,
            "硬编码的 API 密钥",
            "CWE-798",
            "src/config.py",
            3,
            "密钥直接写在源码里，一旦仓库公开或泄露就等于凭据泄露。",
            "改为从环境变量读取，并轮换已泄露的密钥。",
        ),
        finding(
            "VG-002",
            Severity::High,
            "SQL 注入：用户输入被拼接进查询",
            "CWE-89",
            "src/db.py",
            4,
            "查询语句通过 f-string 构造，攻击者可以注入任意 SQL。",
            "改用参数化查询。",
        ),
        finding(
            "VG-003",
            Severity::Medium,
            "CORS 允许任意来源",
            "CWE-942",
            "src/app.py",
            2,
            "Access-Control-Allow-Origin 设为 *，任意网站都能跨域读取响应。",
            "只放行明确的可信来源。",
        ),
    ];
    let fixes = vec![
        Fix {
            finding_id: "VG-001".into(),
            file: "src/config.py".into(),
            old: "import os\n\nAPI_KEY = \"sk-live-1234567890abcdef\"\nDEBUG = False\n".into(),
            new: "import os\n\nAPI_KEY = os.environ[\"API_KEY\"]\nDEBUG = False\n".into(),
        },
        Fix {
            finding_id: "VG-002".into(),
            file: "src/db.py".into(),
            old: "def get_user(conn, name):\n    cur = conn.cursor()\n    cur.execute(f\"SELECT * FROM users WHERE name = '{name}'\")\n    return cur.fetchone()\n".into(),
            new: "def get_user(conn, name):\n    cur = conn.cursor()\n    cur.execute(\"SELECT * FROM users WHERE name = ?\", (name,))\n    return cur.fetchone()\n".into(),
        },
        Fix {
            finding_id: "VG-003".into(),
            file: "src/app.py".into(),
            old: "app = Flask(__name__)\nCORS(app, origins=\"*\")\n".into(),
            new: "app = Flask(__name__)\nCORS(app, origins=[\"https://example.com\"])\n".into(),
        },
    ];
    (findings, fixes)
}

/// 预览整套界面：汇总、逐条确认、结果
pub fn run_demo() -> Result<()> {
    let (findings, fixes) = demo_data();
    ui::warn("演示模式：以下是示例数据，不会读取或修改任何文件");
    println!();
    for line in scan_ui::summary_lines(&findings) {
        println!("{line}");
    }
    let accepted = review_interactive(&findings, &fixes)?;
    println!();
    if accepted.is_empty() {
        ui::info("没有选中任何修复");
    } else {
        let names: Vec<_> = accepted.iter().map(|f| f.finding_id.as_str()).collect();
        ui::success(&format!(
            "已选中 {} 个修复：{}（演示模式，未写入文件）",
            accepted.len(),
            names.join("、")
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(decisions: &[Decision]) -> Vec<String> {
        let (findings, fixes) = demo_data();
        let mut asked = Vec::new();
        let mut it = decisions.iter().copied();
        let accepted = review_with(&findings, &fixes, |f, _, i, n| {
            asked.push((f.id.clone(), i, n));
            Ok(it.next().expect("asked more times than expected"))
        })
        .unwrap();
        assert!(asked.iter().all(|&(_, _, n)| n == 3));
        accepted.iter().map(|f| f.finding_id.clone()).collect()
    }

    #[test]
    fn asks_in_severity_order_and_collects_applied() {
        let got = run(&[Decision::Apply, Decision::Skip, Decision::Apply]);
        assert_eq!(got, ["VG-001", "VG-003"]);
    }

    #[test]
    fn apply_rest_stops_asking() {
        let got = run(&[Decision::Skip, Decision::ApplyRest]);
        assert_eq!(got, ["VG-002", "VG-003"]);
    }

    #[test]
    fn quit_keeps_earlier_choices_only() {
        let got = run(&[Decision::Apply, Decision::Quit]);
        assert_eq!(got, ["VG-001"]);
    }

    #[test]
    fn findings_without_a_fix_are_not_asked() {
        let (findings, mut fixes) = demo_data();
        fixes.retain(|f| f.finding_id != "VG-002");
        let mut ids = Vec::new();
        review_with(&findings, &fixes, |f, _, _, n| {
            assert_eq!(n, 2);
            ids.push(f.id.clone());
            Ok(Decision::Skip)
        })
        .unwrap();
        assert_eq!(ids, ["VG-001", "VG-003"]);
    }

    #[test]
    fn reviewer_steps_through_items_and_ignores_answers_after_the_end() {
        let (findings, fixes) = demo_data();
        let mut r = Reviewer::new(&findings, &fixes);
        assert_eq!(r.total(), 3);
        let (f, _, i, n) = r.current().unwrap();
        assert_eq!((f.id.as_str(), i, n), ("VG-001", 1, 3));

        r.answer(Decision::Skip);
        assert_eq!(r.current().unwrap().2, 2);
        r.answer(Decision::Apply);
        r.answer(Decision::Apply);
        assert!(r.current().is_none());

        r.answer(Decision::Apply); // 已结束，忽略
        let ids: Vec<_> = r.accepted().iter().map(|f| f.finding_id.as_str()).collect();
        assert_eq!(ids, ["VG-002", "VG-003"]);
    }

    #[test]
    fn reviewer_with_nothing_to_review_is_immediately_done() {
        let r = Reviewer::new(&[], &[]);
        assert!(r.current().is_none());
        assert!(r.into_accepted().is_empty());
    }

    #[test]
    fn errors_from_ask_propagate() {
        let (findings, fixes) = demo_data();
        let res = review_with(&findings, &fixes, |_, _, _, _| bail!("用户取消"));
        assert!(res.is_err());
    }

    #[test]
    fn demo_fixes_all_change_something_and_match_findings() {
        let (findings, fixes) = demo_data();
        assert_eq!(findings.len(), fixes.len());
        for fix in &fixes {
            assert!(findings.iter().any(|f| f.id == fix.finding_id && f.file == fix.file));
            assert_ne!(fix.old, fix.new);
        }
    }
}
