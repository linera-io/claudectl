#![allow(dead_code)]

/// Configuration for an external agent (Codex, Aider, custom scripts).
#[derive(Debug, Clone)]
pub struct AgentConfig {
    pub name: String,
    pub agent_type: String,
    pub command: String,
    pub capabilities: Vec<String>,
    pub cwd: String,
}

impl AgentConfig {
    pub fn new(name: String) -> Self {
        Self {
            name,
            agent_type: "custom".into(),
            command: String::new(),
            capabilities: Vec::new(),
            cwd: ".".into(),
        }
    }

    /// Format as a compact line for the brain prompt.
    pub fn prompt_line(&self) -> String {
        let caps = if self.capabilities.is_empty() {
            "general".to_string()
        } else {
            self.capabilities.join(", ")
        };
        format!("- {}: {} ({})", self.name, caps, self.agent_type)
    }
}

/// Format all registered agents as a prompt section for the brain.
pub fn format_agents_prompt(agents: &[AgentConfig]) -> String {
    if agents.is_empty() {
        return String::new();
    }

    let mut lines = vec!["## Available Agents".to_string()];
    for agent in agents {
        lines.push(agent.prompt_line());
    }
    lines.join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn agent_config_defaults() {
        let a = AgentConfig::new("test".into());
        assert_eq!(a.name, "test");
        assert_eq!(a.agent_type, "custom");
        assert_eq!(a.cwd, ".");
        assert!(a.capabilities.is_empty());
    }

    #[test]
    fn prompt_line_format() {
        let mut a = AgentConfig::new("codex".into());
        a.agent_type = "codex".into();
        a.capabilities = vec!["code-review".into(), "refactoring".into()];
        let line = a.prompt_line();
        assert!(line.contains("codex"));
        assert!(line.contains("code-review, refactoring"));
    }

    #[test]
    fn format_agents_prompt_empty() {
        assert_eq!(format_agents_prompt(&[]), "");
    }

    #[test]
    fn format_agents_prompt_multiple() {
        let agents = vec![
            {
                let mut a = AgentConfig::new("codex".into());
                a.capabilities = vec!["review".into()];
                a
            },
            {
                let mut a = AgentConfig::new("aider".into());
                a.capabilities = vec!["implementation".into()];
                a
            },
        ];
        let output = format_agents_prompt(&agents);
        assert!(output.contains("Available Agents"));
        assert!(output.contains("codex"));
        assert!(output.contains("aider"));
    }

    #[test]
    fn parse_agent_config_from_fields() {
        let mut a = AgentConfig::new("test-agent".into());
        a.agent_type = "aider".into();
        a.command = "aider --yes".into();
        a.capabilities = vec!["debugging".into(), "implementation".into()];
        a.cwd = "/tmp/project".into();

        assert_eq!(a.command, "aider --yes");
        assert_eq!(a.capabilities.len(), 2);
    }
}
