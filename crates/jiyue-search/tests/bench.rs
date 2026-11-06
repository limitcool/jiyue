//! Benchmark-style test: indexes 10,000 synthetic mixed zh/en notes (about 2 KB each) on disk,
//! then times searches. Ignored by default. Run with:
//!
//!     cargo test -p jiyue-search --release -- --ignored --nocapture
//!
//! Targets: index in under 60 s, search p95 under 30 ms (release build).

use std::fs;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use jiyue_search::{NoteDoc, SearchIndex};

const NOTE_COUNT: usize = 10_000;
const TARGET_BODY_BYTES: usize = 2_000;
const DAY_MS: i64 = 86_400_000;

const ZH_WORDS: &[&str] = &[
    "异步编程",
    "并发模型",
    "学习笔记",
    "读书摘要",
    "项目计划",
    "会议记录",
    "数据库",
    "索引结构",
    "内存管理",
    "编译器",
    "函数式",
    "类型系统",
    "网络协议",
    "分布式",
    "缓存策略",
    "性能优化",
    "测试驱动",
    "代码审查",
    "版本控制",
    "持续集成",
    "容器部署",
    "日志分析",
    "错误处理",
    "状态机",
    "消息队列",
    "调度算法",
    "文件系统",
    "图形界面",
    "用户体验",
    "产品设计",
    "市场调研",
    "竞品分析",
    "读书笔记",
    "周报总结",
    "想法记录",
    "待办事项",
    "旅行计划",
    "健康管理",
    "财务预算",
    "学习计划",
    "加密算法",
    "零知识证明",
    "向量检索",
    "语义搜索",
    "知识图谱",
    "笔记软件",
    "双向链接",
    "大纲整理",
];

const EN_WORDS: &[&str] = &[
    "rust",
    "async",
    "await",
    "tokio",
    "future",
    "thread",
    "memory",
    "ownership",
    "borrow",
    "lifetime",
    "trait",
    "generic",
    "iterator",
    "closure",
    "compiler",
    "cargo",
    "crate",
    "module",
    "database",
    "index",
    "query",
    "latency",
    "throughput",
    "cache",
    "network",
    "protocol",
    "server",
    "client",
    "deploy",
    "docker",
    "kubernetes",
    "metrics",
    "logging",
    "tracing",
    "markdown",
    "note",
    "link",
    "backlink",
    "vault",
    "search",
    "sync",
    "merge",
    "conflict",
    "snapshot",
    "review",
    "design",
    "system",
    "service",
    "plan",
    "meeting",
    "summary",
    "draft",
    "idea",
    "reading",
    "chapter",
    "quote",
];

const TAGS: &[&str] = &[
    "rust", "notes", "work", "ideas", "reading", "todo", "daily", "design",
];
const FOLDERS: &[&str] = &["想法", "读书", "工作", "日记", "项目/strata", "archive"];

/// Small deterministic PRNG (xorshift64*). No external dependency.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

fn synth_note(i: usize, rng: &mut Rng, now_ms: i64) -> NoteDoc {
    let folder = FOLDERS[rng.below(FOLDERS.len())];
    let title = format!(
        "{} {}",
        ZH_WORDS[rng.below(ZH_WORDS.len())],
        EN_WORDS[rng.below(EN_WORDS.len())]
    );
    let mut body = String::with_capacity(TARGET_BODY_BYTES + 64);
    while body.len() < TARGET_BODY_BYTES {
        if rng.below(3) == 0 {
            body.push_str(EN_WORDS[rng.below(EN_WORDS.len())]);
            body.push(' ');
        } else {
            body.push_str(ZH_WORDS[rng.below(ZH_WORDS.len())]);
            if rng.below(4) == 0 {
                body.push('，');
            }
        }
        if rng.below(12) == 0 {
            body.push_str("。\n");
        }
    }
    let tag_count = rng.below(4);
    let tags = (0..tag_count)
        .map(|_| TAGS[rng.below(TAGS.len())].to_string())
        .collect();
    NoteDoc {
        path: format!("{folder}/note-{i:05}.md"),
        title,
        body,
        tags,
        mtime_ms: now_ms - rng.below(730) as i64 * DAY_MS,
    }
}

const QUERIES: &[&str] = &[
    "异步编程",
    "rust async",
    "编程",
    "步编",
    "rust 异步编程",
    "\"异步编程\"",
    "\"学习笔记\" -草稿",
    "tag:rust 数据库",
    "path:想法/",
    "file:读书",
    "title:笔记",
    "索引 OR 缓存",
    "tokio future",
    "零知识证明",
    "并发 模型 内存",
    "database latency",
    "ownership",
    "双向链接 backlink",
    "not-in-corpus-zzz",
    "用户体验 设计",
];

fn percentile(sorted: &[Duration], p: f64) -> Duration {
    if sorted.is_empty() {
        return Duration::ZERO;
    }
    let rank = ((sorted.len() as f64 - 1.0) * p).round() as usize;
    sorted[rank.min(sorted.len() - 1)]
}

#[test]
#[ignore = "benchmark: run with --release --ignored --nocapture"]
fn benchmark_ten_thousand_notes() {
    let dir: PathBuf =
        std::env::temp_dir().join(format!("jiyue-search-bench-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);

    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| i64::try_from(d.as_millis()).unwrap_or(0))
        .unwrap_or(0);
    let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
    let docs: Vec<NoteDoc> = (0..NOTE_COUNT)
        .map(|i| synth_note(i, &mut rng, now_ms))
        .collect();
    let corpus_bytes: usize = docs.iter().map(|d| d.body.len()).sum();

    let mut index = SearchIndex::open(&dir).expect("open bench index");
    let started = Instant::now();
    index.rebuild(docs.into_iter()).expect("rebuild");
    let index_time = started.elapsed();
    assert_eq!(index.count(), NOTE_COUNT);

    // Warm up the reader, then measure several rounds of every query.
    for q in QUERIES {
        let _ = index.search(q, 20);
    }
    let mut latencies: Vec<Duration> = Vec::new();
    let mut total_hits = 0usize;
    for _round in 0..25 {
        for q in QUERIES {
            let t = Instant::now();
            let hits = index.search(q, 20);
            latencies.push(t.elapsed());
            total_hits += hits.len();
        }
    }
    latencies.sort();
    let p50 = percentile(&latencies, 0.50);
    let p95 = percentile(&latencies, 0.95);
    let max = latencies.last().copied().unwrap_or(Duration::ZERO);

    println!("notes:            {NOTE_COUNT}");
    println!(
        "corpus body:      {:.1} MB",
        corpus_bytes as f64 / 1_048_576.0
    );
    println!("index time:       {:.2} s", index_time.as_secs_f64());
    println!("queries measured: {}", latencies.len());
    println!("search p50:       {:.2} ms", p50.as_secs_f64() * 1000.0);
    println!("search p95:       {:.2} ms", p95.as_secs_f64() * 1000.0);
    println!("search max:       {:.2} ms", max.as_secs_f64() * 1000.0);
    println!(
        "avg hits/query:   {:.1}",
        total_hits as f64 / latencies.len() as f64
    );

    drop(index);
    let _ = fs::remove_dir_all(&dir);

    assert!(
        index_time < Duration::from_secs(60),
        "index took {index_time:?}"
    );
    assert!(p95 < Duration::from_millis(30), "search p95 {p95:?}");
}
