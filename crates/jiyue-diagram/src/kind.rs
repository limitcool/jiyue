/// Mermaid diagram families recognised from the header line.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DiagramKind {
    Flowchart,
    Sequence,
    Class,
    State,
    Er,
    Gantt,
    Pie,
    Journey,
    Mindmap,
    Timeline,
    GitGraph,
    Quadrant,
    XyChart,
    Sankey,
    Requirement,
    C4,
    Block,
    Architecture,
    Packet,
    Kanban,
    Radar,
    Treemap,
    ZenUml,
}

impl DiagramKind {
    /// Human-readable name, used in messages.
    pub fn name(self) -> &'static str {
        match self {
            Self::Flowchart => "flowchart",
            Self::Sequence => "sequence",
            Self::Class => "class",
            Self::State => "state",
            Self::Er => "entity-relationship",
            Self::Gantt => "gantt",
            Self::Pie => "pie",
            Self::Journey => "journey",
            Self::Mindmap => "mindmap",
            Self::Timeline => "timeline",
            Self::GitGraph => "git graph",
            Self::Quadrant => "quadrant",
            Self::XyChart => "xy chart",
            Self::Sankey => "sankey",
            Self::Requirement => "requirement",
            Self::C4 => "C4",
            Self::Block => "block",
            Self::Architecture => "architecture",
            Self::Packet => "packet",
            Self::Kanban => "kanban",
            Self::Radar => "radar",
            Self::Treemap => "treemap",
            Self::ZenUml => "ZenUML",
        }
    }

    /// Whether the native backend (`mermaid-rs-renderer`) renders this family.
    /// Kinds that return `false` are reported as [`crate::DiagramError::Unsupported`]
    /// instead of being sent to the backend.
    pub fn is_supported(self) -> bool {
        matches!(
            self,
            Self::Flowchart
                | Self::Sequence
                | Self::Class
                | Self::State
                | Self::Er
                | Self::Gantt
                | Self::Pie
                | Self::Journey
                | Self::Mindmap
                | Self::Timeline
                | Self::GitGraph
                | Self::Quadrant
                | Self::XyChart
                | Self::Sankey
                | Self::Requirement
                | Self::Block
                | Self::Architecture
                | Self::Packet
                | Self::Kanban
                | Self::Radar
                | Self::Treemap
                | Self::C4
        )
    }
}

/// Detects the diagram family from the first meaningful line of `source`.
///
/// Blank lines, `%%` comments (including `%%{init: ...}%%` directives) and a
/// leading YAML front-matter block (`---` ... `---`) are skipped. Returns `None`
/// when no known header is found.
pub fn detect_kind(source: &str) -> Option<DiagramKind> {
    let word = header_word(source)?;
    let kind = match word.to_ascii_lowercase().as_str() {
        "flowchart" | "flowchart-v2" | "graph" => DiagramKind::Flowchart,
        "sequencediagram" => DiagramKind::Sequence,
        "classdiagram" | "classdiagram-v2" => DiagramKind::Class,
        "statediagram" | "statediagram-v2" => DiagramKind::State,
        "erdiagram" => DiagramKind::Er,
        "gantt" => DiagramKind::Gantt,
        "pie" => DiagramKind::Pie,
        "journey" => DiagramKind::Journey,
        "mindmap" => DiagramKind::Mindmap,
        "timeline" => DiagramKind::Timeline,
        "gitgraph" => DiagramKind::GitGraph,
        "quadrantchart" => DiagramKind::Quadrant,
        "xychart-beta" | "xychart" => DiagramKind::XyChart,
        "sankey-beta" | "sankey" => DiagramKind::Sankey,
        "requirementdiagram" => DiagramKind::Requirement,
        "c4context" | "c4container" | "c4component" | "c4dynamic" | "c4deployment" => {
            DiagramKind::C4
        }
        "block-beta" | "block" => DiagramKind::Block,
        "architecture-beta" => DiagramKind::Architecture,
        "packet-beta" | "packet" => DiagramKind::Packet,
        "kanban" => DiagramKind::Kanban,
        "radar-beta" | "radar" => DiagramKind::Radar,
        "treemap-beta" | "treemap" => DiagramKind::Treemap,
        "zenuml" => DiagramKind::ZenUml,
        _ => return None,
    };
    Some(kind)
}

/// First token of the first meaningful line, or `None`.
pub(crate) fn header_word(source: &str) -> Option<&str> {
    header_word_with_line(source).map(|(word, _)| word)
}

/// Like [`header_word`] but also returns the 1-based line number of the header.
pub(crate) fn header_word_with_line(source: &str) -> Option<(&str, usize)> {
    let mut seen_content = false;
    let mut in_front_matter = false;
    for (idx, line) in source.lines().enumerate() {
        let trimmed = line.trim();
        if in_front_matter {
            if trimmed == "---" {
                in_front_matter = false;
            }
            continue;
        }
        if trimmed.is_empty() || trimmed.starts_with("%%") {
            continue;
        }
        if !seen_content && trimmed == "---" {
            in_front_matter = true;
            seen_content = true;
            continue;
        }
        let word = trimmed
            .split(|c: char| c.is_whitespace() || c == ';' || c == ':')
            .find(|s| !s.is_empty())?;
        return Some((word, idx + 1));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_every_family_header() {
        let cases: &[(&str, DiagramKind)] = &[
            ("flowchart TD\n A-->B", DiagramKind::Flowchart),
            ("graph LR; A-->B", DiagramKind::Flowchart),
            ("sequenceDiagram\n A->>B: hi", DiagramKind::Sequence),
            ("classDiagram\n class A", DiagramKind::Class),
            ("stateDiagram-v2\n [*] --> a", DiagramKind::State),
            ("erDiagram\n A ||--o{ B : has", DiagramKind::Er),
            ("gantt\n title x", DiagramKind::Gantt),
            ("pie title Pets\n \"dogs\" : 3", DiagramKind::Pie),
            ("journey\n title My day", DiagramKind::Journey),
            ("mindmap\n root((x))", DiagramKind::Mindmap),
            ("timeline\n 2020 : a", DiagramKind::Timeline),
            ("gitGraph\n commit", DiagramKind::GitGraph),
            ("quadrantChart\n title q", DiagramKind::Quadrant),
            ("xychart-beta\n x-axis [a]", DiagramKind::XyChart),
            ("sankey-beta\n a,b,1", DiagramKind::Sankey),
            (
                "requirementDiagram\n requirement r {}",
                DiagramKind::Requirement,
            ),
            ("C4Context\n title c", DiagramKind::C4),
            ("block-beta\n columns 2", DiagramKind::Block),
            (
                "architecture-beta\n service a(server)[A]",
                DiagramKind::Architecture,
            ),
            ("packet-beta\n 0-15: \"x\"", DiagramKind::Packet),
            ("kanban\n Todo", DiagramKind::Kanban),
            ("radar-beta\n axis a", DiagramKind::Radar),
            ("treemap-beta\n \"a\": 1", DiagramKind::Treemap),
            ("zenuml\n A->B: x", DiagramKind::ZenUml),
        ];
        for (src, want) in cases {
            assert_eq!(detect_kind(src), Some(*want), "source: {src:?}");
        }
    }

    #[test]
    fn skips_comments_front_matter_and_directives() {
        let src = "---\ntitle: demo\n---\n%%{init: {\"theme\": \"dark\"}}%%\n%% note\n\nflowchart LR\n A-->B";
        assert_eq!(detect_kind(src), Some(DiagramKind::Flowchart));
        let (word, line) = header_word_with_line(src).unwrap_or(("", 0));
        assert_eq!(word, "flowchart");
        assert_eq!(line, 7);
    }

    #[test]
    fn unknown_or_empty_is_none() {
        assert_eq!(detect_kind(""), None);
        assert_eq!(detect_kind("   \n\n"), None);
        assert_eq!(detect_kind("hello world"), None);
        assert_eq!(detect_kind("%% only a comment"), None);
        assert_eq!(detect_kind("---\nunterminated front matter"), None);
    }

    #[test]
    fn header_is_case_insensitive_for_detection() {
        assert_eq!(
            detect_kind("SEQUENCEDIAGRAM\n"),
            Some(DiagramKind::Sequence)
        );
    }

    #[test]
    fn unsupported_kinds_are_flagged() {
        assert!(DiagramKind::Flowchart.is_supported());
        assert!(!DiagramKind::ZenUml.is_supported());
        assert!(!DiagramKind::Kanban.name().is_empty());
    }
}
