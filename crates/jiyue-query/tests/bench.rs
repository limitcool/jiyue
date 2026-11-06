//! Benchmark: 10,000 notes, a typical filter + sort + limit query.
//!
//! Run with: `cargo test -p jiyue-query --release -- --ignored --nocapture`
//! In release builds the median query time is asserted to stay under 20 ms.

use std::time::Instant;

use jiyue_query::date::{days_from_civil, format_date};
use jiyue_query::{PropertyIndex, QueryContext, execute, parse};

const NOTES: usize = 10_000;

fn build_vault(n: usize) -> (PropertyIndex, usize) {
    let folders = ["项目", "日记", "资料/深度", "inbox"];
    let statuses = ["进行中", "完成", "待办"];
    let tags = ["rust", "go", "python"];
    let base_day = days_from_civil(2026, 1, 1);
    let mut idx = PropertyIndex::new();
    let mut bytes = 0usize;
    for i in 0..n {
        let folder = folders.get(i % folders.len()).copied().unwrap_or("inbox");
        let status = statuses.get(i % statuses.len()).copied().unwrap_or("待办");
        let tag = tags.get(i % tags.len()).copied().unwrap_or("rust");
        let due = format_date(base_day + (i as i64 % 400));
        let text = format!(
            "---\ntitle: 笔记 {i}\nstatus: {status}\ndue: {due}\npriority: {}\ntags: [{tag}, misc]\n---\n# 笔记 {i}\n这是正文 with words and 中文 [[笔记 {}]] and [[Other {}]] #{tag}/sub\nmore text here to make the body a little longer for realism.\n",
            i % 5,
            i.saturating_sub(1),
            i % 97,
        );
        bytes += text.len();
        let path = format!("{folder}/note-{i:05}.md");
        let mtime = 1_700_000_000_000i64 + i as i64 * 60_000;
        idx.upsert(&path, &text, mtime, mtime - 1000);
    }
    (idx, bytes)
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted.get(i).copied().unwrap_or(0)
}

#[test]
#[ignore]
fn bench_10k_filter_sort_limit() {
    let t0 = Instant::now();
    let (idx, text_bytes) = build_vault(NOTES);
    let build = t0.elapsed();
    println!(
        "index build: {NOTES} notes in {:.1} ms ({text_bytes} source bytes)",
        build.as_secs_f64() * 1000.0
    );
    println!(
        "approx index memory: {:.2} MB",
        idx.approx_memory_bytes() as f64 / 1_048_576.0
    );

    // 2026-11-04 12:00 UTC.
    let now = days_from_civil(2026, 11, 4) * 86_400_000 + 12 * 3_600_000;
    let ctx = QueryContext::new(now, 0);
    let src = "from folder(\"项目\") and tag(\"rust\") where status = \"进行中\" and due < today() + 7d select name, status, due, tags sort due asc, mtime desc limit 50";

    let parse_start = Instant::now();
    let q = match parse(src) {
        Ok(q) => q,
        Err(e) => panic!("benchmark query does not parse: {e}"),
    };
    println!("parse: {:.1} us", parse_start.elapsed().as_secs_f64() * 1e6);

    let mut times: Vec<u64> = Vec::new();
    let mut last_total = 0usize;
    let mut last_shown = 0usize;
    for _ in 0..50 {
        let start = Instant::now();
        let r = execute(&idx, &q, &ctx);
        times.push(start.elapsed().as_micros() as u64);
        last_total = r.total;
        last_shown = r.shown;
    }
    times.sort_unstable();
    let median_us = percentile(&times, 0.5);
    println!(
        "filter+sort+limit over {NOTES} notes: matched {last_total}, shown {last_shown}; \
         median {:.2} ms, p95 {:.2} ms, min {:.2} ms, max {:.2} ms (50 runs)",
        median_us as f64 / 1000.0,
        percentile(&times, 0.95) as f64 / 1000.0,
        times.first().copied().unwrap_or(0) as f64 / 1000.0,
        times.last().copied().unwrap_or(0) as f64 / 1000.0,
    );

    let group_q = match parse(
        "from folder(\"项目\") where priority >= 2 group by status select name, count() as n, avg(priority) as p sort due desc",
    ) {
        Ok(q) => q,
        Err(e) => panic!("group query does not parse: {e}"),
    };
    let start = Instant::now();
    let r = execute(&idx, &group_q, &ctx);
    println!(
        "group + aggregates: {} groups, matched {} in {:.2} ms",
        r.groups.len(),
        r.total,
        start.elapsed().as_secs_f64() * 1000.0
    );

    if !cfg!(debug_assertions) {
        assert!(
            median_us < 20_000,
            "median query time {} us exceeds the 20 ms budget",
            median_us
        );
    }
}
