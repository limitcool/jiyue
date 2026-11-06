//! Renders sample diagrams in the light and dark themes to PNG files for visual review.
//!
//! Usage: `cargo run -p jiyue-diagram --example diagram_shots -- <output-dir>`

use jiyue_diagram::{DiagramTheme, Rgba8Image, rasterize, render_mermaid};

const SAMPLES: &[(&str, &str)] = &[
    (
        "flowchart_cjk",
        "flowchart LR\n  A[开始] --> B{是否登录?}\n  B -->|是| C[进入首页 Home]\n  B -->|否| D[跳转登录页面]\n  D --> E[输入账号密码] --> B",
    ),
    (
        "flowchart_long_labels",
        "flowchart TD\n  A[这是一个非常长的中文节点标签用于测试裁剪与换行 long mixed label] --> B[短]\n  B --> C[Jiyue 同步: S3 + WebDAV 三方合并 conflict copy]",
    ),
    (
        "sequence_cjk",
        "sequenceDiagram\n  participant U as 用户\n  participant S as Jiyue Server\n  U->>S: 同步请求 sync\n  S-->>U: 返回 200 OK\n  Note over U,S: 冲突时生成副本",
    ),
    (
        "class_mix",
        "classDiagram\n  class Note {\n    +String path\n    +String title\n    +save()\n  }\n  class Vault\n  Vault \"1\" --> \"*\" Note : 包含",
    ),
    (
        "state_cjk",
        "stateDiagram-v2\n  [*] --> 草稿\n  草稿 --> 已保存 : 保存\n  已保存 --> 已同步 : 上传成功\n  已同步 --> 冲突 : 远端变化\n  冲突 --> 已同步 : 合并",
    ),
    (
        "pie_cjk",
        "pie title 笔记标签分布\n  \"工作\" : 42\n  \"学习\" : 30\n  \"生活\" : 18\n  \"other\" : 10",
    ),
    (
        "gantt_mixed",
        "gantt\n  title 发布计划 Release plan\n  dateFormat YYYY-MM-DD\n  section 开发\n  设计 :done, d1, 2026-01-01, 5d\n  实现 Implement :active, d2, after d1, 8d\n  section 测试\n  测试 QA :d3, after d2, 4d",
    ),
    (
        "mindmap_cjk",
        "mindmap\n  root((Jiyue))\n    笔记\n      Markdown\n      双链\n    同步\n      S3\n      WebDAV\n    AI\n      RAG",
    ),
    (
        "journey_cjk",
        "journey\n  title 我的一天\n  section 早上\n    起床: 3: 我\n    写笔记: 5: 我\n  section 晚上\n    复盘: 4: 我",
    ),
    (
        "timeline_cjk",
        "timeline\n  title 项目里程碑\n  2025 : 立项\n  2026 : 第一波并行开发\n        : 同步与搜索\n  2027 : 发布 v1.0",
    ),
    (
        "gitgraph_mixed",
        "gitGraph\n  commit id: \"init\"\n  branch 功能\n  checkout 功能\n  commit id: \"add 搜索\"\n  checkout main\n  merge 功能\n  commit",
    ),
    (
        "er_cjk",
        "erDiagram\n  NOTE ||--o{ TAG : 拥有\n  NOTE {\n    string path\n    string 标题\n  }\n  TAG {\n    string name\n  }",
    ),
    (
        "quadrant_cjk",
        "quadrantChart\n  title 优先级矩阵\n  x-axis 低 --> 高\n  y-axis 次要 --> 重要\n  quadrant-1 立即做\n  quadrant-2 计划\n  quadrant-3 委派\n  quadrant-4 删除\n  同步: [0.8, 0.9]\n  搜索: [0.6, 0.7]",
    ),
    (
        "xychart",
        "xychart-beta\n  title \"月度笔记数\"\n  x-axis [一月, 二月, 三月, 四月]\n  y-axis \"数量\" 0 --> 100\n  bar [30, 55, 70, 42]\n  line [28, 50, 66, 40]",
    ),
    (
        "sankey",
        "sankey-beta\n\nInbox,Work,40\nInbox,Personal,20\nWork,Done,30\nPersonal,Done,15",
    ),
    (
        "requirement",
        "requirementDiagram\n  requirement 同步需求 {\n    id: 1\n    text: 必须支持离线编辑\n    risk: high\n    verifymethod: test\n  }\n  element 客户端 {\n    type: app\n  }\n  客户端 - satisfies -> 同步需求",
    ),
    (
        "block_cjk",
        "block-beta\n  columns 3\n  A[\"编辑器\"] B[\"预览\"] C[\"AI 面板\"]\n  D[\"同步\"]:2 E[\"搜索\"]",
    ),
    (
        "kanban",
        "kanban\n  Todo\n    [写文档]\n    [修 bug]\n  Doing\n    [同步引擎]",
    ),
    (
        "radar",
        "radar-beta\n  axis a[\"速度\"], b[\"稳定\"], c[\"易用\"]\n  curve x[\"Jiyue\"]{3, 4, 5}\n  curve y[\"Other\"]{4, 2, 3}",
    ),
    (
        "treemap",
        "treemap-beta\n  \"笔记\":\n    \"工作\": 40\n    \"学习\": 30\n  \"其他\": 10",
    ),
    (
        "packet",
        "packet-beta\n  0-15: \"源端口\"\n  16-31: \"目标端口\"\n  32-63: \"序号\"",
    ),
    (
        "architecture",
        "architecture-beta\n  group api(cloud)[API]\n  service db(database)[数据库] in api\n  service srv(server)[服务器] in api\n  srv:R --> L:db",
    ),
];

fn main() {
    let out = std::env::args().nth(1).unwrap_or_else(|| ".".into());
    let themes = [
        ("light", DiagramTheme::light()),
        ("dark", DiagramTheme::dark()),
    ];
    for (name, src) in SAMPLES {
        for (theme_name, theme) in &themes {
            let file = format!("{out}/{name}_{theme_name}.png");
            match render_mermaid(src, theme) {
                Ok(diagram) => match rasterize(&diagram, 2.0) {
                    Ok(img) => {
                        save(&img, &file);
                        println!("OK   {file} ({}x{})", img.width, img.height);
                    }
                    Err(e) => println!("RAST {name} {theme_name}: {e}"),
                },
                Err(e) => println!("ERR  {name} {theme_name}: {e}"),
            }
        }
    }
}

fn save(img: &Rgba8Image, path: &str) {
    if let Some(buf) = image::RgbaImage::from_raw(img.width, img.height, img.data.clone()) {
        let _ = buf.save(path);
    }
}
