//! 运行时状态：磁盘上的配置 + 本次运行的覆盖项。

use crate::config::{self, Config, Thinking};
use anyhow::Result;
use std::path::PathBuf;

/// 命令行参数或 REPL 会话中的临时覆盖，不会写入配置文件
#[derive(Debug, Clone, Default)]
pub struct Overrides {
    pub model: Option<String>,
    pub thinking: Option<Thinking>,
    pub theme: Option<String>,
    pub font_style: Option<String>,
}

pub struct App {
    pub config_path: PathBuf,
    /// 配置文件中的内容（不含环境变量），`config set` 修改并保存的就是它
    pub file: Config,
    pub overrides: Overrides,
}

impl App {
    pub fn load(config_path: PathBuf, overrides: Overrides) -> Result<Self> {
        let file = Config::load(&config_path)?;
        Ok(Self {
            config_path,
            file,
            overrides,
        })
    }

    /// 生效配置：配置文件 < 环境变量 < 命令行 / 会话覆盖
    pub fn effective(&self) -> Config {
        let mut cfg = self.file.clone().with_env(config::std_env);
        if let Some(model) = &self.overrides.model {
            cfg.model = model.clone();
        }
        if let Some(thinking) = self.overrides.thinking {
            cfg.thinking = thinking;
        }
        if let Some(theme) = &self.overrides.theme {
            cfg.theme = theme.clone();
        }
        if let Some(font_style) = &self.overrides.font_style {
            cfg.font_style = font_style.clone();
        }
        cfg
    }

    pub fn save(&self) -> Result<()> {
        self.file.save(&self.config_path)
    }
}
