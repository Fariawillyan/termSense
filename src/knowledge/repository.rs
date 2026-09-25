//! In-memory, read-only access to the knowledge base.

use std::collections::HashMap;

use super::loader::{self, LoadError, Loaded};
use super::model::{Category, Entry, EntryKind};
use crate::config::Config;

/// Indexed knowledge. Entries are addressed by position (`usize`) so other
/// components (the search index) can refer to them without borrowing.
#[derive(Debug)]
pub struct Repository {
    entries: Vec<Entry>,
    by_id: HashMap<String, usize>,
    /// Top-level command name → entry (subcommands are reached via `children`).
    commands: HashMap<String, usize>,
    children: HashMap<String, Vec<usize>>,
    /// Entries with a `flag_prefix`, in load order.
    flag_owners: Vec<usize>,
    categories: Vec<Category>,
    warnings: Vec<String>,
    /// No entry came from the user's files: the precompiled index applies.
    builtin_only: bool,
}

impl Repository {
    pub fn new(loaded: Loaded) -> Self {
        let Loaded {
            entries,
            categories,
            warnings,
            user_entries,
            ..
        } = loaded;
        let mut by_id = HashMap::with_capacity(entries.len());
        let mut commands = HashMap::new();
        let mut children: HashMap<String, Vec<usize>> = HashMap::new();
        let mut flag_owners = Vec::new();
        for (i, e) in entries.iter().enumerate() {
            by_id.insert(e.id.clone(), i);
            match &e.parent {
                Some(parent) => children.entry(parent.clone()).or_default().push(i),
                None if e.kind == EntryKind::Command => {
                    commands.insert(e.name.to_lowercase(), i);
                    for name in &e.names {
                        commands.entry(name.to_lowercase()).or_insert(i);
                    }
                }
                None => {}
            }
            if e.flag_prefix.is_some() {
                flag_owners.push(i);
            }
        }
        for list in children.values_mut() {
            list.sort_by(|&a, &b| entries[a].name.cmp(&entries[b].name));
        }
        Self {
            entries,
            by_id,
            commands,
            children,
            flag_owners,
            categories,
            warnings,
            builtin_only: user_entries == 0,
        }
    }

    /// Built-in knowledge plus the user's knowledge directories.
    pub fn load(config: &Config) -> Result<Self, LoadError> {
        loader::load(&config.knowledge_dirs).map(Self::new)
    }

    /// Built-in knowledge only.
    #[cfg(test)]
    pub fn embedded() -> Result<Self, LoadError> {
        loader::load_embedded().map(Self::new)
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Only the built-in knowledge was loaded (no user files).
    pub fn builtin_only(&self) -> bool {
        self.builtin_only
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn entry(&self, index: usize) -> &Entry {
        &self.entries[index]
    }

    pub fn get(&self, id: &str) -> Option<&Entry> {
        self.by_id.get(id).map(|&i| &self.entries[i])
    }

    /// Top-level command by executable name (`grep`, `git`, or an
    /// alternative name such as `mvnw`). A path resolves by its last
    /// component: `/usr/bin/grep`, `./mvnw`.
    pub fn command(&self, name: &str) -> Option<&Entry> {
        let lookup = |n: &str| {
            self.commands
                .get(&n.to_lowercase())
                .map(|&i| &self.entries[i])
        };
        lookup(name).or_else(|| {
            let (_, base) = name.rsplit_once('/')?;
            (!base.is_empty()).then(|| lookup(base)).flatten()
        })
    }

    /// Entry whose `flag_prefix` starts `flag` (`--gtest_filter` → GoogleTest).
    pub fn flag_owner(&self, flag: &str) -> Option<&Entry> {
        self.flag_owners
            .iter()
            .map(|&i| &self.entries[i])
            .find(|e| {
                e.flag_prefix
                    .as_deref()
                    .is_some_and(|p| flag.starts_with(p))
            })
    }

    /// A Kubernetes/OpenShift resource kind (a concept tagged `recurso`) by
    /// any of its spellings: name, alias or tag (`Deployment`, `deployments`,
    /// `deploy`). Short names live in tags so they never annotate other
    /// commands' arguments.
    pub fn resource_kind(&self, word: &str) -> Option<&Entry> {
        let word = word.to_lowercase();
        self.entries.iter().find(|e| {
            e.kind == EntryKind::Concept
                && e.tags.iter().any(|t| t == "recurso")
                && (e.name.to_lowercase() == word
                    || e.aliases.iter().any(|a| a.to_lowercase() == word)
                    || e.tags.contains(&word))
        })
    }

    /// Subcommands of `parent_id`, sorted by name.
    pub fn children(&self, parent_id: &str) -> impl Iterator<Item = &Entry> + '_ {
        self.children
            .get(parent_id)
            .into_iter()
            .flatten()
            .map(|&i| &self.entries[i])
    }

    pub fn has_children(&self, parent_id: &str) -> bool {
        self.children.contains_key(parent_id)
    }

    /// Subcommand of `parent_id` whose last word is `word` (`status`), or
    /// with an alias spelled `<parent> <word>` (`ip a` for `ip addr`).
    pub fn child(&self, parent_id: &str, word: &str) -> Option<&Entry> {
        let parent = self.get(parent_id)?;
        let spelled = format!("{} {}", parent.name, word);
        self.children(parent_id)
            .find(|e| e.leaf_name() == word)
            .or_else(|| {
                self.children(parent_id)
                    .find(|e| e.aliases.contains(&spelled))
            })
    }

    /// Concept whose name (ignoring case) or alias (exact case) equals
    /// `name`: `POST`, `Content-Type`, `MX`. Used to annotate argument values;
    /// aliases are case-sensitive so a lowercase `a` is not the DNS record `A`.
    pub fn concept(&self, name: &str) -> Option<&Entry> {
        // Resource kinds (`build`, `secret`, `pod`) are common words: they
        // are recognized only where a resource is expected (`resource_kind`).
        let concepts = || {
            self.entries
                .iter()
                .filter(|e| e.kind == EntryKind::Concept && !e.tags.iter().any(|t| t == "recurso"))
        };
        concepts()
            .find(|e| e.name.eq_ignore_ascii_case(name))
            .or_else(|| concepts().find(|e| e.aliases.iter().any(|a| a == name)))
    }

    /// Resolves related ids, skipping unknown ones.
    pub fn related<'a>(&'a self, entry: &'a Entry) -> impl Iterator<Item = &'a Entry> + 'a {
        entry.related.iter().filter_map(|id| self.get(id))
    }

    pub fn categories(&self) -> &[Category] {
        &self.categories
    }

    pub fn category(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    pub fn in_category<'a>(&'a self, id: &'a str) -> impl Iterator<Item = &'a Entry> + 'a {
        self.entries.iter().filter(move |e| e.category == id)
    }

    /// Non-fatal loading problems, shown in the status bar.
    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo() -> Repository {
        Repository::embedded().unwrap()
    }

    #[test]
    fn resolves_commands_and_subcommands() {
        let repo = repo();
        assert_eq!(repo.command("grep").unwrap().id, "grep");
        assert_eq!(repo.command("GIT").unwrap().id, "git");
        assert_eq!(repo.child("git", "status").unwrap().id, "git-status");
        assert!(
            repo.command("status").is_none(),
            "subcommands are not top-level"
        );
    }

    #[test]
    fn spec_required_knowledge_exists() {
        let repo = repo();
        let required = [
            // Linux
            "ls",
            "cd",
            "pwd",
            "cp",
            "mv",
            "rm",
            "mkdir",
            "touch",
            "cat",
            "less",
            "head",
            "tail",
            "file",
            "stat",
            "find",
            "xargs",
            "chmod",
            "chown",
            "ln",
            "df",
            "du",
            "mount",
            "which",
            "whereis",
            "locate",
            "groups",
            "grep",
            // Processes
            "ps",
            "top",
            "htop",
            "kill",
            "pkill",
            "jobs",
            "fg",
            "bg",
            "nohup",
            "nice",
            "renice",
            // Networking
            "ip",
            "ss",
            "ping",
            "traceroute",
            "tracepath",
            "curl",
            "wget",
            "nc",
            "dig",
            "nslookup",
            "host",
            "hostname",
            "getent",
            "arp",
            "route",
            "tcpdump",
            "openssl",
            "ssh",
            "scp",
            "sftp",
            "lsof",
            "netstat",
            // SSH
            "ssh-keygen",
            "ssh-agent",
            "ssh-add",
            // Git
            "git",
            "git-status",
            "git-log",
            "git-diff",
            "git-branch",
            "git-switch",
            "git-checkout",
            "git-stash",
            "git-rebase",
            "git-reset",
            "git-restore",
            "git-fetch",
            "git-pull",
            "git-push",
            "git-merge",
            "git-cherry-pick",
            "git-revert",
            // Docker
            "docker",
            "docker-ps",
            "docker-images",
            "docker-logs",
            "docker-exec",
            "docker-inspect",
            "docker-network",
            "docker-volume",
            "docker-compose",
            // Concepts
            "regex",
            "tcp",
            "udp",
            "ip-protocol",
            "ipv4",
            "ipv6",
            "icmp",
            "arp-protocol",
            "nat",
            "dns",
            "dhcp",
            "socket",
            "porta",
            "gateway",
            "subnet",
            "cidr",
            "localhost",
            "loopback",
            "interface",
            "tls",
            "https",
            "certificado",
            "ca",
            "handshake",
            "sni",
            "http",
            "pipe",
            "redirect",
            "stdin",
            "stdout",
            "stderr",
            "environment-variables",
            "command-substitution",
            "globbing",
            "quotes",
            "escaping",
            "exit-code",
            "process-substitution",
            "subshell",
            // Shell grammar and builtins
            "if",
            "for",
            "while",
            "case",
            "double-bracket",
            "test-bracket",
            "test",
            "shell-function",
            "arithmetic",
            "heredoc",
            "parameter-expansion",
            "special-variables",
            "read",
            "set",
            // Tools, packages, editors, WSL
            "jq",
            "rsync",
            "crontab",
            "cron-syntax",
            "apt",
            "dpkg",
            "dnf",
            "vim",
            "nano",
            "tmux",
            "ufw",
            "wslpath",
            "crlf",
            "git-reflog",
            "docker-compose-up",
            // Java, C++, Node, OpenShift
            "java",
            "javac",
            "mvn",
            "mvn-install",
            "keytool",
            "gpp",
            "cmake",
            "ctest",
            "gtest",
            "npm",
            "npm-install",
            "npx",
            "nvm",
            "oc",
            "oc-get",
            "oc-logs",
            "oc-rollout-restart",
            "pod-kind",
            // Error messages
            "error-command-not-found",
            "error-crashloopbackoff",
            "error-pkix",
            "error-undefined-reference",
            "error-npm-eresolve",
            "error-permission-denied",
            "error-ssh-publickey",
            "error-no-space",
        ];
        for id in required {
            assert!(repo.get(id).is_some(), "falta a entrada '{id}'");
        }
    }
}
