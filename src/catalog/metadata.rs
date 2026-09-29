//! Presentation metadata lives in full-line comments, which OpenSSH ignores:
//! <https://man.openbsd.org/ssh_config#DESCRIPTION>. It never changes SSH options
//! or participates in Host/Match evaluation.

use super::tokenize;
use std::collections::HashSet;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AccentColor {
    Red,
    Yellow,
    Green,
    Blue,
    Magenta,
    Cyan,
    White,
    Gray,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HostMetadata {
    pub group: Option<String>,
    pub tags: Vec<String>,
    pub color: Option<AccentColor>,
    pub hidden: bool,
}

/// An absent field inherits its value; an explicit empty field clears it.
#[derive(Default)]
pub(super) struct Annotation {
    group: Option<Option<String>>,
    tags: Option<Vec<String>>,
    color: Option<Option<AccentColor>>,
    hidden: Option<bool>,
}

impl Annotation {
    pub(super) fn apply(&self, metadata: &mut HostMetadata) {
        if let Some(group) = &self.group {
            metadata.group.clone_from(group);
        }
        if let Some(tags) = &self.tags {
            metadata.tags.clone_from(tags);
        }
        if let Some(color) = self.color {
            metadata.color = color;
        }
        if let Some(hidden) = self.hidden {
            metadata.hidden = hidden;
        }
    }
}

/// Parse the whole annotation before applying any of it. A typo must not leave
/// a host half-reclassified, and malformed presentation data cannot invalidate
/// an otherwise valid SSH configuration.
pub(super) fn parse(line: &str) -> Option<Result<Annotation, String>> {
    let body = line.trim_start_matches([' ', '\t']).strip_prefix('#')?;
    let body = body
        .trim_start_matches([' ', '\t'])
        .strip_prefix("porteiro:")?;
    Some(parse_fields(body))
}

fn parse_fields(body: &str) -> Result<Annotation, String> {
    let tokens = tokenize(body)?;
    if tokens.is_empty() {
        return Err("expected group=, tags=, color= or hidden=".into());
    }
    let mut annotation = Annotation::default();
    let mut seen = HashSet::new();
    for token in tokens {
        let (key, value) = token
            .value
            .split_once('=')
            .ok_or_else(|| "expected key=value (quote values containing spaces)".to_string())?;
        if !seen.insert(key.to_string()) {
            return Err(format!("duplicate metadata key {key:?}"));
        }
        // Metadata is displayed verbatim. Do not admit terminal controls or
        // directional overrides that could disguise the selected destination.
        if value.chars().any(|c| {
            c.is_control()
                || matches!(c, '\u{061c}' | '\u{200e}' | '\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
        }) {
            return Err("metadata values cannot contain control or bidi formatting characters".into());
        }
        match key {
            "group" => {
                let name = value.trim();
                annotation.group = Some((!name.is_empty()).then(|| name.to_string()));
            }
            "tags" => {
                let mut tags = Vec::new();
                let mut seen_tags = HashSet::new();
                if !value.is_empty() {
                    for tag in value.split(',').map(str::trim) {
                        if tag.is_empty() {
                            return Err("tags must be comma-separated nonempty names".into());
                        }
                        if seen_tags.insert(tag) {
                            tags.push(tag.to_string());
                        }
                    }
                }
                annotation.tags = Some(tags);
            }
            "color" => {
                annotation.color = Some(match value {
                    "default" => None,
                    "red" => Some(AccentColor::Red),
                    "yellow" => Some(AccentColor::Yellow),
                    "green" => Some(AccentColor::Green),
                    "blue" => Some(AccentColor::Blue),
                    "magenta" => Some(AccentColor::Magenta),
                    "cyan" => Some(AccentColor::Cyan),
                    "white" => Some(AccentColor::White),
                    "gray" => Some(AccentColor::Gray),
                    _ => return Err(format!("unknown color {value:?}; use red, yellow, green, blue, magenta, cyan, white, gray or default")),
                });
            }
            "hidden" => {
                annotation.hidden = Some(match value {
                    "true" => true,
                    "false" => false,
                    _ => return Err("hidden must be true or false".into()),
                });
            }
            _ => {
                return Err(format!(
                    "unknown metadata key {key:?}; expected group, tags, color or hidden"
                ))
            }
        }
    }
    Ok(annotation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn annotations_support_quotes_comments_unicode_and_stable_tag_deduplication() {
        let annotation =
            parse("  # porteiro: group=\"Equipe São Paulo\" tags='prod,api,prod' color=red # note")
                .unwrap()
                .unwrap();
        let mut metadata = HostMetadata::default();
        annotation.apply(&mut metadata);
        assert_eq!(metadata.group.as_deref(), Some("Equipe São Paulo"));
        assert_eq!(metadata.tags, ["prod", "api"]);
        assert_eq!(metadata.color, Some(AccentColor::Red));
    }

    #[test]
    fn explicit_resets_clear_inherited_values() {
        let mut metadata = HostMetadata {
            group: Some("Work".into()),
            tags: vec!["prod".into()],
            color: Some(AccentColor::Blue),
            hidden: true,
        };
        parse("# porteiro: group=\"\" tags= color=default hidden=false")
            .unwrap()
            .unwrap()
            .apply(&mut metadata);
        assert_eq!(metadata, HostMetadata::default());
    }

    #[test]
    fn malformed_annotations_are_errors_but_ordinary_comments_are_not_annotations() {
        for line in [
            "# porteiro:",
            "# porteiro: group=\"unclosed",
            "# porteiro: color=orange",
            "# porteiro: group=One group=Two",
            "# porteiro: tags=a,,b",
            "# porteiro: groups=Work",
            "# porteiro: group = Work",
            "# porteiro: hidden=yes",
            "# porteiro: hidden=",
            "# porteiro: hidden=true hidden=false",
            "# porteiro: group=\"escape\u{1b}[31m\"",
            "# porteiro: tags=\"prod\u{202e}dev\"",
        ] {
            assert!(parse(line).unwrap().is_err(), "{line:?}");
        }
        for line in [
            "# a normal comment",
            "Host api # porteiro: group=Work",
            "# porteiro-ish: group=Work",
        ] {
            assert!(parse(line).is_none());
        }
    }
}
