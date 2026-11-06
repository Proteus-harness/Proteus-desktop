//! Public repositories offered in the sources dialog. Official sources are
//! added once; removing them remains a persistent user choice.

use super::Source;

pub struct Open {
    pub title: &'static str,
    pub url: &'static str,
    pub about: &'static str,
}

impl Open {
    pub fn matches(&self, source: &Source) -> bool {
        self.url
            .trim_end_matches('/')
            .trim_end_matches(".git")
            .eq_ignore_ascii_case(source.url.trim_end_matches('/').trim_end_matches(".git"))
    }

    pub fn source(&self) -> Source {
        Source {
            url: self.url.to_string(),
            branch: None,
        }
    }
}

pub const OPEN: [Open; 4] = [
    Open {
        title: "Anthropic",
        url: "https://github.com/anthropics/skills",
        about: "docx, pdf, pptx, xlsx, frontend-design, mcp-builder, skill-creator",
    },
    Open {
        title: "OpenAI",
        url: "https://github.com/openai/skills",
        about: "Curated skills Codex: GitHub, Figma, Playwright, Sentry, деплой",
    },
    Open {
        title: "Superpowers",
        url: "https://github.com/obra/superpowers",
        about: "Планы, TDD, отладка, code review, git worktrees",
    },
    Open {
        title: "Hugging Face",
        url: "https://github.com/huggingface/skills",
        about: "Hub, датасеты, обучение моделей, Spaces",
    },
];

pub const OFFICIAL: [Open; 2] = [
    Open {
        title: "Proteus Modules",
        url: "https://github.com/Proteus-harness/Proteus-modules.git",
        about: "Процессные модули: Tools, модели, workflow, контекст, policy и hooks",
    },
    Open {
        title: "Proteus Extensions",
        url: "https://github.com/Proteus-harness/Proteus-desktop-extensions.git",
        about: "Панели, документы, виджеты и рендереры клиента",
    },
];
