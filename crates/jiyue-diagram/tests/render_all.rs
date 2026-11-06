//! End-to-end tests: every supported diagram family, in both themes, must render to
//! safe, well-formed SVG that rasterises with visible content.

use std::time::{Duration, Instant};

use jiyue_diagram::{
    DiagramError, DiagramKind, DiagramTheme, RenderedDiagram, detect_kind, rasterize,
    render_mermaid, render_mermaid_with_timeout,
};

/// Asserts that `svg` is well-formed XML and has no active or external content.
fn assert_safe_svg(svg: &str) {
    let doc = roxmltree::Document::parse(svg).unwrap_or_else(|e| panic!("not XML: {e}\n{svg}"));
    assert_eq!(doc.root_element().tag_name().name(), "svg");
    let lower = svg.to_ascii_lowercase();
    for banned in [
        "<script",
        "foreignobject",
        "javascript:",
        "onload=",
        "onclick=",
        "onerror=",
        "href=\"http",
        "href=\"https",
        "xlink:href=\"http",
        "url(http",
        "url('http",
        "url(\"http",
        "@import",
        "<iframe",
        "<object",
        "<embed",
        "<animate",
        "<set ",
    ] {
        assert!(!lower.contains(banned), "SVG contains banned `{banned}`");
    }
}

/// Number of distinct RGBA colours in the rasterised output (capped for speed).
fn distinct_colours(d: &RenderedDiagram) -> usize {
    let img = rasterize(d, 1.0).unwrap_or_else(|e| panic!("rasterize failed: {e}"));
    let mut seen = std::collections::HashSet::new();
    for px in img.data.as_chunks::<4>().0 {
        seen.insert([px[0], px[1], px[2], px[3]]);
        if seen.len() > 64 {
            break;
        }
    }
    seen.len()
}

fn check_kind(source: &str, expected: DiagramKind) {
    assert_eq!(
        detect_kind(source),
        Some(expected),
        "detection for {source:?}"
    );
    for theme in [DiagramTheme::light(), DiagramTheme::dark()] {
        let d = render_mermaid(source, &theme)
            .unwrap_or_else(|e| panic!("{} failed in {:?}: {e}", expected.name(), theme.dark));
        assert_safe_svg(&d.svg);
        assert!(
            d.width > 0.0 && d.height > 0.0,
            "zero size for {}",
            expected.name()
        );
        assert!(
            distinct_colours(&d) >= 3,
            "{} rasterised to a blank-looking image",
            expected.name()
        );
    }
}

macro_rules! kind_tests {
    ($( $test:ident => $kind:ident, $src:expr; )*) => {
        $(
            #[test]
            fn $test() {
                check_kind($src, DiagramKind::$kind);
            }
        )*
    };
}

kind_tests! {
    renders_flowchart => Flowchart, "flowchart LR\n  A[开始] --> B{是否登录?}\n  B -->|是| C[Home]";
    renders_graph_keyword => Flowchart, "graph TD\n  A-->B";
    renders_sequence => Sequence, "sequenceDiagram\n  participant U as 用户\n  participant S as Server\n  U->>S: 同步 sync\n  S-->>U: 200 OK";
    renders_class => Class, "classDiagram\n  class Note {\n    +String path\n  }\n  class Vault\n  Vault \"1\" --> \"*\" Note : 包含";
    renders_state => State, "stateDiagram-v2\n  [*] --> 草稿\n  草稿 --> 已保存 : 保存\n  已保存 --> [*]";
    renders_er => Er, "erDiagram\n  NOTE ||--o{ TAG : 拥有\n  NOTE {\n    string path\n  }";
    renders_gantt => Gantt, "gantt\n  title 发布计划\n  dateFormat YYYY-MM-DD\n  section 开发\n  设计 :done, d1, 2026-01-01, 5d\n  实现 :d2, after d1, 3d";
    renders_pie => Pie, "pie title 笔记标签\n  \"工作\" : 42\n  \"学习\" : 30\n  \"other\" : 10";
    renders_journey => Journey, "journey\n  title 我的一天\n  section 早上\n    起床: 3: 我\n    写笔记: 5: 我";
    renders_mindmap => Mindmap, "mindmap\n  root((Jiyue))\n    笔记\n      Markdown\n    同步\n      S3";
    renders_timeline => Timeline, "timeline\n  title 里程碑\n  2025 : 立项\n  2026 : 发布 v1.0";
    renders_gitgraph => GitGraph, "gitGraph\n  commit id: \"init\"\n  branch dev\n  checkout dev\n  commit\n  checkout main\n  merge dev";
    renders_quadrant => Quadrant, "quadrantChart\n  title 优先级\n  x-axis 低 --> 高\n  y-axis 次要 --> 重要\n  同步: [0.8, 0.9]";
    renders_xychart => XyChart, "xychart-beta\n  title \"月度\"\n  x-axis [一月, 二月, 三月]\n  bar [30, 55, 70]";
    renders_sankey => Sankey, "sankey-beta\n\nInbox,Work,40\nWork,Done,30";
    renders_requirement => Requirement, "requirementDiagram\n  requirement 同步需求 {\n    id: 1\n    text: 离线\n  }\n  element 客户端 {\n    type: app\n  }\n  客户端 - satisfies -> 同步需求";
    renders_block => Block, "block-beta\n  columns 2\n  a[\"编辑器\"] b[\"预览\"]";
    renders_architecture => Architecture, "architecture-beta\n  group api(cloud)[API]\n  service db(database)[数据库] in api";
    renders_packet => Packet, "packet-beta\n  0-15: \"源端口\"\n  16-31: \"目标端口\"";
    renders_kanban => Kanban, "kanban\n  Todo\n    [写文档]\n  Doing\n    [同步]";
    renders_radar => Radar, "radar-beta\n  axis a, b, c\n  curve x{1, 2, 3}";
    renders_treemap => Treemap, "treemap-beta\n  \"笔记\":\n    \"工作\": 40\n    \"学习\": 30";
}

#[test]
fn chinese_labels_appear_in_svg_text() {
    let d = render_mermaid(
        "flowchart LR\n A[开始同步] --> B[冲突副本]",
        &DiagramTheme::light(),
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert!(d.svg.contains("开始同步"));
    assert!(d.svg.contains("冲突副本"));
}

/// Node boxes must be wide enough for their CJK labels (about one em per glyph),
/// otherwise text overflows and is clipped in the preview.
#[test]
fn cjk_labels_fit_inside_their_boxes() {
    let label = "这是一个用于测试中文宽度的很长标签";
    let src = format!("flowchart LR\n  A[{label}] --> B[短]");
    let theme = DiagramTheme::light();
    let d = render_mermaid(&src, &theme).unwrap_or_else(|e| panic!("{e}"));
    let doc = roxmltree::Document::parse(&d.svg).unwrap_or_else(|e| panic!("{e}"));
    let widest_rect = doc
        .descendants()
        .filter(|n| n.tag_name().name() == "rect")
        .filter_map(|n| n.attribute("width")?.parse::<f32>().ok())
        .fold(0.0_f32, f32::max);
    let cjk_chars = label.chars().count() as f32;
    let needed = cjk_chars * theme.font_size * 0.9;
    assert!(
        widest_rect >= needed,
        "widest box {widest_rect}px is narrower than {needed}px needed for {cjk_chars} CJK glyphs"
    );
}

#[test]
fn long_mixed_labels_wrap_and_render() {
    let src = "flowchart TD\n  A[这是一个非常长的中文节点标签用于测试裁剪与换行 long mixed label] --> B[短]";
    let d = render_mermaid(src, &DiagramTheme::dark()).unwrap_or_else(|e| panic!("{e}"));
    assert_safe_svg(&d.svg);
    assert!(d.svg.contains("long mixed label") || d.svg.contains("long"));
}

#[test]
fn theme_colours_are_applied() {
    let src = "pie title 占比\n  \"甲\" : 3\n  \"乙\" : 1";
    let light = render_mermaid(src, &DiagramTheme::light()).unwrap_or_else(|e| panic!("{e}"));
    let dark = render_mermaid(src, &DiagramTheme::dark()).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(light.svg, dark.svg);
    assert!(dark.svg.contains("#1E1E1E"), "dark background missing");
    assert!(light.svg.contains("#FFFFFF"), "light background missing");
    // Accent-derived pie colours must differ between two accents.
    let mut blue = DiagramTheme::light();
    blue.accent = "#2563EB".into();
    let mut red = DiagramTheme::light();
    red.accent = "#DC2626".into();
    let b = render_mermaid(src, &blue).unwrap_or_else(|e| panic!("{e}"));
    let r = render_mermaid(src, &red).unwrap_or_else(|e| panic!("{e}"));
    assert_ne!(b.svg, r.svg, "accent did not influence the palette");
}

#[test]
fn foreground_colour_reaches_text() {
    let mut theme = DiagramTheme::light();
    theme.foreground = "#AB1234".into();
    let d = render_mermaid("flowchart LR\n A[hi] --> B[there]", &theme)
        .unwrap_or_else(|e| panic!("{e}"));
    assert!(d.svg.to_ascii_uppercase().contains("#AB1234"));
}

#[test]
fn hostile_theme_values_never_reach_the_svg() {
    let theme = DiagramTheme {
        background: "\"><script>alert(1)</script>".into(),
        foreground: "red\" onload=\"x".into(),
        muted: "url(http://evil.example/x)".into(),
        accent: "<svg".into(),
        node_fill: "javascript:alert(2)".into(),
        node_stroke: "#123456".into(),
        line: "#654321".into(),
        font_family: "Arial\"></style><script>x</script>".into(),
        font_size: f32::INFINITY,
        dark: true,
    };
    let d = render_mermaid("flowchart LR\n A-->B", &theme).unwrap_or_else(|e| panic!("{e}"));
    assert_safe_svg(&d.svg);
    assert!(!d.svg.contains("onload"));
}

#[test]
fn rendering_is_deterministic() {
    let src = "sequenceDiagram\n  A->>B: 你好\n  B-->>A: 再见";
    let theme = DiagramTheme::dark();
    let a = render_mermaid(src, &theme).unwrap_or_else(|e| panic!("{e}"));
    let b = render_mermaid(src, &theme).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(a, b);
}

#[test]
fn empty_input_is_a_parse_error() {
    for src in ["", "   ", "\n\n\t\n"] {
        assert!(matches!(
            render_mermaid(src, &DiagramTheme::light()),
            Err(DiagramError::Parse { .. })
        ));
    }
}

#[test]
fn garbage_input_is_a_friendly_error_never_a_panic() {
    let garbage = [
        "\0\0\0",
        "flowchart",
        "graph",
        "pie",
        "sequenceDiagram\n A->>",
        "classDiagram\n class {{{",
        "🙂🙂🙂🙂",
        "%%{init: {bad json}}%%\nflowchart LR\nA-->B",
        "%%{init: {bad json}}%%",
        "stateDiagram-v2\n [*] -->",
        "erDiagram\n A ||--o{",
        "gantt\n dateFormat YYYY-MM-DD\n t :x, notadate, 3d",
        "flowchart LR\n A[",
        "flowchart LR\n subgraph s\n A-->B",
    ];
    for src in garbage {
        match render_mermaid(src, &DiagramTheme::light()) {
            Ok(d) => assert_safe_svg(&d.svg),
            Err(DiagramError::Parse { message, .. }) => assert!(!message.is_empty()),
            Err(DiagramError::Unsupported(_) | DiagramError::Render(_)) => {}
            Err(other) => panic!("unexpected error for {src:?}: {other}"),
        }
    }
    assert!(matches!(
        render_mermaid("\0\0\0", &DiagramTheme::light()),
        Err(DiagramError::Parse { .. })
    ));
}

#[test]
fn deeply_nested_brackets_do_not_crash() {
    let src = format!("flowchart LR\n A{}", "[(".repeat(3000));
    let result = render_mermaid(&src, &DiagramTheme::light());
    assert!(result.is_ok() || result.is_err());
}

#[test]
fn oversized_input_is_rejected_fast() {
    let src = format!("flowchart LR\n{}", "A-->B\n".repeat(25_000));
    assert!(src.len() > 100 * 1024);
    let started = Instant::now();
    assert_eq!(
        render_mermaid(&src, &DiagramTheme::light()),
        Err(DiagramError::TooLarge)
    );
    assert!(started.elapsed() < Duration::from_millis(200));
}

#[test]
fn unknown_header_reports_line_number() {
    let err = render_mermaid("%% title\n\nwhat is this", &DiagramTheme::light());
    match err {
        Err(DiagramError::Parse { line, .. }) => assert_eq!(line, Some(3)),
        other => panic!("unexpected {other:?}"),
    }
}

#[test]
fn zero_timeout_reports_timeout() {
    let src = "flowchart LR\n A-->B";
    assert_eq!(
        render_mermaid_with_timeout(src, &DiagramTheme::light(), Duration::ZERO),
        Err(DiagramError::Timeout)
    );
}

#[test]
fn generous_timeout_succeeds() {
    let src = "flowchart LR\n A-->B";
    let d = render_mermaid_with_timeout(src, &DiagramTheme::light(), Duration::from_secs(30));
    assert!(d.is_ok());
}

#[test]
fn svg_is_xml_valid_for_every_theme_variant() {
    let mut transparent = DiagramTheme::dark();
    transparent.background = "transparent".into();
    for theme in [DiagramTheme::light(), DiagramTheme::dark(), transparent] {
        let d = render_mermaid("flowchart LR\n A[你好] -->|是| B[world]", &theme)
            .unwrap_or_else(|e| panic!("{e}"));
        assert_safe_svg(&d.svg);
    }
}

#[test]
fn rasterised_scale_two_has_double_pixels() {
    let d = render_mermaid("pie\n \"a\" : 1\n \"b\" : 1", &DiagramTheme::light())
        .unwrap_or_else(|e| panic!("{e}"));
    let one = rasterize(&d, 1.0).unwrap_or_else(|e| panic!("{e}"));
    let two = rasterize(&d, 2.0).unwrap_or_else(|e| panic!("{e}"));
    assert!(two.width >= one.width * 2 - 1 && two.width <= one.width * 2 + 1);
    assert!(two.height >= one.height * 2 - 1 && two.height <= one.height * 2 + 1);
}

kind_tests! {
    renders_c4 => C4, "C4Context\n  title 系统上下文\n  Person(user, \"用户\")\n  System(jiyue, \"Jiyue\")\n  Rel(user, jiyue, \"使用\")";
}
