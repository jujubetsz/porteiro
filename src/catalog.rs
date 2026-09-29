mod metadata;

pub use metadata::{AccentColor, HostMetadata};

use glob::{glob_with, MatchOptions, Pattern};
use sha1::{Digest, Sha1};
use std::collections::{BTreeMap, HashMap, HashSet};
#[cfg(unix)]
use std::ffi::{CStr, CString};
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use thiserror::Error;

const MAX_INCLUDE_DEPTH: usize = 16;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostHints {
    pub hostname: Option<String>,
    pub user: Option<String>,
    pub port: Option<String>,
    pub proxy_jump: Option<String>,
    pub proxy_command: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Host {
    pub alias: String,
    pub source: PathBuf,
    pub line: usize,
    pub hints: HostHints,
    pub metadata: HostMetadata,
}

impl Host {
    pub fn search_score(&self, query: &str) -> Option<i64> {
        [
            self.alias.as_str(),
            self.hints.hostname.as_deref().unwrap_or_default(),
            self.hints.user.as_deref().unwrap_or_default(),
            self.hints.port.as_deref().unwrap_or_default(),
            self.hints.proxy_jump.as_deref().unwrap_or_default(),
            self.metadata.group.as_deref().unwrap_or_default(),
        ]
        .into_iter()
        .chain(self.metadata.tags.iter().map(String::as_str))
        .filter_map(|candidate| fuzzy_score(query, candidate))
        .max()
    }
}

#[derive(Debug, Clone)]
pub struct Catalog {
    config_path: PathBuf,
    hosts: Vec<Host>,
    warnings: Vec<String>,
}

impl Catalog {
    pub fn load(config_path: &Path) -> Result<Self, CatalogError> {
        let identity = current_account_identity().map_err(CatalogError::AccountIdentity)?;
        Self::load_with_identity(config_path, identity)
    }

    #[cfg(test)]
    fn load_with_home(config_path: &Path, home: &Path) -> Result<Self, CatalogError> {
        Self::load_with_identity(config_path, AccountIdentity::for_test(home.to_path_buf()))
    }

    fn load_with_identity(
        config_path: &Path,
        identity: AccountIdentity,
    ) -> Result<Self, CatalogError> {
        let canonical = fs::canonicalize(config_path).map_err(|source| {
            if source.kind() == io::ErrorKind::NotFound {
                CatalogError::MissingConfig {
                    path: config_path.to_path_buf(),
                }
            } else {
                read_error(config_path)(source)
            }
        })?;
        Builder::new(identity).into_catalog(config_path, &canonical)
    }

    pub fn config_path(&self) -> &Path {
        &self.config_path
    }

    pub fn hosts(&self) -> &[Host] {
        &self.hosts
    }

    pub fn warnings(&self) -> &[String] {
        &self.warnings
    }

    pub fn filtered_indices(&self, query: &str) -> Vec<usize> {
        let query = query.trim();
        if query.is_empty() {
            return (0..self.hosts.len()).collect();
        }

        let mut matches: Vec<(usize, i64)> = self
            .hosts
            .iter()
            .enumerate()
            .filter_map(|(index, host)| host.search_score(query).map(|score| (index, score)))
            .collect();
        matches.sort_by_key(|&(index, score)| (std::cmp::Reverse(score), index));
        matches.into_iter().map(|(index, _)| index).collect()
    }

    #[cfg(test)]
    pub(crate) fn from_hosts_for_test(hosts: Vec<Host>) -> Self {
        Self {
            config_path: PathBuf::from("/home/tester/.ssh/config"),
            hosts,
            warnings: Vec::new(),
        }
    }
}

#[derive(Debug, Error)]
pub enum CatalogError {
    #[error("SSH config not found: {path}. Create it or pass --config PATH")]
    MissingConfig { path: PathBuf },

    #[error("cannot read SSH config {path}: {source}")]
    Read { path: PathBuf, source: io::Error },

    #[error("invalid SSH config at {path}:{line}: {message}")]
    Syntax {
        path: PathBuf,
        line: usize,
        message: String,
    },

    #[error("cannot determine the local account identity used by OpenSSH: {0}")]
    AccountIdentity(io::Error),

    #[cfg(unix)]
    #[error(
        "unsafe permissions on included SSH config {path}: owner uid {owner}, mode {mode:#06o}; OpenSSH requires ownership by root or uid {expected_owner} and no group/world write bits"
    )]
    UnsafeInclude {
        path: PathBuf,
        owner: u32,
        expected_owner: u32,
        mode: u32,
    },

    #[error("too many recursive SSH configuration includes while reading {path} (maximum depth: {limit})")]
    IncludeDepth { path: PathBuf, limit: usize },

    #[error("recursive SSH configuration Include cycle through {path}; OpenSSH would exceed its maximum include depth")]
    IncludeCycle { path: PathBuf },
}

fn syntax(path: &Path, line: usize) -> impl Fn(String) -> CatalogError + '_ {
    move |message| CatalogError::Syntax {
        path: path.to_path_buf(),
        line,
        message,
    }
}

fn read_error(path: &Path) -> impl Fn(io::Error) -> CatalogError + '_ {
    move |source| CatalogError::Read {
        path: path.to_path_buf(),
        source,
    }
}

/// Canonicalizes a configuration file being entered, enforcing OpenSSH's
/// include depth limit and rejecting a file that is already being parsed.
fn enter_include(stack: &mut HashSet<PathBuf>, path: &Path) -> Result<PathBuf, CatalogError> {
    if stack.len() > MAX_INCLUDE_DEPTH {
        return Err(CatalogError::IncludeDepth {
            path: path.to_path_buf(),
            limit: MAX_INCLUDE_DEPTH,
        });
    }
    let canonical = fs::canonicalize(path).map_err(read_error(path))?;
    if !stack.insert(canonical.clone()) {
        return Err(CatalogError::IncludeCycle { path: canonical });
    }
    Ok(canonical)
}

fn read_config_contents(
    path: &Path,
    is_include: bool,
    local_uid: u32,
) -> Result<String, CatalogError> {
    let mut file = fs::File::open(path).map_err(read_error(path))?;

    #[cfg(unix)]
    if is_include {
        use std::os::unix::fs::MetadataExt;

        let metadata = file.metadata().map_err(read_error(path))?;
        let owner = metadata.uid();
        let mode = metadata.mode() & 0o7777;
        if (owner != 0 && owner != local_uid) || mode & 0o022 != 0 {
            return Err(CatalogError::UnsafeInclude {
                path: path.to_path_buf(),
                owner,
                expected_owner: local_uid,
                mode,
            });
        }
    }

    #[cfg(not(unix))]
    let _ = (is_include, local_uid);

    let mut contents = String::new();
    file.read_to_string(&mut contents)
        .map_err(read_error(path))?;
    Ok(contents)
}

#[derive(Clone)]
struct AccountIdentity {
    username: String,
    home: PathBuf,
    uid: u32,
}

#[cfg(test)]
impl AccountIdentity {
    fn for_test(home: PathBuf) -> Self {
        Self {
            username: "tester".to_string(),
            home,
            uid: test_uid(),
        }
    }
}

#[cfg(all(test, unix))]
fn test_uid() -> u32 {
    unsafe { libc::getuid() }
}

#[cfg(all(test, not(unix)))]
fn test_uid() -> u32 {
    0
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct IncludeHop {
    source: PathBuf,
    line: usize,
    pattern_index: usize,
    included: PathBuf,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct DeclarationOrigin {
    include_chain: Vec<IncludeHop>,
    source: PathBuf,
    line: usize,
    pattern_index: usize,
}

struct Builder {
    home: PathBuf,
    ssh_dir: PathBuf,
    discovery_root: PathBuf,
    environment: HashMap<String, String>,
    local_hostname: String,
    local_username: String,
    local_uid: u32,
    hosts: Vec<Host>,
    host_origins: Vec<DeclarationOrigin>,
    host_key_aliases: HashMap<usize, String>,
    proxy_options: HashMap<usize, ProxyOptions>,
    include_stack: HashSet<PathBuf>,
    warnings: Vec<String>,
    #[cfg(test)]
    user_homes: HashMap<String, PathBuf>,
}

impl Builder {
    fn new(identity: AccountIdentity) -> Self {
        let environment = unicode_environment();
        let ssh_dir = identity.home.join(".ssh");
        Self {
            discovery_root: ssh_dir.clone(),
            ssh_dir,
            home: identity.home,
            environment,
            local_hostname: system_hostname().unwrap_or_else(|_| "localhost".to_string()),
            local_username: identity.username,
            local_uid: identity.uid,
            hosts: Vec::new(),
            host_origins: Vec::new(),
            host_key_aliases: HashMap::new(),
            proxy_options: HashMap::new(),
            include_stack: HashSet::new(),
            warnings: Vec::new(),
            #[cfg(test)]
            user_homes: HashMap::new(),
        }
    }

    /// Discovers every candidate declaration under `root`, keeps the aliases
    /// an exact per-alias pass proves reachable, and reports them as `config_path`.
    fn into_catalog(mut self, config_path: &Path, root: &Path) -> Result<Catalog, CatalogError> {
        self.parse_file(root, false, &mut Vec::new(), &mut false, &mut Vec::new())?;
        self.retain_reachable_hosts(root)?;
        Ok(Catalog {
            config_path: config_path.to_path_buf(),
            hosts: self.hosts,
            warnings: self.warnings,
        })
    }

    fn parse_file(
        &mut self,
        path: &Path,
        is_include: bool,
        active_hosts: &mut Vec<usize>,
        has_unknown_host: &mut bool,
        include_chain: &mut Vec<IncludeHop>,
    ) -> Result<(), CatalogError> {
        let canonical = enter_include(&mut self.include_stack, path)?;
        if !is_include && include_chain.is_empty() {
            self.discovery_root = canonical
                .parent()
                .unwrap_or_else(|| Path::new("/"))
                .to_path_buf();
        }
        let result = self.parse_file_contents(
            &canonical,
            is_include,
            active_hosts,
            has_unknown_host,
            include_chain,
        );
        self.include_stack.remove(&canonical);
        result
    }

    fn parse_file_contents(
        &mut self,
        path: &Path,
        is_include: bool,
        active_hosts: &mut Vec<usize>,
        has_unknown_host: &mut bool,
        include_chain: &mut Vec<IncludeHop>,
    ) -> Result<(), CatalogError> {
        let contents = read_config_contents(path, is_include, self.local_uid)?;
        let mut defaults = HostMetadata::default();
        // Annotation scope is lexical and file-local, unlike OpenSSH's options:
        // Includes cannot inherit defaults or redirect a parent's annotations.
        let mut metadata_hosts: Option<Vec<usize>> = None;

        for (line_index, raw_line) in contents.lines().enumerate() {
            let line = line_index + 1;
            if let Some(annotation) = metadata::parse(raw_line) {
                match annotation {
                    Ok(annotation) => match &metadata_hosts {
                        None => annotation.apply(&mut defaults),
                        Some(indices) if !indices.is_empty() => {
                            for index in indices {
                                annotation.apply(&mut self.hosts[*index].metadata);
                            }
                        }
                        Some(_) => self.warn_once(format!(
                            "{}:{line}: ignored porteiro annotation: no concrete Host aliases in this block",
                            path.display()
                        )),
                    },
                    Err(error) => self.warn_once(format!(
                        "{}:{line}: ignored porteiro annotation: {error}", path.display()
                    )),
                }
                continue;
            }
            let Some(directive) = parse_directive(raw_line).map_err(syntax(path, line))? else {
                continue;
            };

            let arguments = &directive.arguments;
            match directive.keyword.as_str() {
                "host" => {
                    self.start_host_block(
                        path,
                        line,
                        arguments,
                        active_hosts,
                        has_unknown_host,
                        include_chain,
                    );
                    for index in active_hosts.iter() {
                        self.hosts[*index].metadata.clone_from(&defaults);
                    }
                    metadata_hosts = Some(active_hosts.clone());
                }
                "match" => {
                    metadata_hosts = Some(Vec::new());
                    active_hosts.clear();
                    *has_unknown_host = false;
                }
                "include" => {
                    self.parse_includes(
                        path,
                        line,
                        arguments,
                        active_hosts,
                        has_unknown_host,
                        include_chain,
                    )?;
                }
                "hostname" => {
                    self.set_hint(active_hosts, arguments.first(), |hints| &mut hints.hostname)
                }
                "user" => self.set_hint(active_hosts, arguments.first(), |hints| &mut hints.user),
                "port" => {
                    let port = normalize_port(arguments).map_err(syntax(path, line))?;
                    self.set_hint(active_hosts, Some(&port), |hints| &mut hints.port);
                }
                "canonicalizehostname" => {
                    normalize_canonicalize_hostname(arguments).map_err(syntax(path, line))?;
                }
                "canonicalizemaxdots" => {
                    normalize_canonicalize_max_dots(arguments).map_err(syntax(path, line))?;
                }
                "proxyjump" => {
                    if let Some(raw) = arguments.first() {
                        let host = normalize_proxy_jump(raw).map_err(syntax(path, line))?;
                        self.update_proxy(active_hosts, |proxy| {
                            proxy.set_jump(Truth::Yes, raw, &host)
                        });
                    }
                }
                "proxycommand" => self.update_proxy(active_hosts, |proxy| {
                    proxy.set_command(Truth::Yes, arguments)
                }),
                "hostkeyalias" => {
                    if let Some(value) = arguments.first() {
                        for index in active_hosts.iter() {
                            self.host_key_aliases
                                .entry(*index)
                                .or_insert_with(|| value.clone());
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    fn start_host_block(
        &mut self,
        source: &Path,
        line: usize,
        patterns: &[String],
        active_hosts: &mut Vec<usize>,
        has_unknown_host: &mut bool,
        include_chain: &[IncludeHop],
    ) {
        active_hosts.clear();
        *has_unknown_host = patterns.iter().any(|pattern| {
            !pattern.starts_with('!') && (pattern.contains('*') || pattern.contains('?'))
        });
        let negative_patterns: Vec<&str> = patterns
            .iter()
            .filter_map(|pattern| pattern.strip_prefix('!'))
            .collect();

        for (pattern_index, alias) in patterns
            .iter()
            .enumerate()
            .filter(|(_, pattern)| is_concrete_alias(pattern))
        {
            if negative_patterns
                .iter()
                .any(|pattern| pattern_matches(pattern, alias))
            {
                continue;
            }

            let index = self.hosts.len();
            self.hosts.push(Host {
                alias: alias.clone(),
                source: source.to_path_buf(),
                line,
                hints: HostHints::default(),
                metadata: HostMetadata::default(),
            });
            self.host_origins.push(DeclarationOrigin {
                include_chain: include_chain.to_vec(),
                source: source.to_path_buf(),
                line,
                pattern_index,
            });
            active_hosts.push(index);
        }
    }

    fn parse_includes(
        &mut self,
        source: &Path,
        line: usize,
        patterns: &[String],
        active_hosts: &mut Vec<usize>,
        has_unknown_host: &mut bool,
        include_chain: &mut Vec<IncludeHop>,
    ) -> Result<(), CatalogError> {
        let mut contexts: Vec<Option<usize>> = active_hosts.iter().copied().map(Some).collect();
        if active_hosts.is_empty() || *has_unknown_host {
            contexts.push(None);
        }

        for (pattern_index, raw_pattern) in patterns.iter().enumerate() {
            let paths = self.include_paths(source, line, raw_pattern, &contexts)?;
            let containing_hosts = active_hosts.clone();
            let containing_unknown_host = *has_unknown_host;
            for (path, only_conservative) in paths {
                *active_hosts = containing_hosts.clone();
                *has_unknown_host = containing_unknown_host;
                let result =
                    fs::canonicalize(&path)
                        .map_err(read_error(&path))
                        .and_then(|included| {
                            if only_conservative && self.include_stack.contains(&included) {
                                // The active include chain is already being parsed and
                                // cannot contribute a new declaration by being scanned
                                // again. Exact validation still reports a reachable cycle.
                                return Ok(());
                            }
                            include_chain.push(IncludeHop {
                                source: source.to_path_buf(),
                                line,
                                pattern_index,
                                included: included.clone(),
                            });
                            let result = self.parse_file(
                                &included,
                                true,
                                active_hosts,
                                has_unknown_host,
                                include_chain,
                            );
                            include_chain.pop();
                            result
                        });
                match result {
                    Ok(()) => {}
                    Err(error) if only_conservative => self.warn_once(format!(
                        "cannot parse conservative Include candidate {}: {error}",
                        path.display()
                    )),
                    Err(error) => return Err(error),
                }
            }
            *active_hosts = containing_hosts;
            *has_unknown_host = containing_unknown_host;
        }
        Ok(())
    }

    /// Expands one Include pattern for every active host context and resolves
    /// the resulting globs. Each path maps to whether it was reached only by
    /// a conservative scan for target-dependent tokens.
    fn include_paths(
        &mut self,
        source: &Path,
        line: usize,
        raw_pattern: &str,
        contexts: &[Option<usize>],
    ) -> Result<BTreeMap<PathBuf, bool>, CatalogError> {
        let mut patterns = BTreeMap::new();
        for context in contexts {
            let target = context.map(|index| self.include_target(index));
            let Some(expanded) =
                self.expand_or_warn(source, line, raw_pattern, target.as_ref(), false, "expand")?
            else {
                continue;
            };
            let conservative = expanded.tokens.unknown_target;
            if conservative {
                self.warn_once(format!(
                    "Include {raw_pattern:?} uses target-dependent tokens; scanning matching paths for host discovery"
                ));
            }
            patterns
                .entry(expanded.path)
                .and_modify(|only_conservative| *only_conservative &= conservative)
                .or_insert(conservative);
            if context.is_some() && expanded.tokens.inherited_fallback {
                if let Some(fallback) = self.expand_or_warn(
                    source,
                    line,
                    raw_pattern,
                    None,
                    false,
                    "conservatively expand",
                )? {
                    self.warn_once(format!(
                        "Include {raw_pattern:?} uses target-dependent tokens; scanning matching paths for inherited SSH values"
                    ));
                    patterns.entry(fallback.path).or_insert(true);
                }
            }
        }
        if raw_pattern.contains("%j") {
            if let Some(expanded) = self.expand_or_warn(
                source,
                line,
                raw_pattern,
                None,
                true,
                "conservatively expand empty %j in",
            )? {
                patterns.entry(expanded.path).or_insert(true);
            }
        }

        let mut paths = BTreeMap::new();
        for (expanded, only_conservative) in patterns {
            let pattern = collapse_repeated_stars(&expanded.to_string_lossy());
            let result = if only_conservative {
                let boundaries = [self.discovery_root.as_path(), self.ssh_dir.as_path()];
                conservative_include_matches(&pattern, &boundaries)
            } else {
                exact_include_matches(&pattern)
            };
            let (matches, warnings) = match result {
                Ok(result) => result,
                Err(error) => {
                    // OpenSSH treats malformed glob syntax as an Include
                    // with no matches. Token expansion can legitimately
                    // create that shape, such as `[%n]` for `team/prod`.
                    self.warn_once(format!(
                        "ignoring invalid Include pattern {raw_pattern:?} at {}:{line}: {error}",
                        source.display()
                    ));
                    continue;
                }
            };
            let kind = if only_conservative {
                "conservative Include match"
            } else {
                "Include match"
            };
            for warning in warnings {
                self.warn_once(format!(
                    "cannot inspect {kind} for {raw_pattern:?}: {warning}"
                ));
            }
            for path in matches {
                if only_conservative {
                    paths.entry(path).or_insert(true);
                } else {
                    paths.insert(path, false);
                }
            }
        }
        Ok(paths)
    }

    /// Expands an Include pattern, turning failures OpenSSH ignores into a
    /// warning prefixed by `verb` and invalid token syntax into an error.
    fn expand_or_warn(
        &mut self,
        source: &Path,
        line: usize,
        raw_pattern: &str,
        target: Option<&IncludeTarget>,
        empty_unknown_jump: bool,
        verb: &str,
    ) -> Result<Option<ExpandedInclude>, CatalogError> {
        match self.expand_include(raw_pattern, target, empty_unknown_jump) {
            Ok(expanded) => Ok(Some(expanded)),
            Err(IncludeExpansionError::Ignored(message)) => {
                self.warn_once(format!(
                    "cannot {verb} Include {raw_pattern:?} at {}:{line}: {message}",
                    source.display()
                ));
                Ok(None)
            }
            Err(IncludeExpansionError::Fatal(message)) => Err(syntax(source, line)(message)),
        }
    }

    fn expand_include(
        &self,
        pattern: &str,
        target: Option<&IncludeTarget>,
        empty_unknown_jump: bool,
    ) -> Result<ExpandedInclude, IncludeExpansionError> {
        let environment = expand_environment(pattern, &self.environment)
            .map_err(IncludeExpansionError::Ignored)?;
        let expanded = self
            .expand_include_tokens(&environment, target, empty_unknown_jump)
            .map_err(IncludeExpansionError::Fatal)?;
        let path = self
            .expand_tilde(&expanded.value)
            .map_err(IncludeExpansionError::Ignored)?;
        // OpenSSH resolves relative Includes in a user config from ~/.ssh,
        // not from the directory of the file containing the directive.
        let path = if path.is_absolute() {
            path
        } else {
            self.ssh_dir.join(path)
        };
        Ok(ExpandedInclude {
            path,
            tokens: expanded.tokens,
        })
    }

    fn expand_tilde(&self, pattern: &str) -> Result<PathBuf, String> {
        let Some(remainder) = pattern.strip_prefix('~') else {
            return Ok(PathBuf::from(pattern));
        };
        let split = remainder.find('/').unwrap_or(remainder.len());
        let username = &remainder[..split];
        let suffix = remainder[split..].trim_start_matches('/');
        let user_home = if username.is_empty() || username == self.local_username {
            Some(self.home.clone())
        } else {
            #[cfg(test)]
            if let Some(home) = self.user_homes.get(username) {
                return Ok(home.join(suffix));
            }
            lookup_user_home(username)
                .map_err(|error| format!("cannot look up ~{username}: {error}"))?
        };
        user_home
            .map(|home| home.join(suffix))
            .ok_or_else(|| format!("unknown user in Include path: ~{username}"))
    }

    fn expand_include_tokens(
        &self,
        pattern: &str,
        target: Option<&IncludeTarget>,
        empty_unknown_jump: bool,
    ) -> Result<ExpandedTokens, String> {
        let mut value = String::with_capacity(pattern.len());
        let mut tokens = TokenUse::default();
        let mut characters = pattern.chars();

        while let Some(character) = characters.next() {
            if character != '%' {
                value.push(character);
                continue;
            }
            let token = characters
                .next()
                .ok_or_else(|| "trailing '%' token".to_string())?;
            let local = match token {
                '%' => Some("%".to_string()),
                'd' => Some(self.home.to_string_lossy().into_owned()),
                'i' => Some(self.local_uid.to_string()),
                'L' => Some(
                    self.local_hostname
                        .split('.')
                        .next()
                        .unwrap_or(&self.local_hostname)
                        .to_string(),
                ),
                'l' => Some(self.local_hostname.clone()),
                'u' => Some(self.local_username.clone()),
                'C' | 'h' | 'j' | 'k' | 'n' | 'p' | 'r' => None,
                _ => return Err(format!("unsupported Include token %{token}")),
            };
            if let Some(local) = local {
                value.push_str(&local);
                continue;
            }
            // %n is always the original command-line host. The remaining
            // target values may be inherited from broader Host blocks and
            // therefore need a conservative discovery fallback.
            tokens.inherited_fallback |= token != 'n';
            let replacement = match target {
                None => {
                    tokens.unknown_target = true;
                    if token == 'j' && empty_unknown_jump {
                        String::new()
                    } else {
                        "*".to_string()
                    }
                }
                Some(target) => {
                    let known = match token {
                        'C' => connection_hash(&self.local_hostname, target),
                        'h' => target.hostname.clone(),
                        'j' => target.jump_host.clone(),
                        'k' => target.key_alias.clone(),
                        'n' => Some(target.original_host.clone()),
                        'p' => target.port.clone(),
                        _ => target.remote_user.clone(),
                    };
                    known.unwrap_or_else(|| {
                        tokens.uncertain_target = true;
                        "*".to_string()
                    })
                }
            };
            value.push_str(&replacement);
        }

        Ok(ExpandedTokens { value, tokens })
    }

    fn include_target(&self, index: usize) -> IncludeTarget {
        let host = &self.hosts[index];
        let hostname = host
            .hints
            .hostname
            .as_deref()
            .map(|hostname| expand_hostname(hostname, &host.alias))
            .unwrap_or_else(|| host.alias.clone());
        let jump_host = self
            .proxy_options
            .get(&index)
            .map_or_else(|| Some(String::new()), ProxyOptions::jump_host)
            .unwrap_or_else(|| "*".to_string());
        IncludeTarget {
            original_host: host.alias.clone(),
            key_alias: Some(
                self.host_key_aliases
                    .get(&index)
                    .cloned()
                    .unwrap_or_else(|| hostname.clone()),
            ),
            hostname: Some(hostname),
            port: Some(host.hints.port.clone().unwrap_or_else(|| "22".to_string())),
            remote_user: Some(
                host.hints
                    .user
                    .clone()
                    .unwrap_or_else(|| self.local_username.clone()),
            ),
            jump_host: Some(jump_host),
        }
    }

    fn update_proxy(&mut self, active_hosts: &[usize], update: impl Fn(&mut ProxyOptions)) {
        for index in active_hosts {
            let proxy = self.proxy_options.entry(*index).or_default();
            update(proxy);
            let hints = &mut self.hosts[*index].hints;
            hints.proxy_jump = proxy.jump_hint();
            hints.proxy_command = proxy.command_hint();
        }
    }

    /// Records the first value of a declaration hint for every active host.
    fn set_hint(
        &mut self,
        active_hosts: &[usize],
        value: Option<&String>,
        slot: fn(&mut HostHints) -> &mut Option<String>,
    ) {
        if let Some(value) = value {
            for index in active_hosts {
                slot(&mut self.hosts[*index].hints).get_or_insert_with(|| value.clone());
            }
        }
    }

    fn warn_once(&mut self, warning: String) {
        if !self.warnings.contains(&warning) {
            self.warnings.push(warning);
        }
    }

    fn retain_reachable_hosts(&mut self, root: &Path) -> Result<(), CatalogError> {
        let candidates = std::mem::take(&mut self.hosts);
        let origins = std::mem::take(&mut self.host_origins);
        let mut reachable = Vec::with_capacity(candidates.len());
        let mut validations: HashMap<String, Option<ValidatedHost>> = HashMap::new();
        let mut seen_folded = HashSet::new();

        for (mut host, origin) in candidates.into_iter().zip(origins) {
            if !validations.contains_key(&host.alias) {
                let validation = AliasValidator::new(self, &host.alias).validate(root)?;
                validations.insert(host.alias.clone(), validation);
            }
            let Some(validation) = validations.get(&host.alias).cloned().flatten() else {
                continue;
            };
            if validation.origin != origin {
                continue;
            }
            if !seen_folded.insert(host.alias.to_lowercase()) {
                continue;
            }
            host.source = validation.source;
            host.line = validation.line;
            host.hints = validation.hints;
            reachable.push(host);
        }
        self.hosts = reachable;
        self.host_key_aliases.clear();
        self.proxy_options.clear();
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Truth {
    No,
    Yes,
    Unknown,
}

impl Truth {
    fn and(self, other: Self) -> Self {
        match (self, other) {
            (Self::No, _) | (_, Self::No) => Self::No,
            (Self::Yes, Self::Yes) => Self::Yes,
            _ => Self::Unknown,
        }
    }

    fn not(self) -> Self {
        match self {
            Self::No => Self::Yes,
            Self::Yes => Self::No,
            Self::Unknown => Self::Unknown,
        }
    }
}

impl From<bool> for Truth {
    fn from(value: bool) -> Self {
        if value {
            Self::Yes
        } else {
            Self::No
        }
    }
}

#[derive(Clone, Debug, Default)]
enum FirstValue {
    #[default]
    Unset,
    Known(String),
    Unknown,
}

impl FirstValue {
    fn set(&mut self, active: Truth, arguments: &[String]) {
        if let Some(value) = arguments.first() {
            self.set_value(active, value.clone());
        }
    }

    fn set_value(&mut self, active: Truth, value: String) {
        if !matches!(self, Self::Unset) {
            return;
        }
        match active {
            Truth::No => {}
            Truth::Yes => *self = Self::Known(value),
            Truth::Unknown => *self = Self::Unknown,
        }
    }

    fn known(&self) -> Option<&str> {
        match self {
            Self::Known(value) => Some(value),
            Self::Unset | Self::Unknown => None,
        }
    }

    fn resolve_or(&self, fallback: impl FnOnce() -> String) -> Option<String> {
        match self {
            Self::Unset => Some(fallback()),
            Self::Known(value) => Some(value.clone()),
            Self::Unknown => None,
        }
    }

    fn taint_if_unset(&mut self) {
        if matches!(self, Self::Unset) {
            *self = Self::Unknown;
        }
    }
}

#[derive(Clone, Debug, Default)]
enum JumpState {
    #[default]
    Unset,
    Known {
        raw: String,
        host: String,
    },
    Unknown,
}

#[derive(Clone, Debug, Default)]
struct ProxyOptions {
    jump: JumpState,
    command: FirstValue,
}

impl ProxyOptions {
    fn set_jump(&mut self, active: Truth, raw: &str, host: &str) {
        if active == Truth::No {
            return;
        }
        if matches!(self.jump, JumpState::Known { .. })
            || matches!(self.command, FirstValue::Known(_))
        {
            return;
        }
        if matches!(self.jump, JumpState::Unknown) || matches!(self.command, FirstValue::Unknown) {
            self.jump = JumpState::Unknown;
            return;
        }

        if active == Truth::Yes {
            self.jump = JumpState::Known {
                raw: raw.to_string(),
                host: host.to_string(),
            };
            if !host.is_empty() {
                self.command = FirstValue::Known("none".to_string());
            }
        } else {
            self.jump = JumpState::Unknown;
            if !host.is_empty() {
                self.command = FirstValue::Unknown;
            }
        }
    }

    fn set_command(&mut self, active: Truth, arguments: &[String]) {
        if !arguments.is_empty() {
            self.command.set_value(active, arguments.join(" "));
        }
    }

    fn jump_host(&self) -> Option<String> {
        match &self.jump {
            JumpState::Unset => Some(String::new()),
            JumpState::Known { host, .. } => Some(host.clone()),
            JumpState::Unknown => None,
        }
    }

    fn jump_hint(&self) -> Option<String> {
        match &self.jump {
            JumpState::Known { raw, host } if !host.is_empty() => Some(raw.clone()),
            JumpState::Unset | JumpState::Known { .. } | JumpState::Unknown => None,
        }
    }

    fn command_hint(&self) -> Option<String> {
        self.command
            .known()
            .filter(|command| !command.eq_ignore_ascii_case("none"))
            .map(str::to_string)
    }

    fn directness(&self) -> Truth {
        let command_clear = match &self.command {
            FirstValue::Unset => Truth::Yes,
            FirstValue::Known(command) if command.eq_ignore_ascii_case("none") => Truth::Yes,
            FirstValue::Known(_) => Truth::No,
            FirstValue::Unknown => Truth::Unknown,
        };
        let jump_clear = match &self.jump {
            JumpState::Unset => Truth::Yes,
            JumpState::Known { host, .. } if host.is_empty() => Truth::Yes,
            JumpState::Known { .. } => Truth::No,
            JumpState::Unknown => Truth::Unknown,
        };
        command_clear.and(jump_clear)
    }

    fn taint_if_unset(&mut self) {
        if matches!(self.jump, JumpState::Unset) {
            self.jump = JumpState::Unknown;
        }
        self.command.taint_if_unset();
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Canonicalization {
    No,
    Yes,
    Always,
    Unknown,
}

#[derive(Clone, Debug, Default)]
struct TargetOptions {
    hostname: FirstValue,
    user: FirstValue,
    port: FirstValue,
    proxy: ProxyOptions,
    host_key_alias: FirstValue,
    canonicalize_hostname: FirstValue,
    canonicalize_max_dots: FirstValue,
}

impl TargetOptions {
    fn taint_unset(&mut self) {
        self.hostname.taint_if_unset();
        self.user.taint_if_unset();
        self.port.taint_if_unset();
        self.proxy.taint_if_unset();
        self.host_key_alias.taint_if_unset();
        self.canonicalize_hostname.taint_if_unset();
        self.canonicalize_max_dots.taint_if_unset();
    }

    fn canonicalization(&self) -> Canonicalization {
        match &self.canonicalize_hostname {
            FirstValue::Unset => Canonicalization::No,
            FirstValue::Known(value) if value == "no" => Canonicalization::No,
            FirstValue::Known(value) if value == "yes" => Canonicalization::Yes,
            FirstValue::Known(_) => Canonicalization::Always,
            FirstValue::Unknown => Canonicalization::Unknown,
        }
    }

    fn canonicalize_max_dots(&self) -> Option<usize> {
        match &self.canonicalize_max_dots {
            FirstValue::Unset => Some(1),
            FirstValue::Known(value) => value.parse().ok(),
            FirstValue::Unknown => None,
        }
    }

    fn canonicalization_may_use_dns(
        &self,
        canonicalization: Canonicalization,
        hostname: &str,
    ) -> bool {
        match canonicalization {
            Canonicalization::No => return false,
            Canonicalization::Yes if self.proxy.directness() == Truth::No => return false,
            Canonicalization::Yes | Canonicalization::Always | Canonicalization::Unknown => {}
        }

        // OpenSSH resolves anchored names before applying CanonicalizeMaxDots.
        if hostname.ends_with('.') {
            return true;
        }

        let dots = hostname.bytes().filter(|byte| *byte == b'.').count();
        self.canonicalize_max_dots()
            .is_none_or(|max_dots| dots <= max_dots)
    }

    fn hints(&self, alias: &str) -> HostHints {
        HostHints {
            hostname: self
                .hostname
                .known()
                .map(|hostname| expand_hostname(hostname, alias)),
            user: self.user.known().map(str::to_string),
            port: self.port.known().map(str::to_string),
            proxy_jump: self.proxy.jump_hint(),
            proxy_command: self.proxy.command_hint(),
        }
    }
}

#[derive(Clone)]
struct ValidatedHost {
    source: PathBuf,
    line: usize,
    hints: HostHints,
    origin: DeclarationOrigin,
}

struct AliasValidator<'a> {
    context: &'a Builder,
    alias: &'a str,
    options: TargetOptions,
    declaration: Option<DeclarationOrigin>,
    include_stack: HashSet<PathBuf>,
    final_pass: bool,
    wants_final_pass: bool,
}

impl<'a> AliasValidator<'a> {
    fn new(context: &'a Builder, alias: &'a str) -> Self {
        Self {
            context,
            alias,
            options: TargetOptions::default(),
            declaration: None,
            include_stack: HashSet::new(),
            final_pass: false,
            wants_final_pass: false,
        }
    }

    fn validate(mut self, root: &Path) -> Result<Option<ValidatedHost>, CatalogError> {
        let mut active = Truth::Yes;
        let mut include_chain = Vec::new();
        self.parse_file(root, false, Truth::Yes, &mut active, &mut include_chain)?;
        let canonicalization = self.options.canonicalization();
        if self.wants_final_pass
            || matches!(
                canonicalization,
                Canonicalization::Yes | Canonicalization::Always
            )
        {
            self.final_pass = true;
            let final_hostname = self
                .options
                .hostname
                .resolve_or(|| self.alias.to_string())
                .map(|hostname| expand_hostname(&hostname, self.alias))
                .and_then(|hostname| {
                    let numeric_host = normalize_numeric_host(&hostname);
                    let needs_dns = numeric_host.is_none()
                        && self
                            .options
                            .canonicalization_may_use_dns(canonicalization, &hostname);
                    (!needs_dns).then(|| normalize_final_host(hostname))
                });
            self.options.hostname = final_hostname
                .map(FirstValue::Known)
                .unwrap_or(FirstValue::Unknown);
            self.include_stack.clear();
            active = Truth::Yes;
            include_chain.clear();
            self.parse_file(root, false, Truth::Yes, &mut active, &mut include_chain)?;
        }

        Ok(self.declaration.map(|origin| ValidatedHost {
            source: origin.source.clone(),
            line: origin.line,
            hints: self.options.hints(self.alias),
            origin,
        }))
    }

    fn parse_file(
        &mut self,
        path: &Path,
        is_include: bool,
        can_activate: Truth,
        active: &mut Truth,
        include_chain: &mut Vec<IncludeHop>,
    ) -> Result<(), CatalogError> {
        let canonical = enter_include(&mut self.include_stack, path)?;
        let result =
            self.parse_file_contents(&canonical, is_include, can_activate, active, include_chain);
        self.include_stack.remove(&canonical);
        result
    }

    fn parse_file_contents(
        &mut self,
        path: &Path,
        is_include: bool,
        can_activate: Truth,
        active: &mut Truth,
        include_chain: &mut Vec<IncludeHop>,
    ) -> Result<(), CatalogError> {
        let contents = read_config_contents(path, is_include, self.context.local_uid)?;

        for (line_index, raw_line) in contents.lines().enumerate() {
            let line = line_index + 1;
            let Some(directive) = parse_directive(raw_line).map_err(syntax(path, line))? else {
                continue;
            };

            match directive.keyword.as_str() {
                "host" => {
                    let match_host = if self.final_pass {
                        self.options.hostname.known()
                    } else {
                        Some(self.alias)
                    };
                    let host_matches = match_host.map_or(Truth::Unknown, |host| {
                        host_block_matches(&directive.arguments, host).into()
                    });
                    *active = can_activate.and(host_matches);
                    if *active == Truth::Yes && self.declaration.is_none() {
                        if let Some(pattern_index) =
                            declared_alias_index(&directive.arguments, self.alias)
                        {
                            self.declaration = Some(DeclarationOrigin {
                                include_chain: include_chain.clone(),
                                source: path.to_path_buf(),
                                line,
                                pattern_index,
                            });
                        }
                    }
                }
                "match" => {
                    *active = can_activate.and(self.match_conditions(&directive.arguments));
                }
                "include" => {
                    self.parse_includes(path, line, &directive.arguments, active, include_chain)?;
                }
                "hostname" => self.options.hostname.set(*active, &directive.arguments),
                "user" => self.options.user.set(*active, &directive.arguments),
                "port" => {
                    let port = normalize_port(&directive.arguments).map_err(syntax(path, line))?;
                    self.options.port.set_value(*active, port);
                }
                "canonicalizehostname" => {
                    let value = normalize_canonicalize_hostname(&directive.arguments)
                        .map_err(syntax(path, line))?;
                    self.options.canonicalize_hostname.set_value(*active, value);
                }
                "canonicalizemaxdots" => {
                    let value = normalize_canonicalize_max_dots(&directive.arguments)
                        .map_err(syntax(path, line))?;
                    self.options.canonicalize_max_dots.set_value(*active, value);
                }
                "proxyjump" => {
                    if let Some(raw) = directive.arguments.first() {
                        let host = normalize_proxy_jump(raw).map_err(syntax(path, line))?;
                        self.options.proxy.set_jump(*active, raw, &host);
                    }
                }
                "proxycommand" => self
                    .options
                    .proxy
                    .set_command(*active, &directive.arguments),
                "hostkeyalias" => self
                    .options
                    .host_key_alias
                    .set(*active, &directive.arguments),
                _ => {}
            }
        }
        Ok(())
    }

    fn parse_includes(
        &mut self,
        source: &Path,
        line: usize,
        patterns: &[String],
        active: &mut Truth,
        include_chain: &mut Vec<IncludeHop>,
    ) -> Result<(), CatalogError> {
        let containing_active = *active;
        for (pattern_index, raw_pattern) in patterns.iter().enumerate() {
            let target = self.include_target();
            let expanded = match self
                .context
                .expand_include(raw_pattern, Some(&target), false)
            {
                Ok(expanded) => expanded,
                Err(IncludeExpansionError::Ignored(_)) => continue,
                Err(IncludeExpansionError::Fatal(message)) => {
                    return Err(syntax(source, line)(message));
                }
            };
            if expanded.tokens.uncertain_target {
                if containing_active != Truth::No {
                    self.options.taint_unset();
                }
                continue;
            }
            // libc glob, and therefore OpenSSH, treats malformed glob syntax
            // as no match and ignores directory-iteration failures. A path the
            // glob does return remains exact, so its later canonicalization or
            // read failure is still fatal.
            let pattern = collapse_repeated_stars(&expanded.path.to_string_lossy());
            let Ok((mut paths, _)) = exact_include_matches(&pattern) else {
                continue;
            };
            paths.sort();
            paths.dedup();
            for path in paths {
                let included = fs::canonicalize(&path).map_err(read_error(&path))?;
                include_chain.push(IncludeHop {
                    source: source.to_path_buf(),
                    line,
                    pattern_index,
                    included: included.clone(),
                });
                let mut included_active = containing_active;
                let result = self.parse_file(
                    &included,
                    true,
                    containing_active,
                    &mut included_active,
                    include_chain,
                );
                include_chain.pop();
                result?;
                *active = containing_active;
            }
        }
        Ok(())
    }

    fn include_target(&self) -> IncludeTarget {
        let hostname = self
            .options
            .hostname
            .resolve_or(|| self.alias.to_string())
            .map(|hostname| expand_hostname(&hostname, self.alias));
        let key_alias = match &self.options.host_key_alias {
            FirstValue::Unset => hostname.clone(),
            FirstValue::Known(value) => Some(value.clone()),
            FirstValue::Unknown => None,
        };
        IncludeTarget {
            original_host: self.alias.to_string(),
            key_alias,
            hostname,
            port: self.options.port.resolve_or(|| "22".to_string()),
            remote_user: self
                .options
                .user
                .resolve_or(|| self.context.local_username.clone()),
            jump_host: self.options.proxy.jump_host(),
        }
    }

    fn match_conditions(&mut self, arguments: &[String]) -> Truth {
        let target = self.include_target();
        let mut index = 0;
        let mut result = Truth::Yes;
        while index < arguments.len() {
            let raw_criterion = &arguments[index];
            index += 1;
            let (criterion, attached) = raw_criterion
                .split_once('=')
                .map_or((raw_criterion.as_str(), None), |(name, value)| {
                    (name, Some(value))
                });
            let (negated, criterion) = criterion
                .strip_prefix('!')
                .map_or((false, criterion), |criterion| (true, criterion));
            let criterion = criterion.to_ascii_lowercase();

            let matched = match criterion.as_str() {
                "all" => Truth::Yes,
                "canonical" => self.final_pass.into(),
                "final" => {
                    if !negated {
                        self.wants_final_pass = true;
                    }
                    self.final_pass.into()
                }
                "host" | "originalhost" | "user" | "localuser" => {
                    let value = match attached {
                        Some(value) => value,
                        None => {
                            let Some(value) = arguments.get(index) else {
                                return Truth::No;
                            };
                            index += 1;
                            value
                        }
                    };
                    let candidate = match criterion.as_str() {
                        "host" => target.hostname.as_deref(),
                        "originalhost" => Some(self.alias),
                        "user" => target.remote_user.as_deref(),
                        "localuser" => Some(self.context.local_username.as_str()),
                        _ => unreachable!(),
                    };
                    candidate.map_or(Truth::Unknown, |candidate| {
                        pattern_list_matches(
                            value,
                            candidate,
                            matches!(criterion.as_str(), "host" | "originalhost"),
                        )
                        .into()
                    })
                }
                // Evaluating these can execute user commands, inspect mutable
                // machine state, or depend on connection-only context. Keep
                // their outcome unknown so later first-value options and
                // target-dependent Includes cannot be treated as proven.
                "exec" | "localnetwork" | "version" | "tagged" | "command" | "sessiontype" => {
                    if attached.is_none() {
                        let Some(_value) = arguments.get(index) else {
                            return Truth::No;
                        };
                        index += 1;
                    }
                    Truth::Unknown
                }
                _ => return Truth::No,
            };
            result = result.and(if negated { matched.not() } else { matched });
        }
        result
    }
}

/// How an Include pattern used connection-target tokens.
#[derive(Clone, Copy, Default)]
struct TokenUse {
    /// A target token was expanded without a target, so `*` stands in.
    unknown_target: bool,
    /// A token may inherit its value from broader Host blocks and needs a
    /// conservative fallback scan.
    inherited_fallback: bool,
    /// A target value was unknown because of stateful Match criteria.
    uncertain_target: bool,
}

struct ExpandedInclude {
    path: PathBuf,
    tokens: TokenUse,
}

struct ExpandedTokens {
    value: String,
    tokens: TokenUse,
}

#[derive(Debug)]
enum IncludeExpansionError {
    // OpenSSH reports these expansion failures but otherwise continues with
    // the Include omitted, such as an unset ${ENVIRONMENT_VARIABLE}.
    Ignored(String),
    // Invalid percent syntax aborts OpenSSH configuration processing.
    Fatal(String),
}

struct IncludeTarget {
    hostname: Option<String>,
    original_host: String,
    remote_user: Option<String>,
    port: Option<String>,
    jump_host: Option<String>,
    key_alias: Option<String>,
}

fn connection_hash(local_hostname: &str, target: &IncludeTarget) -> Option<String> {
    use std::fmt::Write;

    let mut digest = Sha1::new();
    for value in [
        local_hostname,
        target.hostname.as_deref()?,
        target.port.as_deref()?,
        target.remote_user.as_deref()?,
        target.jump_host.as_deref()?,
    ] {
        digest.update(value.as_bytes());
    }
    let digest = digest.finalize();
    let mut hexadecimal = String::with_capacity(digest.len() * 2);
    for byte in digest {
        write!(&mut hexadecimal, "{byte:02x}").expect("writing to a String cannot fail");
    }
    Some(hexadecimal)
}

fn expand_hostname(value: &str, original_host: &str) -> String {
    let mut expanded = String::with_capacity(value.len());
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '%' {
            expanded.push(character);
            continue;
        }
        match characters.next() {
            Some('%') => expanded.push('%'),
            Some('h') => expanded.push_str(original_host),
            Some(token) => {
                expanded.push('%');
                expanded.push(token);
            }
            None => expanded.push('%'),
        }
    }
    expanded
}

fn normalize_final_host(host: String) -> String {
    normalize_numeric_host(&host).unwrap_or_else(|| host.to_ascii_lowercase())
}

#[cfg(unix)]
fn normalize_numeric_host(host: &str) -> Option<String> {
    let node = CString::new(host).ok()?;
    let mut output = [0 as libc::c_char; 1_025];
    let output_length = libc::socklen_t::try_from(output.len()).ok()?;
    let mut hints = unsafe { std::mem::zeroed::<libc::addrinfo>() };
    hints.ai_family = libc::AF_UNSPEC;
    hints.ai_socktype = libc::SOCK_STREAM;
    hints.ai_flags = libc::AI_NUMERICHOST;

    let mut addresses = std::ptr::null_mut();
    let status =
        unsafe { libc::getaddrinfo(node.as_ptr(), std::ptr::null(), &hints, &mut addresses) };
    if status != 0 || addresses.is_null() {
        return None;
    }

    let status = unsafe {
        libc::getnameinfo(
            (*addresses).ai_addr,
            (*addresses).ai_addrlen,
            output.as_mut_ptr(),
            output_length,
            std::ptr::null_mut(),
            0,
            libc::NI_NUMERICHOST,
        )
    };
    let normalized = if status == 0 {
        unsafe { CStr::from_ptr(output.as_ptr()) }
            .to_str()
            .ok()
            .map(str::to_string)
    } else {
        None
    };
    unsafe { libc::freeaddrinfo(addresses) };
    normalized
}

#[cfg(not(unix))]
fn normalize_numeric_host(host: &str) -> Option<String> {
    host.parse::<std::net::IpAddr>()
        .ok()
        .map(|address| address.to_string())
}

/// Parses the decimal spelling OpenSSH accepts: an optional `+` and digits.
fn parse_decimal(value: &str) -> Option<u32> {
    let digits = value.strip_prefix('+').unwrap_or(value);
    let numeric = !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit());
    numeric.then(|| digits.parse().ok()).flatten()
}

fn normalize_port(arguments: &[String]) -> Result<String, String> {
    let value = arguments
        .first()
        .ok_or_else(|| "Port requires a value".to_string())?;
    parse_decimal(value)
        .filter(|port| (1..=u32::from(u16::MAX)).contains(port))
        .map(|port| port.to_string())
        .ok_or_else(|| format!("invalid Port {value:?}"))
}

fn normalize_canonicalize_hostname(arguments: &[String]) -> Result<String, String> {
    let Some(value) = arguments.first() else {
        return Err("CanonicalizeHostname requires a value".to_string());
    };
    match value.to_ascii_lowercase().as_str() {
        "yes" | "true" => Ok("yes".to_string()),
        "no" | "false" => Ok("no".to_string()),
        "always" => Ok("always".to_string()),
        _ => Err(format!("invalid CanonicalizeHostname {value:?}")),
    }
}

fn normalize_canonicalize_max_dots(arguments: &[String]) -> Result<String, String> {
    let value = arguments
        .first()
        .ok_or_else(|| "CanonicalizeMaxDots requires a value".to_string())?;
    parse_decimal(value)
        .filter(|max_dots| i32::try_from(*max_dots).is_ok())
        .map(|max_dots| max_dots.to_string())
        .ok_or_else(|| format!("invalid CanonicalizeMaxDots {value:?}"))
}

fn normalize_proxy_jump(value: &str) -> Result<String, String> {
    let value = value.trim();
    if value.eq_ignore_ascii_case("none") {
        return Ok(String::new());
    }
    if value.is_empty() {
        return Err("ProxyJump requires a destination".to_string());
    }

    let mut final_host = None;
    for hop in value.split(',') {
        final_host = Some(parse_proxy_jump_hop(hop.trim())?);
    }
    final_host.ok_or_else(|| "ProxyJump requires a destination".to_string())
}

fn parse_proxy_jump_hop(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("ProxyJump contains an empty hop".to_string());
    }
    let authority = if let Some(authority) = value.strip_prefix("ssh://") {
        if authority.contains('/') || authority.contains('?') || authority.contains('#') {
            return Err(format!("invalid ProxyJump URI {value:?}"));
        }
        authority
    } else {
        value
    };
    let host_port = match authority.rsplit_once('@') {
        Some((user, host_port)) if !user.is_empty() && !host_port.is_empty() => host_port,
        Some(_) => return Err(format!("invalid ProxyJump destination {value:?}")),
        None => authority,
    };

    let host = if let Some(bracketed) = host_port.strip_prefix('[') {
        let Some(close) = bracketed.find(']') else {
            return Err(format!("invalid bracketed ProxyJump host {value:?}"));
        };
        let host = &bracketed[..close];
        let remainder = &bracketed[close + 1..];
        if let Some(port) = remainder.strip_prefix(':') {
            validate_proxy_jump_port(port, value)?;
        } else if !remainder.is_empty() {
            return Err(format!("invalid ProxyJump destination {value:?}"));
        }
        host
    } else {
        match host_port.matches(':').count() {
            0 => host_port,
            1 => {
                let (host, port) = host_port
                    .rsplit_once(':')
                    .expect("one colon must have a split point");
                validate_proxy_jump_port(port, value)?;
                host
            }
            _ => {
                return Err(format!(
                    "IPv6 ProxyJump hosts must be enclosed in brackets: {value:?}"
                ));
            }
        }
    };

    if host.is_empty() || host.chars().any(char::is_whitespace) {
        return Err(format!("invalid ProxyJump host in {value:?}"));
    }
    Ok(host.to_string())
}

fn validate_proxy_jump_port(port: &str, value: &str) -> Result<(), String> {
    let valid = port.parse::<u16>().ok().is_some_and(|port| port != 0);
    if valid {
        Ok(())
    } else {
        Err(format!("invalid ProxyJump port in {value:?}"))
    }
}

fn expand_environment(
    value: &str,
    environment: &HashMap<String, String>,
) -> Result<String, String> {
    let mut expanded = String::with_capacity(value.len());
    let mut remainder = value;
    while let Some(start) = remainder.find("${") {
        expanded.push_str(&remainder[..start]);
        let variable_start = start + 2;
        let after_start = &remainder[variable_start..];
        let end = after_start
            .find('}')
            .ok_or_else(|| "unterminated environment variable".to_string())?;
        let name = &after_start[..end];
        if name.is_empty() {
            return Err("empty environment variable name".to_string());
        }
        let replacement = environment
            .get(name)
            .ok_or_else(|| format!("environment variable {name} is not set"))?;
        // percent_dollar_expand() does not recursively treat '%' bytes from
        // an environment value as tokens. Doubling them preserves that
        // one-pass OpenSSH behavior for the token expansion below.
        expanded.push_str(&replacement.replace('%', "%%"));
        remainder = &after_start[end + 1..];
    }
    expanded.push_str(remainder);
    Ok(expanded)
}

/// Expands an Include glob the way OpenSSH does: separators and leading dots
/// must be literal. Directory-iteration failures are reported, not fatal.
fn exact_include_matches(pattern: &str) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: true,
        require_literal_leading_dot: true,
    };
    let entries = glob_with(pattern, options).map_err(|error| error.to_string())?;
    let (mut paths, mut warnings) = (Vec::new(), Vec::new());
    for entry in entries {
        match entry {
            Ok(path) => paths.push(path),
            Err(error) => warnings.push(error.to_string()),
        }
    }
    Ok((paths, warnings))
}

fn collapse_repeated_stars(pattern: &str) -> String {
    let mut normalized = String::with_capacity(pattern.len());
    let mut previous_was_star = false;
    for character in pattern.chars() {
        if character == '*' {
            if !previous_was_star {
                normalized.push(character);
            }
            previous_was_star = true;
        } else {
            normalized.push(character);
            previous_was_star = false;
        }
    }
    normalized
}

fn conservative_include_matches(
    pattern: &str,
    scan_boundaries: &[&Path],
) -> Result<(Vec<PathBuf>, Vec<String>), String> {
    let normalized_pattern: PathBuf = Path::new(pattern).components().collect();
    let mut widened_pattern = PathBuf::new();
    for component in normalized_pattern.components() {
        let value = component.as_os_str().to_string_lossy();
        if value.contains('*') {
            // Target-token fallbacks use `*`. Widen the complete component so
            // surrounding glob syntax (for example `[%n]`) cannot capture the
            // fallback and reinterpret it as a literal character.
            widened_pattern.push("*");
        } else {
            widened_pattern.push(component.as_os_str());
        }
    }
    let matcher =
        Pattern::new(&widened_pattern.to_string_lossy()).map_err(|error| error.to_string())?;
    let mut literal_root = PathBuf::new();
    for component in widened_pattern.components() {
        let value = component.as_os_str().to_string_lossy();
        if value
            .chars()
            .any(|character| matches!(character, '*' | '?' | '['))
        {
            break;
        }
        literal_root.push(component.as_os_str());
    }
    if literal_root.as_os_str().is_empty() {
        literal_root.push(".");
    }

    // A target token can contain separators and parent components. Scan from
    // the containing configuration tree, then present each real path to the
    // matcher through every lexical `..` spelling back to the literal prefix.
    // This discovers `by/../outside.conf` without recursively walking an
    // entire filesystem for every target-dependent Include.
    let scan_root = scan_boundaries
        .iter()
        .copied()
        .filter(|boundary| literal_root.starts_with(boundary))
        .min_by_key(|boundary| boundary.components().count())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| literal_root.parent().unwrap_or(&literal_root).to_path_buf());
    let mut lexical_bases = Vec::new();
    let mut ancestor = literal_root.clone();
    let mut lexical_base = literal_root;
    loop {
        lexical_bases.push((ancestor.clone(), lexical_base.clone()));
        if ancestor == scan_root {
            break;
        }
        let Some(parent) = ancestor.parent() else {
            break;
        };
        if !parent.starts_with(&scan_root) {
            break;
        }
        ancestor = parent.to_path_buf();
        lexical_base.push("..");
    }

    let options = MatchOptions {
        case_sensitive: true,
        require_literal_separator: false,
        require_literal_leading_dot: false,
    };
    let mut pending = vec![(scan_root, Vec::<PathBuf>::new())];
    let mut matches = Vec::new();
    let mut warnings = Vec::new();

    while let Some((path, ancestors)) = pending.pop() {
        let path_matches = lexical_bases.iter().any(|(ancestor, lexical_base)| {
            let Ok(relative) = path.strip_prefix(ancestor) else {
                return false;
            };
            matcher.matches_path_with(&lexical_base.join(relative), options)
        });

        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                if path_matches {
                    // A dangling symlink is still a glob match. Preserve it so
                    // the caller can downgrade its canonicalization failure.
                    matches.push(path);
                }
                continue;
            }
            Err(error) => {
                warnings.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        if !metadata.is_dir() {
            if path_matches {
                matches.push(path);
            }
            continue;
        }

        let canonical = match fs::canonicalize(&path) {
            Ok(canonical) => canonical,
            Err(error) => {
                warnings.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        if ancestors.contains(&canonical) {
            continue;
        }
        let mut child_ancestors = ancestors;
        child_ancestors.push(canonical);

        let entries = match fs::read_dir(&path) {
            Ok(entries) => entries,
            Err(error) => {
                warnings.push(format!("{}: {error}", path.display()));
                continue;
            }
        };
        let mut children = Vec::new();
        for entry in entries {
            match entry {
                Ok(entry) => children.push(entry.path()),
                Err(error) => warnings.push(format!("{}: {error}", path.display())),
            }
        }
        children.sort_by(|left, right| right.cmp(left));
        pending.extend(
            children
                .into_iter()
                .map(|child| (child, child_ancestors.clone())),
        );
    }

    matches.sort();
    matches.dedup();
    warnings.sort();
    warnings.dedup();
    Ok((matches, warnings))
}

fn unicode_environment() -> HashMap<String, String> {
    std::env::vars_os()
        .filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

#[cfg(unix)]
fn current_account_identity() -> io::Result<AccountIdentity> {
    let uid = unsafe { libc::getuid() };
    let (username, home) = passwd_entry(|entry, buffer, capacity, result| unsafe {
        libc::getpwuid_r(uid, entry, buffer, capacity, result)
    })?
    .ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::NotFound,
            format!("no passwd entry for uid {uid}"),
        )
    })?;
    Ok(AccountIdentity {
        username,
        home,
        uid,
    })
}

#[cfg(not(unix))]
fn current_account_identity() -> io::Result<AccountIdentity> {
    let environment = unicode_environment();
    let username = environment
        .get("USERNAME")
        .or_else(|| environment.get("USER"))
        .cloned()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "username is unavailable"))?;
    let home = std::env::home_dir()
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "home directory is unavailable"))?;
    Ok(AccountIdentity {
        username,
        home,
        uid: 0,
    })
}

#[cfg(unix)]
fn system_hostname() -> io::Result<String> {
    let mut buffer = vec![0_u8; 1_024];
    let result = unsafe { libc::gethostname(buffer.as_mut_ptr().cast(), buffer.len()) };
    if result != 0 {
        return Err(io::Error::last_os_error());
    }
    let length = buffer
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(buffer.len());
    Ok(String::from_utf8_lossy(&buffer[..length]).into_owned())
}

#[cfg(not(unix))]
fn system_hostname() -> io::Result<String> {
    std::env::var("COMPUTERNAME").map_err(|error| io::Error::other(error.to_string()))
}

#[cfg(unix)]
fn lookup_user_home(username: &str) -> io::Result<Option<PathBuf>> {
    let name = CString::new(username)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "username contains NUL"))?;
    let entry = passwd_entry(|entry, buffer, capacity, result| unsafe {
        libc::getpwnam_r(name.as_ptr(), entry, buffer, capacity, result)
    })?;
    Ok(entry.map(|(_, home)| home))
}

/// Runs a reentrant passwd lookup with a buffer grown on `ERANGE`, returning
/// the entry's username and home directory when one exists.
#[cfg(unix)]
fn passwd_entry(
    lookup: impl Fn(*mut libc::passwd, *mut libc::c_char, usize, *mut *mut libc::passwd) -> libc::c_int,
) -> io::Result<Option<(String, PathBuf)>> {
    use std::mem::MaybeUninit;
    use std::os::unix::ffi::OsStringExt;

    let mut capacity = passwd_buffer_size();
    loop {
        let mut entry = MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        let mut buffer = vec![0_u8; capacity];
        let code = lookup(
            entry.as_mut_ptr(),
            buffer.as_mut_ptr().cast(),
            buffer.len(),
            &mut result,
        );
        if code == libc::ERANGE && capacity < 1024 * 1024 {
            capacity *= 2;
            continue;
        }
        if code != 0 {
            return Err(io::Error::from_raw_os_error(code));
        }
        if result.is_null() {
            return Ok(None);
        }
        let entry = unsafe { entry.assume_init() };
        if entry.pw_name.is_null() || entry.pw_dir.is_null() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "passwd entry has no username or home directory",
            ));
        }
        let username = unsafe { CStr::from_ptr(entry.pw_name) }
            .to_str()
            .map_err(|_| {
                io::Error::new(io::ErrorKind::InvalidData, "passwd username is not UTF-8")
            })?
            .to_string();
        let home = unsafe { CStr::from_ptr(entry.pw_dir) }.to_bytes().to_vec();
        return Ok(Some((
            username,
            PathBuf::from(std::ffi::OsString::from_vec(home)),
        )));
    }
}

#[cfg(unix)]
fn passwd_buffer_size() -> usize {
    let recommended = unsafe { libc::sysconf(libc::_SC_GETPW_R_SIZE_MAX) };
    usize::try_from(recommended)
        .ok()
        .filter(|size| *size > 0)
        .unwrap_or(16 * 1_024)
        .max(1_024)
}

#[cfg(not(unix))]
fn lookup_user_home(_username: &str) -> io::Result<Option<PathBuf>> {
    Ok(None)
}

struct Directive {
    keyword: String,
    arguments: Vec<String>,
}

struct LexToken {
    value: String,
    first_character_protected: bool,
}

fn parse_directive(line: &str) -> Result<Option<Directive>, String> {
    let mut tokens = tokenize(line)?;
    if tokens.is_empty() {
        return Ok(None);
    }

    let first = tokens.remove(0);
    let (keyword, attached) = match first.value.split_once('=') {
        Some((keyword, value)) if !keyword.is_empty() => (keyword, Some(value)),
        _ => (first.value.as_str(), None),
    };
    let keyword = keyword.to_lowercase();
    let preserve_empty = keyword == "match";
    let mut arguments = Vec::new();
    if let Some(value) = attached.filter(|value| preserve_empty || !value.is_empty()) {
        arguments.push(value.to_string());
    }
    if let Some(token) = tokens
        .first_mut()
        .filter(|token| !token.first_character_protected)
    {
        if token.value == "=" {
            tokens.remove(0);
        } else if let Some(value) = token.value.strip_prefix('=') {
            token.value = value.to_string();
        }
    }
    arguments.extend(
        tokens
            .into_iter()
            .map(|token| token.value)
            .filter(|token| preserve_empty || !token.is_empty()),
    );

    Ok(Some(Directive { keyword, arguments }))
}

fn tokenize(line: &str) -> Result<Vec<LexToken>, String> {
    let characters: Vec<char> = line.chars().collect();
    let mut tokens = Vec::new();
    let mut index = 0;

    while index < characters.len() {
        while index < characters.len() && matches!(characters[index], ' ' | '\t') {
            index += 1;
        }
        if index == characters.len() || characters[index] == '#' {
            break;
        }

        let mut current = String::new();
        let mut quote = None;
        let mut first_character_protected = None;

        while index < characters.len() {
            let character = characters[index];
            if character == '\\' {
                let recognized = characters.get(index + 1).is_some_and(|next| {
                    matches!(next, '\'' | '"' | '\\') || (quote.is_none() && *next == ' ')
                });
                if recognized {
                    first_character_protected.get_or_insert(true);
                    current.push(characters[index + 1]);
                    index += 2;
                } else {
                    first_character_protected.get_or_insert(quote.is_some());
                    current.push(character);
                    index += 1;
                }
            } else if quote.is_none() && matches!(character, ' ' | '\t') {
                break;
            } else if quote.is_none() && matches!(character, '\'' | '"') {
                quote = Some(character);
                index += 1;
            } else if quote == Some(character) {
                quote = None;
                index += 1;
            } else {
                first_character_protected.get_or_insert(quote.is_some());
                current.push(character);
                index += 1;
            }
        }
        if let Some(delimiter) = quote {
            return Err(format!("unterminated {delimiter} quote"));
        }
        tokens.push(LexToken {
            value: current,
            first_character_protected: first_character_protected.unwrap_or(false),
        });
    }
    Ok(tokens)
}

fn is_concrete_alias(pattern: &str) -> bool {
    !pattern.is_empty()
        && !pattern.starts_with('!')
        && !pattern.contains('*')
        && !pattern.contains('?')
        && is_literal_ssh_destination(pattern)
}

fn is_literal_ssh_destination(value: &str) -> bool {
    !value.starts_with('-')
        && !value.contains('@')
        && value.get(..6) != Some("ssh://")
        && !value.chars().any(|character| {
            character.is_whitespace()
                || character.is_control()
                || "'`\"$\\;&<>|(){},".contains(character)
        })
}

fn host_block_matches(patterns: &[String], alias: &str) -> bool {
    let mut matched_positive = false;
    for pattern in patterns {
        if let Some(negative) = pattern.strip_prefix('!') {
            if pattern_matches(negative, alias) {
                return false;
            }
        } else if pattern_matches(pattern, alias) {
            matched_positive = true;
        }
    }
    matched_positive
}

fn declared_alias_index(patterns: &[String], alias: &str) -> Option<usize> {
    if patterns
        .iter()
        .filter_map(|pattern| pattern.strip_prefix('!'))
        .any(|negative| pattern_matches(negative, alias))
    {
        return None;
    }
    patterns
        .iter()
        .enumerate()
        .find(|(_, pattern)| is_concrete_alias(pattern) && pattern.as_str() == alias)
        .map(|(index, _)| index)
}

fn pattern_list_matches(patterns: &str, value: &str, hostname_style: bool) -> bool {
    let mut matched_positive = false;
    for pattern in patterns.split(',') {
        let (negative, pattern) = pattern
            .strip_prefix('!')
            .map_or((false, pattern), |pattern| (true, pattern));
        let matched = if hostname_style {
            hostname_pattern_matches(pattern, value)
        } else {
            pattern_matches(pattern, value)
        };
        if negative && matched {
            return false;
        }
        if !negative && matched {
            matched_positive = true;
        }
    }
    matched_positive
}

fn pattern_matches(pattern: &str, value: &str) -> bool {
    wildcard_pattern_matches(pattern.chars().collect(), value.chars().collect())
}

fn hostname_pattern_matches(pattern: &str, value: &str) -> bool {
    wildcard_pattern_matches(
        pattern.to_lowercase().chars().collect(),
        value.to_lowercase().chars().collect(),
    )
}

fn wildcard_pattern_matches(pattern: Vec<char>, value: Vec<char>) -> bool {
    let mut previous = vec![false; value.len() + 1];
    previous[0] = true;

    for token in pattern {
        let mut current = vec![false; value.len() + 1];
        if token == '*' {
            current[0] = previous[0];
            for index in 1..=value.len() {
                current[index] = previous[index] || current[index - 1];
            }
        } else {
            for index in 1..=value.len() {
                current[index] = previous[index - 1] && (token == '?' || token == value[index - 1]);
            }
        }
        previous = current;
    }
    previous[value.len()]
}

fn fuzzy_score(query: &str, candidate: &str) -> Option<i64> {
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return Some(0);
    }
    let candidate = candidate.to_lowercase();
    let query_chars: Vec<char> = query.chars().collect();
    let candidate_chars: Vec<char> = candidate.chars().collect();

    let mut query_index = 0;
    let mut last_match: Option<usize> = None;
    let mut score = 0_i64;
    for (index, character) in candidate_chars.iter().enumerate() {
        if query_index >= query_chars.len() || *character != query_chars[query_index] {
            continue;
        }
        score += 10;
        if last_match.is_some_and(|last| last + 1 == index) {
            score += 15;
        }
        if index == 0
            || candidate_chars
                .get(index.saturating_sub(1))
                .is_some_and(|previous| matches!(previous, '-' | '_' | '.' | '/' | ' '))
        {
            score += 8;
        }
        if let Some(last) = last_match {
            score -= i64::try_from(index.saturating_sub(last + 1)).unwrap_or(i64::MAX);
        }
        last_match = Some(index);
        query_index += 1;
    }
    if query_index != query_chars.len() {
        return None;
    }
    if candidate.contains(&query) {
        score += 100;
    }
    Some(score - i64::try_from(candidate_chars.len()).unwrap_or(i64::MAX))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn write(path: &Path, contents: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, contents).unwrap();
    }

    /// A temporary home directory whose `.ssh/config` is the root config.
    struct Fixture(TempDir);

    impl Fixture {
        fn new() -> Self {
            Self(TempDir::new().unwrap())
        }

        fn home(&self) -> &Path {
            self.0.path()
        }

        fn config(&self) -> PathBuf {
            self.home().join(".ssh/config")
        }

        /// Writes a file at a path relative to the home directory.
        fn write(&self, relative: impl AsRef<Path>, contents: &str) -> PathBuf {
            let path = self.home().join(relative);
            write(&path, contents);
            path
        }

        fn builder(&self) -> Builder {
            Builder::new(AccountIdentity::for_test(self.home().to_path_buf()))
        }

        fn load(&self) -> Result<Catalog, CatalogError> {
            Catalog::load_with_home(&self.config(), self.home())
        }

        fn aliases(&self) -> Vec<String> {
            aliases(&self.load().unwrap())
                .into_iter()
                .map(str::to_string)
                .collect()
        }
    }

    fn aliases(catalog: &Catalog) -> Vec<&str> {
        catalog
            .hosts()
            .iter()
            .map(|host| host.alias.as_str())
            .collect()
    }

    fn load_with_builder(config: &Path, builder: Builder) -> Catalog {
        builder.into_catalog(config, config).unwrap()
    }

    #[test]
    fn metadata_defaults_apply_to_concrete_declarations_and_hosts_can_override_or_clear_them() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            r#"
# porteiro: group="Work Team" tags=company color=blue
Host api api-alt *.internal !excluded excluded
    # porteiro: tags=prod,api color=red
Host staging
Host personal
    # porteiro: group="Personal" tags=home
Host standalone
    # porteiro: group="" tags= color=default
Host API
    # porteiro: group="Duplicate must not win"
"#,
        );
        let catalog = fixture.load().unwrap();
        assert!(catalog.warnings().is_empty());
        assert_eq!(
            aliases(&catalog),
            ["api", "api-alt", "staging", "personal", "standalone"]
        );
        let hosts = catalog.hosts();
        assert_eq!(hosts[0].metadata, hosts[1].metadata);
        assert_eq!(hosts[0].metadata.group.as_deref(), Some("Work Team"));
        assert_eq!(hosts[0].metadata.tags, ["prod", "api"]);
        assert_eq!(hosts[0].metadata.color, Some(AccentColor::Red));
        assert_eq!(hosts[2].metadata.tags, ["company"]);
        assert_eq!(hosts[2].metadata.color, Some(AccentColor::Blue));
        assert_eq!(hosts[3].metadata.group.as_deref(), Some("Personal"));
        assert_eq!(hosts[4].metadata, HostMetadata::default());
        assert_eq!(catalog.filtered_indices("Work Team").len(), 3);
        assert_eq!(catalog.filtered_indices("prod"), [0, 1]);
    }

    #[test]
    fn annotation_scope_stays_in_its_file_across_includes() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            r#"
# porteiro: group=Root color=blue
Include conf.d/*.conf missing.conf
Host parent
    Include options.conf
    # porteiro: tags=parent
Host sibling
"#,
        );
        fixture.write(
            ".ssh/conf.d/a.conf",
            "# porteiro: group=Child\nHost child\n  # porteiro: tags=child\n",
        );
        fixture.write(".ssh/conf.d/b.conf", "Host plain\n");
        fixture.write(
            ".ssh/options.conf",
            "# porteiro: group=MustNotLeak color=red\nUser deploy\n",
        );
        let catalog = fixture.load().unwrap();
        assert_eq!(aliases(&catalog), ["child", "plain", "parent", "sibling"]);
        let hosts = catalog.hosts();
        assert_eq!(hosts[0].metadata.group.as_deref(), Some("Child"));
        assert_eq!(hosts[0].metadata.color, None);
        assert_eq!(hosts[1].metadata, HostMetadata::default());
        assert_eq!(hosts[2].metadata.group.as_deref(), Some("Root"));
        assert_eq!(hosts[2].metadata.color, Some(AccentColor::Blue));
        assert_eq!(hosts[2].metadata.tags, ["parent"]);
        assert_eq!(hosts[2].hints.user.as_deref(), Some("deploy"));
        assert!(hosts[3].metadata.tags.is_empty());
    }

    #[test]
    fn annotations_cannot_make_conditional_aliases_reachable_or_override_the_selected_origin() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host gate\nInclude conditional.conf\nHost *\nInclude global.conf\n",
        );
        fixture.write(
            ".ssh/conditional.conf",
            "# porteiro: group=Unreachable\nHost duplicate hidden\n",
        );
        fixture.write(
            ".ssh/global.conf",
            "# porteiro: group=Reachable\nHost duplicate\n",
        );
        let catalog = fixture.load().unwrap();
        assert_eq!(aliases(&catalog), ["gate", "duplicate"]);
        assert_eq!(
            catalog.hosts()[1].metadata.group.as_deref(),
            Some("Reachable")
        );
    }

    #[test]
    fn annotation_failures_warn_with_source_without_partially_updating_metadata() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "# porteiro: group=Work\nHost api\n# porteiro: group=Wrong color=invalid\nHost *\n# porteiro: tags=ignored\nMatch all\n# porteiro: group=ignored\nHost other # porteiro: group=inline-is-not-metadata\n");
        let catalog = fixture.load().unwrap();
        assert_eq!(aliases(&catalog), ["api", "other"]);
        assert!(catalog
            .hosts()
            .iter()
            .all(|host| host.metadata.group.as_deref() == Some("Work")));
        assert_eq!(catalog.warnings().len(), 3);
        assert!(catalog.warnings()[0].contains("config:3: ignored porteiro annotation"));
        assert!(catalog.warnings()[0].contains("unknown color"));
        assert!(catalog.warnings()[1].contains("config:5:"));
        assert!(catalog.warnings()[2].contains("config:7:"));
    }

    #[test]
    fn annotations_do_not_mask_include_cycles_or_execute_match_criteria() {
        let fixture = Fixture::new();
        let marker = fixture.home().join("must-not-exist");
        fixture.write(".ssh/config", &format!(
            "# porteiro: group=Root\nMatch exec \"touch {}\"\nInclude hidden.conf\nHost visible\n# porteiro: tags=safe\n", marker.display()
        ));
        fixture.write(
            ".ssh/hidden.conf",
            "# porteiro: group=Hidden\nHost hidden\n",
        );
        let catalog = fixture.load().unwrap();
        assert_eq!(aliases(&catalog), ["visible"]);
        assert!(!marker.exists());
        fixture.write(
            ".ssh/config",
            "# porteiro: group=Root\nInclude cycle.conf\nHost api\n",
        );
        fixture.write(
            ".ssh/cycle.conf",
            "# porteiro: group=Cycle\nInclude config\n",
        );
        assert!(fixture.load().unwrap_err().to_string().contains("cycle"));
    }

    #[test]
    fn discovers_concrete_hosts_with_hints_in_declaration_order() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            r#"
Host *
  ServerAliveInterval 30

Host prod prod-alt *.corp !skip
  HostName prod.internal
  User deploy
  Port 2222
  ProxyJump bastion

Host staging
  HostName staging.internal
"#,
        );

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);
        assert_eq!(aliases, ["prod", "prod-alt", "staging"]);
        assert_eq!(
            catalog.hosts()[0].hints,
            HostHints {
                hostname: Some("prod.internal".into()),
                user: Some("deploy".into()),
                port: Some("2222".into()),
                proxy_jump: Some("bastion".into()),
                proxy_command: None,
            }
        );
    }

    #[test]
    fn follows_includes_in_lexical_order() {
        let fixture = Fixture::new();
        let home = fixture.home();
        let ssh_dir = home.join(".ssh");
        fixture.write(
            ".ssh/config",
            "Include conf.d/*.conf\nHost root\n  User root-user\nHost after\n",
        );
        write(&ssh_dir.join("conf.d/b.conf"), "Host beta\n");
        write(&ssh_dir.join("conf.d/a.conf"), "Host alpha\n");

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);
        assert_eq!(aliases, ["alpha", "beta", "root", "after"]);
        assert_eq!(catalog.hosts()[2].hints.user.as_deref(), Some("root-user"));
        assert_eq!(catalog.hosts()[1].hints.user, None);
        assert!(catalog.warnings().is_empty());
    }

    #[test]
    fn rejects_reachable_include_cycles() {
        let fixture = Fixture::new();
        let home = fixture.home();
        let ssh_dir = home.join(".ssh");
        let config = ssh_dir.join("config");
        fixture.write(".ssh/config", "Include conf.d/cycle.conf\nHost root\n");
        write(
            &ssh_dir.join("conf.d/cycle.conf"),
            "Host included\nInclude config\n",
        );

        let error = fixture.load().unwrap_err();
        match error {
            CatalogError::IncludeCycle { path } => {
                assert_eq!(path, fs::canonicalize(config).unwrap());
            }
            other => panic!("expected an Include cycle error, got {other}"),
        }
    }

    #[test]
    fn expands_environment_home_token_and_user_tilde_and_skips_hidden_matches() {
        let temp = TempDir::new().unwrap();
        let home = temp.path().join("home");
        let ssh_dir = home.join(".ssh");
        let config = ssh_dir.join("config");
        let environment_root = temp.path().join("100%-environment");
        let other_home = temp.path().join("other-home");
        write(
            &config,
            "Include ${PORTEIRO_FIXTURE_ROOT}/*.conf\nInclude %d/.ssh/token.conf\nInclude ~fixture-user/user.conf\nInclude missing/*.conf\nHost root\n",
        );
        write(&environment_root.join("a.conf"), "Host environment\n");
        write(&environment_root.join(".disabled.conf"), "Host hidden\n");
        write(&ssh_dir.join("token.conf"), "Host home-token\n");
        write(&other_home.join("user.conf"), "Host other-user\n");

        let mut builder = Builder::new(AccountIdentity::for_test(home));
        builder.environment.insert(
            "PORTEIRO_FIXTURE_ROOT".to_string(),
            environment_root.to_string_lossy().into_owned(),
        );
        builder
            .user_homes
            .insert("fixture-user".to_string(), other_home);
        let catalog = load_with_builder(&config, builder);
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["environment", "home-token", "other-user", "root"]);
        assert!(catalog.warnings().is_empty());
    }

    #[test]
    fn include_identity_comes_from_the_account_instead_of_home_or_user_env() {
        let temp = TempDir::new().unwrap();
        let account_home = temp.path().join("passwd-home");
        let config = account_home.join(".ssh/config");
        write(
            &config,
            "Include relative.conf\nInclude %d/token.conf\nInclude ~passwd-user/user.conf\n",
        );
        write(&account_home.join(".ssh/relative.conf"), "Host relative\n");
        write(&account_home.join("token.conf"), "Host home-token\n");
        write(&account_home.join("user.conf"), "Host current-user\n");

        let mut builder = Builder::new(AccountIdentity {
            username: "passwd-user".to_string(),
            home: account_home,
            uid: test_uid(),
        });
        builder
            .environment
            .insert("USER".to_string(), "environment-user".to_string());
        builder.environment.insert(
            "HOME".to_string(),
            temp.path().join("environment-home").display().to_string(),
        );
        let catalog = load_with_builder(&config, builder);
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["relative", "home-token", "current-user"]);
    }

    #[test]
    fn missing_include_environment_variable_is_ignored_with_a_warning() {
        let fixture = Fixture::new();
        let config = fixture.config();
        fixture.write(
            ".ssh/config",
            "Include ${PORTEIRO_TEST_MISSING}/config\nHost retained\n",
        );

        let mut builder = fixture.builder();
        builder.environment.remove("PORTEIRO_TEST_MISSING");
        let catalog = load_with_builder(&config, builder);

        assert_eq!(catalog.hosts()[0].alias, "retained");
        assert_eq!(
            catalog.warnings().len(),
            1,
            "warnings: {:?}",
            catalog.warnings()
        );
        assert!(catalog.warnings()[0].contains("PORTEIRO_TEST_MISSING"));
    }

    #[test]
    fn rejects_include_nesting_beyond_the_openssh_limit() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Include nested/0.conf\n");
        for depth in 0..17 {
            let contents = if depth == 16 {
                "Host unreachable\n".to_string()
            } else {
                format!("Include nested/{}.conf\n", depth + 1)
            };
            fixture.write(format!(".ssh/nested/{depth}.conf"), &contents);
        }

        let error = fixture.load().unwrap_err().to_string();

        assert!(error.contains("too many recursive"));
        assert!(error.contains("maximum depth: 16"));
    }

    #[test]
    fn expands_all_supported_include_percent_tokens_for_a_concrete_host() {
        let temp = TempDir::new().unwrap();
        let mut builder = Builder::new(AccountIdentity::for_test(temp.path().to_path_buf()));
        builder.hosts.push(Host {
            alias: "original".to_string(),
            source: temp.path().join("config"),
            line: 1,
            hints: HostHints {
                hostname: Some("host-%h".to_string()),
                user: Some("remote-user".to_string()),
                port: Some("2200".to_string()),
                ..HostHints::default()
            },
            metadata: HostMetadata::default(),
        });
        builder.update_proxy(&[0], |proxy| proxy.set_jump(Truth::Yes, "jump", "jump"));
        builder.host_key_aliases.insert(0, "key-alias".to_string());
        let target = builder.include_target(0);
        let hash = connection_hash(&builder.local_hostname, &target).unwrap();

        let expanded = builder
            .expand_include_tokens(
                "%%/%C/%d/%h/%i/%j/%k/%L/%l/%n/%p/%r/%u",
                Some(&target),
                false,
            )
            .unwrap();
        let expected = format!(
            "%/{hash}/{}/{}/{}/jump/key-alias/{}/{}/original/2200/remote-user/{}",
            builder.home.display(),
            "host-original",
            builder.local_uid,
            builder
                .local_hostname
                .split('.')
                .next()
                .unwrap_or(&builder.local_hostname),
            builder.local_hostname,
            builder.local_username,
        );
        assert_eq!(expanded.value, expected);
        assert!(!expanded.tokens.unknown_target);
        assert!(expanded.tokens.inherited_fallback);
    }

    #[test]
    fn conservatively_discovers_includes_using_inherited_target_values() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host *\n  HostName inherited-target\n  Include inherited/%h\n",
        );
        fixture.write(".ssh/inherited/inherited-target", "Host inherited-alias\n");

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["inherited-alias"]);
        assert_eq!(
            catalog.warnings().len(),
            1,
            "warnings: {:?}",
            catalog.warnings()
        );
        assert!(catalog.warnings()[0].contains("target-dependent tokens"));
    }

    #[test]
    fn original_host_token_does_not_scan_sibling_files() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  Include exact/%n.conf\n");
        fixture.write(".ssh/exact/expected.conf", "Host expected\n");
        fixture.write(".ssh/exact/unrelated.conf", "Host spurious\n");

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["expected"]);
        assert_eq!(catalog.warnings().len(), 1);
    }

    #[test]
    fn mixed_concrete_and_wildcard_hosts_keep_an_unknown_include_context() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host seed *\n  Include by/%n.conf\n");
        fixture.write(
            ".ssh/by/hidden.conf",
            "Host hidden\n  HostName valid.example\n",
        );

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["seed", "hidden"]);
        assert_eq!(
            catalog.hosts()[1].hints.hostname.as_deref(),
            Some("valid.example")
        );
    }

    #[test]
    fn conservative_target_scans_cross_separators_and_leading_dots() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  Include by/%n.conf\n");
        fixture.write(
            ".ssh/by/team/prod.conf",
            "Host team/prod\n  HostName valid.example\n",
        );
        fixture.write(
            ".ssh/by/.hidden.conf",
            "Host .hidden\n  HostName hidden.example\n",
        );

        assert_eq!(fixture.aliases(), [".hidden", "team/prod"]);
    }

    #[test]
    fn conservative_target_scans_follow_parent_components_within_the_config_tree() {
        let fixture = Fixture::new();
        let home = fixture.home();
        let ssh_dir = home.join(".ssh");
        fixture.write(".ssh/config", "Host *\n  Include by/%n.conf\n");
        fs::create_dir_all(ssh_dir.join("by")).unwrap();
        write(
            &ssh_dir.join("outside.conf"),
            "Host ../outside\n  HostName outside.example\n",
        );

        let catalog = fixture.load().unwrap();

        assert_eq!(catalog.hosts().len(), 1);
        assert_eq!(catalog.hosts()[0].alias, "../outside");
        assert_eq!(
            catalog.hosts()[0].hints.hostname.as_deref(),
            Some("outside.example")
        );
    }

    #[test]
    fn unknown_target_tokens_are_not_captured_by_glob_character_classes() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  Include classes/[%n].conf\n");
        fixture.write(".ssh/classes/a.conf", "Host a\n  HostName a.example\n");
        fixture.write(
            ".ssh/classes/z.conf",
            "Host spurious\n  HostName wrong.example\n",
        );

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["a"]);
        assert_eq!(
            catalog.hosts()[0].hints.hostname.as_deref(),
            Some("a.example")
        );
    }

    #[test]
    fn multiple_target_dependent_includes_share_their_host_context() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host seed *\n  Include by/%n.conf\n  Include classes/[%n].conf\n",
        );
        fixture.write(
            ".ssh/by/hidden.conf",
            "Host hidden\n  HostName valid.example\n",
        );
        fixture.write(
            ".ssh/by/.hidden.conf",
            "Host .hidden\n  HostName hidden.example\n",
        );
        fixture.write(
            ".ssh/by/team/prod.conf",
            "Host team/prod\n  HostName team.example\n",
        );
        fixture.write(".ssh/classes/a.conf", "Host a\n  HostName a.example\n");
        fixture.write(
            ".ssh/classes/z.conf",
            "Host spurious\n  HostName wrong.example\n",
        );

        assert_eq!(
            fixture.aliases(),
            ["seed", ".hidden", "hidden", "team/prod", "a"]
        );
    }

    #[test]
    fn malformed_include_globs_are_ignored_like_openssh() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Include broken/[\nHost kept\n");

        let catalog = fixture.load().unwrap();

        assert_eq!(catalog.hosts().len(), 1);
        assert_eq!(catalog.hosts()[0].alias, "kept");
        assert!(catalog
            .warnings()
            .iter()
            .any(|warning| warning.contains("ignoring invalid Include pattern")));
    }

    #[test]
    fn invalid_include_percent_tokens_are_fatal_like_openssh() {
        for (pattern, expected) in [
            ("invalid-%x.conf", "unsupported"),
            ("invalid-%", "trailing"),
        ] {
            let fixture = Fixture::new();
            fixture.write(
                ".ssh/config",
                &format!("Host prod\n  Include {pattern}\n  HostName prod.example\n"),
            );

            let error = fixture.load().unwrap_err().to_string();

            assert!(error.contains(expected), "unexpected error: {error}");
            assert!(error.contains(":2:"), "unexpected error: {error}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn conservative_scans_preserve_lexical_symlink_paths() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let home = fixture.home();
        let by = home.join(".ssh/by");
        fixture.write(".ssh/config", "Host *\n  Include by/%n/fixed/hosts.conf\n");
        write(
            &by.join("a-real/hosts.conf"),
            "Host z-team\n  HostName team.example\n",
        );
        fs::create_dir_all(by.join("z-team")).unwrap();
        symlink("../a-real", by.join("z-team/fixed")).unwrap();

        let catalog = fixture.load().unwrap();
        assert_eq!(catalog.hosts().len(), 1);
        assert_eq!(catalog.hosts()[0].alias, "z-team");
        assert_eq!(
            catalog.hosts()[0].hints.hostname.as_deref(),
            Some("team.example")
        );
    }

    #[test]
    fn exact_include_globs_do_not_cross_separators_or_leading_dots() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Include exact/*.conf\n");
        fixture.write(".ssh/exact/top.conf", "Host top\n");
        fixture.write(".ssh/exact/nested/deep.conf", "Host nested\n");
        fixture.write(".ssh/exact/.hidden.conf", "Host hidden\n");

        assert_eq!(fixture.aliases(), ["top"]);
    }

    #[test]
    fn conditional_includes_do_not_register_unreachable_aliases() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host root\n  Include conditional.conf\nMatch originalhost never\n  Include match.conf\nHost kept\n",
        );
        fixture.write(".ssh/conditional.conf", "Host unreachable-host\n");
        fixture.write(".ssh/match.conf", "Host unreachable-match\n");

        assert_eq!(fixture.aliases(), ["root", "kept"]);
    }

    #[test]
    fn host_and_user_conditions_are_case_sensitive_but_hostname_matches_are_not() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host foo*\n  Include wrong-host.conf\nMatch user TESTER\n  Include wrong-user.conf\nMatch localuser TESTER\n  Include wrong-local-user.conf\nHost *\n  HostName PROD-TARGET\nMatch host prod-target\n  Include matched-host.conf\n",
        );
        fixture.write(".ssh/wrong-host.conf", "Host FooIncluded\n");
        fixture.write(".ssh/wrong-user.conf", "Host wrong-user\n");
        fixture.write(".ssh/wrong-local-user.conf", "Host wrong-local-user\n");
        fixture.write(".ssh/matched-host.conf", "Host matched\n");

        assert_eq!(fixture.aliases(), ["matched"]);
    }

    #[test]
    fn dynamic_match_values_cannot_authorize_target_dependent_includes() {
        struct Case<'a> {
            directive: &'a str,
            include: &'a str,
            fallback_path: &'a str,
        }

        for case in [
            Case {
                directive: "HostName dynamic-host",
                include: "dynamic/%h.conf",
                fallback_path: "dynamic/victim.conf",
            },
            Case {
                directive: "User dynamic-user",
                include: "dynamic/%r.conf",
                fallback_path: "dynamic/tester.conf",
            },
            Case {
                directive: "Port 2200",
                include: "dynamic/%p.conf",
                fallback_path: "dynamic/22.conf",
            },
            Case {
                directive: "ProxyJump jump.example",
                include: "dynamic/%j/hosts.conf",
                fallback_path: "dynamic/hosts.conf",
            },
            Case {
                directive: "HostKeyAlias dynamic-key",
                include: "dynamic/%k.conf",
                fallback_path: "dynamic/victim.conf",
            },
        ] {
            let fixture = Fixture::new();
            let home = fixture.home();
            fixture.write(
                ".ssh/config",
                &format!(
                    "Match exec \"true\"\n  {}\nHost *\n  Include {}\n",
                    case.directive, case.include
                ),
            );
            write(&home.join(".ssh").join(case.fallback_path), "Host victim\n");

            let catalog = fixture.load().unwrap();
            assert!(
                catalog.hosts().is_empty(),
                "dynamic directive unexpectedly authorized an Include: {}",
                case.directive
            );
        }
    }

    #[test]
    fn dynamic_match_taints_connection_hash_includes() {
        let fixture = Fixture::new();
        let builder = fixture.builder();
        let target = IncludeTarget {
            hostname: Some("victim".to_string()),
            original_host: "victim".to_string(),
            remote_user: Some("tester".to_string()),
            port: Some("22".to_string()),
            jump_host: Some(String::new()),
            key_alias: Some("victim".to_string()),
        };
        let fallback_hash = connection_hash(&builder.local_hostname, &target).unwrap();
        fixture.write(
            ".ssh/config",
            "Match exec \"true\"\n  HostName dynamic-host\nHost *\n  Include dynamic/%C.conf\n",
        );
        fixture.write(
            format!(".ssh/dynamic/{fallback_hash}.conf"),
            "Host victim\n",
        );

        assert!(fixture.aliases().is_empty());
    }

    #[test]
    fn session_dependent_match_values_taint_target_dependent_includes() {
        struct Case<'a> {
            directive: &'a str,
            include: &'a str,
            fallback_path: &'a str,
        }

        for criterion in ["sessiontype shell", "command \"\""] {
            for case in [
                Case {
                    directive: "HostName dynamic-host",
                    include: "dynamic/%h.conf",
                    fallback_path: "dynamic/victim.conf",
                },
                Case {
                    directive: "User dynamic-user",
                    include: "dynamic/%r.conf",
                    fallback_path: "dynamic/tester.conf",
                },
                Case {
                    directive: "Port 2200",
                    include: "dynamic/%p.conf",
                    fallback_path: "dynamic/22.conf",
                },
                Case {
                    directive: "ProxyJump jump.example",
                    include: "dynamic/%j/hosts.conf",
                    fallback_path: "dynamic/hosts.conf",
                },
                Case {
                    directive: "HostKeyAlias dynamic-key",
                    include: "dynamic/%k.conf",
                    fallback_path: "dynamic/victim.conf",
                },
            ] {
                let fixture = Fixture::new();
                let home = fixture.home();
                fixture.write(
                    ".ssh/config",
                    &format!(
                        "Match {criterion}\n  {}\nHost *\n  Include {}\n",
                        case.directive, case.include
                    ),
                );
                write(&home.join(".ssh").join(case.fallback_path), "Host victim\n");

                let catalog = fixture.load().unwrap();
                assert!(
                    catalog.hosts().is_empty(),
                    "{criterion:?} unexpectedly authorized an Include after {}",
                    case.directive
                );
            }
        }
    }

    #[test]
    fn final_pass_matches_host_blocks_against_the_effective_lowercase_host() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host *\n  HostName TARGET.EXAMPLE\nMatch final\n  Include final.conf\n",
        );
        fixture.write(".ssh/final.conf", "Host prod\nHost target.example\n");

        assert_eq!(fixture.aliases(), ["target.example"]);
    }

    #[test]
    fn canonical_matches_during_a_final_pass_requested_by_match_final() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Match final\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod\n");

        assert_eq!(fixture.aliases(), ["prod"]);
    }

    #[test]
    fn canonicalize_hostname_requests_a_deterministic_final_pass_for_ip_aliases() {
        for setting in ["yes", "true", "always"] {
            let fixture = Fixture::new();
            fixture.write(".ssh/config", &format!(
                    "Host *\n  CanonicalizeHostname {setting}\nMatch canonical\n  Include canonical.conf\n"
                ),
            );
            fixture.write(".ssh/canonical.conf", "Host 192.0.2.1\n");

            assert_eq!(fixture.aliases(), ["192.0.2.1"], "setting {setting:?}");
        }
    }

    #[test]
    fn canonicalization_that_can_use_dns_does_not_authorize_final_host_blocks() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host *\n  CanonicalizeHostname always\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod\n");

        assert!(fixture.aliases().is_empty());
    }

    #[test]
    fn canonicalize_max_dots_allows_a_dns_free_final_pass_above_the_default_limit() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  HostName PROD.EXAMPLE.TEST\n  CanonicalizeHostname yes\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod.example.test\n");

        assert_eq!(fixture.aliases(), ["prod.example.test"]);
    }

    #[test]
    fn canonicalize_max_dots_keeps_names_at_the_limit_dns_dependent() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  HostName prod.example.test\n  CanonicalizeHostname yes\n  CanonicalizeMaxDots 2\n  CanonicalDomains example.invalid\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod.example.test\n");

        assert!(fixture.aliases().is_empty());
    }

    #[test]
    fn canonicalize_max_dots_uses_the_first_obtained_value() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  HostName prod.example.test\n  CanonicalizeHostname yes\n  CanonicalizeMaxDots 1\n  CanonicalizeMaxDots 2\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod.example.test\n");

        assert_eq!(fixture.aliases(), ["prod.example.test"]);
    }

    #[test]
    fn an_unknown_canonicalize_max_dots_does_not_assume_dns_is_skipped() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  HostName prod.example.test\n  CanonicalizeHostname yes\nMatch exec true\n  CanonicalizeMaxDots 1\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod.example.test\n");

        assert!(fixture.aliases().is_empty());
    }

    #[test]
    fn invalid_canonicalize_max_dots_is_rejected() {
        for value in ["-1", "2147483648", "many"] {
            let fixture = Fixture::new();
            fixture.write(
                ".ssh/config",
                &format!("CanonicalizeMaxDots {value}\nHost prod\n"),
            );

            let error = fixture.load().unwrap_err();
            assert!(
                error.to_string().contains("invalid CanonicalizeMaxDots"),
                "value {value:?}: {error}"
            );
        }
    }

    #[test]
    fn canonicalize_yes_preserves_a_known_proxied_hostname_for_the_final_pass() {
        for proxy in ["ProxyJump jump.example", "ProxyCommand nc proxy 22"] {
            let fixture = Fixture::new();
            fixture.write(".ssh/config", &format!(
                    "Host *\n  CanonicalizeHostname yes\n  {proxy}\nMatch canonical\n  Include canonical.conf\n"
                ),
            );
            fixture.write(".ssh/canonical.conf", "Host prod\n");

            assert_eq!(fixture.aliases(), ["prod"], "proxy {proxy:?}");
        }
    }

    #[test]
    fn canonicalize_always_keeps_proxied_dns_names_unknown() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  CanonicalizeHostname always\n  ProxyJump jump.example\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host prod\n");

        assert!(fixture.aliases().is_empty());
    }

    #[test]
    fn canonical_final_pass_uses_the_canonical_textual_ip_address() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host *\n  CanonicalizeHostname yes\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(
            ".ssh/canonical.conf",
            "Host 2001:0DB8:0:0::1\nHost 2001:db8::1\n",
        );

        assert_eq!(fixture.aliases(), ["2001:db8::1"]);
    }

    #[cfg(unix)]
    #[test]
    fn canonical_final_pass_normalizes_legacy_numeric_addresses_without_dns() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  HostName 192.000.002.001\n  CanonicalizeHostname yes\nMatch canonical\n  Include canonical.conf\n",
        );
        fixture.write(".ssh/canonical.conf", "Host 192.0.2.1\n");

        assert_eq!(fixture.aliases(), ["192.0.2.1"]);
        assert_eq!(
            normalize_numeric_host("192.000.002.001").as_deref(),
            Some("192.0.2.1")
        );
        assert!(normalize_numeric_host("fe80::1%1")
            .is_some_and(|address| address.starts_with("fe80::1%")));
    }

    #[test]
    fn reachable_declaration_replaces_unreachable_candidate_hints_and_source() {
        let fixture = Fixture::new();
        let home = fixture.home();
        let conditional = home.join(".ssh/conditional.conf");
        let global = home.join(".ssh/global.conf");
        fixture.write(
            ".ssh/config",
            "Host gate\n  Include conditional.conf\nHost *\n  Include global.conf\n",
        );
        write(
            &conditional,
            "Host duplicate\n  HostName unreachable.example\n",
        );
        write(&global, "Host duplicate\n  HostName reachable.example\n");

        let catalog = fixture.load().unwrap();
        let duplicate = catalog
            .hosts()
            .iter()
            .find(|host| host.alias == "duplicate")
            .unwrap();

        assert_eq!(duplicate.source, fs::canonicalize(global).unwrap());
        assert_eq!(
            duplicate.hints.hostname.as_deref(),
            Some("reachable.example")
        );
    }

    #[test]
    fn reachable_duplicate_uses_its_actual_traversal_position() {
        let fixture = Fixture::new();
        let home = fixture.home();
        let shared = home.join(".ssh/shared.conf");
        fixture.write(
            ".ssh/config",
            "Host gate\n  Include shared.conf\nHost middle\nHost *\n  Include shared.conf\n",
        );
        write(&shared, "Host duplicate\n");

        assert_eq!(fixture.aliases(), ["gate", "middle", "duplicate"]);
    }

    #[cfg(unix)]
    #[test]
    fn rejects_group_or_world_writable_included_files_but_not_the_root() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new();
        let home = fixture.home();
        let config = fixture.config();
        let included = home.join(".ssh/unsafe.conf");
        fixture.write(".ssh/config", "Include unsafe.conf\n");
        write(&included, "Host unsafe\n");
        fs::set_permissions(&config, fs::Permissions::from_mode(0o666)).unwrap();
        fs::set_permissions(&included, fs::Permissions::from_mode(0o666)).unwrap();

        let error = fixture.load().unwrap_err();
        match error {
            CatalogError::UnsafeInclude { path, mode, .. } => {
                assert_eq!(path, fs::canonicalize(included).unwrap());
                assert_eq!(mode & 0o022, 0o022);
            }
            other => panic!("expected an unsafe Include error, got {other}"),
        }
    }

    #[test]
    fn normalizes_proxy_jump_forms_to_the_final_hop() {
        assert_eq!(
            normalize_proxy_jump("alice@jump.example:2222").unwrap(),
            "jump.example"
        );
        assert_eq!(
            normalize_proxy_jump("ssh://alice@jump.example:2222").unwrap(),
            "jump.example"
        );
        assert_eq!(
            normalize_proxy_jump("alice@[2001:db8::1]:2222").unwrap(),
            "2001:db8::1"
        );
        assert_eq!(
            normalize_proxy_jump("first.example,bob@final.example:2200").unwrap(),
            "final.example"
        );
        assert_eq!(normalize_proxy_jump("none").unwrap(), "");
    }

    #[test]
    fn normalized_and_empty_jump_tokens_reach_their_includes() {
        let fixture = Fixture::new();
        let home = fixture.home();
        fixture.write(
            ".ssh/config",
            "Host *\n  ProxyJump alice@jump.example:2222\n  Include jump/%j.conf\n",
        );
        fixture.write(".ssh/jump/jump.example.conf", "Host normalized\n");
        let catalog = fixture.load().unwrap();
        assert_eq!(
            catalog.hosts().first().map(|host| host.alias.as_str()),
            Some("normalized"),
            "warnings: {:?}",
            catalog.warnings()
        );

        let empty_config = home.join(".ssh/empty-config");
        write(
            &empty_config,
            "Host *\n  Include jump-empty/%j/hosts.conf\n",
        );
        fixture.write(".ssh/jump-empty/hosts.conf", "Host no-jump\n");
        let catalog = Catalog::load_with_home(&empty_config, home).unwrap();
        assert_eq!(
            catalog.hosts().first().map(|host| host.alias.as_str()),
            Some("no-jump"),
            "warnings: {:?}",
            catalog.warnings()
        );
    }

    #[test]
    fn proxy_command_precedence_keeps_jump_token_empty() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host *\n  ProxyCommand nc proxy 22\n  ProxyJump ignored.example\n  Include command-first/%j/hosts.conf\n",
        );
        fixture.write(".ssh/command-first/hosts.conf", "Host command-first\n");

        let catalog = fixture.load().unwrap();
        assert_eq!(
            catalog.hosts().first().map(|host| host.alias.as_str()),
            Some("command-first"),
            "warnings: {:?}",
            catalog.warnings()
        );
        assert_eq!(
            catalog.hosts()[0].hints.proxy_command.as_deref(),
            Some("nc proxy 22")
        );
        assert!(catalog.hosts()[0].hints.proxy_jump.is_none());
    }

    #[test]
    fn connection_hash_uses_the_normalized_jump_host() {
        let fixture = Fixture::new();
        let builder = fixture.builder();
        let target = IncludeTarget {
            hostname: Some("hashed".to_string()),
            original_host: "hashed".to_string(),
            remote_user: Some("tester".to_string()),
            port: Some("22".to_string()),
            jump_host: Some("jump.example".to_string()),
            key_alias: Some("hashed".to_string()),
        };
        let hash = connection_hash(&builder.local_hostname, &target).unwrap();
        fixture.write(
            ".ssh/config",
            "Host *\n  ProxyJump alice@jump.example:2222\n  Include hashes/%C.conf\n",
        );
        fixture.write(format!(".ssh/hashes/{hash}.conf"), "Host hashed\n");

        let catalog = fixture.load().unwrap();
        assert_eq!(catalog.hosts()[0].alias, "hashed");
    }

    #[test]
    fn port_tokens_use_canonical_decimal_values() {
        for raw_port in ["0022", "+22"] {
            let fixture = Fixture::new();
            fixture.write(
                ".ssh/config",
                &format!("Host *\n  Port {raw_port}\n  Include by-port/%p.conf\n"),
            );
            fixture.write(".ssh/by-port/22.conf", "Host normalized\n");
            fixture.write(
                format!(".ssh/by-port/{raw_port}.conf"),
                "Host raw-spelling\n",
            );

            let catalog = fixture.load().unwrap();
            assert_eq!(catalog.hosts().len(), 1, "raw port {raw_port:?}");
            assert_eq!(catalog.hosts()[0].alias, "normalized");
            assert_eq!(catalog.hosts()[0].hints.port.as_deref(), Some("22"));
        }
    }

    #[test]
    fn connection_hash_uses_the_canonical_decimal_port() {
        let fixture = Fixture::new();
        let builder = fixture.builder();
        let mut target = IncludeTarget {
            hostname: Some("hashed-port".to_string()),
            original_host: "hashed-port".to_string(),
            remote_user: Some("tester".to_string()),
            port: Some("22".to_string()),
            jump_host: Some(String::new()),
            key_alias: Some("hashed-port".to_string()),
        };
        let canonical_hash = connection_hash(&builder.local_hostname, &target).unwrap();
        target.port = Some("0022".to_string());
        let raw_hash = connection_hash(&builder.local_hostname, &target).unwrap();
        assert_ne!(canonical_hash, raw_hash);
        fixture.write(
            ".ssh/config",
            "Host *\n  Port 0022\n  Include hashes/%C.conf\n",
        );
        fixture.write(
            format!(".ssh/hashes/{canonical_hash}.conf"),
            "Host hashed-port\n",
        );
        fixture.write(format!(".ssh/hashes/{raw_hash}.conf"), "Host raw-hash\n");

        assert_eq!(fixture.aliases(), ["hashed-port"]);
    }

    #[test]
    fn invalid_ports_are_rejected() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host prod\n  Port 70000\n");

        let error = fixture.load().unwrap_err().to_string();
        assert!(error.contains("invalid Port"));
    }

    #[test]
    fn malformed_conservative_candidates_do_not_block_reachable_aliases() {
        let fixture = Fixture::new();
        let home = fixture.home();
        fixture.write(".ssh/config", "Host *\n  Include candidates/%h\n");
        fixture.write(".ssh/candidates/good", "Host good\n");
        let malformed = home.join(".ssh/candidates/bad");
        fs::write(&malformed, [0xff, 0xfe]).unwrap();

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);

        assert_eq!(aliases, ["good"]);
        assert!(catalog
            .warnings()
            .iter()
            .any(|warning| warning.contains("conservative Include candidate")));
    }

    #[cfg(unix)]
    #[test]
    fn dangling_conservative_candidates_warn_but_exact_candidates_error() {
        use std::os::unix::fs::symlink;

        let fixture = Fixture::new();
        let home = fixture.home();
        let candidates = home.join(".ssh/candidates");
        fixture.write(".ssh/config", "Host *\n  Include candidates/%h.conf\n");
        write(&candidates.join("good.conf"), "Host good\n");
        let stale = candidates.join("stale.conf");
        symlink("missing.conf", &stale).unwrap();

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);
        assert_eq!(aliases, ["good"]);
        assert!(catalog.warnings().iter().any(|warning| {
            warning.contains("conservative Include candidate") && warning.contains("stale.conf")
        }));

        let exact_config = home.join(".ssh/exact-config");
        let exact = home.join(".ssh/exact.conf");
        write(&exact_config, "Include exact.conf\n");
        symlink("missing.conf", &exact).unwrap();
        assert!(matches!(
            Catalog::load_with_home(&exact_config, home),
            Err(CatalogError::Read { path, .. }) if path == exact
        ));
    }

    #[cfg(unix)]
    #[test]
    fn unreadable_exact_glob_entries_do_not_block_readable_aliases() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = Fixture::new();
        let home = fixture.home();
        let blocked = home.join(".ssh/blocked");
        fixture.write(
            ".ssh/config",
            "Include readable.conf\nInclude blocked/*.conf\n",
        );
        fixture.write(".ssh/readable.conf", "Host kept\n  HostName kept.example\n");
        write(&blocked.join("hidden.conf"), "Host hidden\n");

        let original_permissions = fs::metadata(&blocked).unwrap().permissions();
        let mut unreadable = original_permissions.clone();
        unreadable.set_mode(0o000);
        fs::set_permissions(&blocked, unreadable).unwrap();
        if fs::read_dir(&blocked).is_ok() {
            fs::set_permissions(&blocked, original_permissions).unwrap();
            return;
        }

        let result = fixture.load();
        fs::set_permissions(&blocked, original_permissions).unwrap();
        let catalog = result.unwrap();

        assert_eq!(catalog.hosts().len(), 1);
        assert_eq!(catalog.hosts()[0].alias, "kept");
        assert!(catalog
            .warnings()
            .iter()
            .any(|warning| warning.contains("cannot inspect Include match")));
    }

    #[test]
    fn treats_consecutive_stars_like_a_libc_glob() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Include tree/**/hosts.conf\nHost root\n");
        fixture.write(".ssh/tree/one/hosts.conf", "Host shallow\n");
        fixture.write(".ssh/tree/one/two/hosts.conf", "Host recursive-only\n");

        assert_eq!(fixture.aliases(), ["shallow", "root"]);
        assert_eq!(collapse_repeated_stars("a/***/b**c"), "a/*/b*c");
    }

    #[test]
    fn quoted_values_comments_and_equals_are_supported_but_invalid_aliases_are_skipped() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host=\"my lab\" plain # comment\n  HostName = host.example\n",
        );

        let catalog = fixture.load().unwrap();
        assert_eq!(catalog.hosts().len(), 1);
        assert_eq!(catalog.hosts()[0].alias, "plain");
        assert_eq!(
            catalog.hosts()[0].hints.hostname.as_deref(),
            Some("host.example")
        );
    }

    #[test]
    fn unspaced_equals_is_a_separator_but_protected_equals_is_data() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Include =included.conf\nHost =plain\n  Port =0022\nHost ==literal\nHost \"=protected\"\n",
        );
        fixture.write(".ssh/included.conf", "Host included\n");

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);
        assert_eq!(aliases, ["included", "plain", "=literal", "=protected"]);
        assert_eq!(
            catalog
                .hosts()
                .iter()
                .find(|host| host.alias == "plain")
                .unwrap()
                .hints
                .port
                .as_deref(),
            Some("22")
        );

        assert_eq!(
            parse_directive("Host =foo").unwrap().unwrap().arguments,
            ["foo"]
        );
        assert_eq!(
            parse_directive("Host ==foo").unwrap().unwrap().arguments,
            ["=foo"]
        );
        assert_eq!(
            parse_directive("Host \"=foo\"").unwrap().unwrap().arguments,
            ["=foo"]
        );
    }

    #[test]
    fn aliases_must_remain_literal_ssh_host_arguments() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host \"my lab\" -prod foo@bar ssh://uri.example SSH://literal \"bad;name\" valid-alias\n",
        );

        assert_eq!(fixture.aliases(), ["SSH://literal", "valid-alias"]);
    }

    #[test]
    fn empty_match_arguments_are_preserved() {
        let directive = parse_directive("Match command \"\" tagged=")
            .unwrap()
            .unwrap();

        assert_eq!(directive.arguments, ["command", "", "tagged="]);
    }

    #[test]
    fn lexer_preserves_inline_comments_and_unrecognized_escapes() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host \"=foo\" prod#blue prod\\q valid # trailing comment\n  Port 23456\n",
        );

        let catalog = fixture.load().unwrap();
        let aliases = aliases(&catalog);
        assert_eq!(aliases, ["=foo", "prod#blue", "valid"]);
        assert!(!aliases.contains(&"foo"));
        assert!(!aliases.contains(&"prod"));
        assert!(!aliases.contains(&"prodq"));
        assert_eq!(catalog.hosts()[1].hints.port.as_deref(), Some("23456"));

        assert_eq!(
            parse_directive("Host = plain").unwrap().unwrap().arguments,
            ["plain"]
        );
        assert_eq!(
            parse_directive("Include \"=hosts.conf\"")
                .unwrap()
                .unwrap()
                .arguments,
            ["=hosts.conf"]
        );
        assert_eq!(
            parse_directive("Match \"=criterion\"")
                .unwrap()
                .unwrap()
                .arguments,
            ["=criterion"]
        );
        assert_eq!(
            parse_directive("Host prod\\q").unwrap().unwrap().arguments,
            ["prod\\q"]
        );
        assert_eq!(
            parse_directive("Host trailing\\")
                .unwrap()
                .unwrap()
                .arguments,
            ["trailing\\"]
        );
    }

    #[test]
    fn final_pass_lowercases_names_and_formats_ip_addresses_canonically() {
        assert_eq!(
            normalize_final_host("TARGET.EXAMPLE".to_string()),
            "target.example"
        );
        assert_eq!(
            normalize_final_host("2001:DB8::1".to_string()),
            "2001:db8::1"
        );
    }

    #[test]
    fn aliases_are_deduplicated_case_insensitively() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host Production\nHost production\nHost other\n",
        );

        assert_eq!(fixture.aliases(), ["Production", "other"]);
    }

    #[test]
    fn negated_patterns_can_exclude_a_concrete_alias() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host foo !f* bar\n");

        assert_eq!(fixture.aliases(), ["bar"]);
    }

    #[test]
    fn missing_root_config_has_an_actionable_error() {
        let temp = TempDir::new().unwrap();
        let error = Catalog::load_with_home(&temp.path().join("missing"), temp.path())
            .unwrap_err()
            .to_string();
        assert!(error.contains("SSH config not found"));
        assert!(error.contains("--config"));
    }

    #[test]
    fn malformed_quotes_report_the_file_and_line() {
        let fixture = Fixture::new();
        fixture.write(".ssh/config", "Host \"unterminated\n");

        let error = fixture.load().unwrap_err().to_string();
        assert!(error.contains(":1:"));
        assert!(error.contains("unterminated"));
    }

    #[test]
    fn fuzzy_filter_matches_alias_hostname_and_user() {
        let fixture = Fixture::new();
        fixture.write(
            ".ssh/config",
            "Host production\n HostName api.internal\n User deploy\nHost other\n",
        );
        let catalog = fixture.load().unwrap();

        assert_eq!(catalog.filtered_indices("prd"), [0]);
        assert_eq!(catalog.filtered_indices("api"), [0]);
        assert_eq!(catalog.filtered_indices("dpl"), [0]);
    }

    #[test]
    fn wildcard_matchers_use_the_required_case_modes() {
        assert!(!pattern_matches("PROD-*", "prod-api"));
        assert!(hostname_pattern_matches("PROD-*", "prod-api"));
        assert!(pattern_matches("db-?", "db-1"));
        assert!(!pattern_matches("db-?", "db-12"));
    }
}
