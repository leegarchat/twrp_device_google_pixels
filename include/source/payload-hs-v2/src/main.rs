//! payload-hs CLI: probe | extract. Host test harness; the recovery
//! side (recovery-pixel-boot install_rom) will reuse manifest/extract
//! with block devices instead of files.

use payload_hs::{extract, manifest, zipx};
use std::time::Instant;

fn usage() -> ! {
    eprintln!(
        "usage:\n  payload-hs probe --zip PATH\n  payload-hs plan --zip PATH [--slot a|b] [--sysroot DIR]\n  payload-hs extract --zip PATH --out DIR [--parts a,b] [--threads N]"
    );
    std::process::exit(2);
}

fn flag(args: &[String], name: &str) -> Option<String> {
    // Both "--flag value" and "--flag=value" forms (the installer execs
    // the glued form; humans type the split one).
    let glued = format!("{name}=");
    let mut it = args.iter().peekable();
    while let Some(a) = it.next() {
        if a == name {
            return it.peek().map(|s| s.to_string());
        }
        if let Some(v) = a.strip_prefix(&glued) {
            return Some(v.to_string());
        }
    }
    None
}

fn fmt_gb(n: u64) -> String {
    format!("{:.1} MB", n as f64 / 1e6)
}

fn load(zip: &str) -> (zipx::ZipLayout, Vec<manifest::Part>) {
    let layout = zipx::probe(zip).unwrap_or_else(|e| {
        eprintln!("zip probe failed: {e}");
        std::process::exit(1);
    });
    let parts = manifest::parse(&layout.manifest).unwrap_or_else(|e| {
        eprintln!("manifest parse failed: {e}");
        std::process::exit(1);
    });
    (layout, parts)
}

fn cmd_probe(args: &[String]) {
    let zip = flag(args, "--zip").unwrap_or_else(|| usage());
    let t0 = Instant::now();
    let (layout, parts) = load(&zip);
    println!(
        "zip: {} ({} bytes)  payload_off={}  manifest={} bytes  bs={}",
        zip, layout.file_len, layout.payload_off, layout.manifest_len, layout.block_size
    );
    let mut tot_out = 0u64;
    let mut tot_dl = 0u64;
    for p in &parts {
        let hist: Vec<String> = p
            .hist()
            .iter()
            .map(|(t, n)| format!("{}x{}", manifest::op_name(*t), n))
            .collect();
        let out = p.out_size(layout.block_size);
        let dl = p.dl_size();
        tot_out += out;
        tot_dl += dl;
        println!(
            "  {:16} ops={:<4} {:48} out={:<10} dl={:<10} old_info={} new_size={} hash={}",
            p.name,
            p.ops.len(),
            hist.join("+"),
            fmt_gb(out),
            fmt_gb(dl),
            p.has_old_info,
            if p.new_size > 0 {
                fmt_gb(p.new_size)
            } else {
                "-".into()
            },
            if p.new_hash.len() == 32 { "yes" } else { "no" },
        );
    }
    println!(
        "parts={} out_total={} dl_total={} probe_ms={}",
        parts.len(),
        fmt_gb(tot_out),
        fmt_gb(tot_dl),
        t0.elapsed().as_millis()
    );
    match manifest::verdict_full(&parts) {
        Ok(()) => println!("VERDICT: FULL OTA -> fast path eligible"),
        Err(r) => println!("VERDICT: INCREMENTAL-ish ({r}) -> stock update_engine path"),
    }
}

fn cmd_plan(args: &[String]) {
    let zip = flag(args, "--zip").unwrap_or_else(|| usage());
    // Target slot: opposite of the booted one. In recovery this comes from
    // ro.boot.slot_suffix; on host pass --slot explicitly.
    let slot = flag(args, "--slot").unwrap_or_else(|| "b".to_string());
    let sysroot = flag(args, "--sysroot").unwrap_or_else(|| "/system/bin".to_string());
    let live_json = flag(args, "--live");
    let (layout, parts) = load(&zip);

    println!("== payload: {zip} -> slot _{slot} ==");
    match manifest::verdict_full(&parts) {
        Ok(()) => println!("ota kind: FULL"),
        Err(r) => println!("ota kind: INCREMENTAL-ish ({r})"),
    }

    // --- dynamic groups from the manifest itself ---
    let dm = manifest::parse_dynmeta(&layout.manifest).unwrap_or_else(|e| {
        eprintln!("dynmeta parse failed: {e}");
        std::process::exit(1);
    });
    let mut in_group = std::collections::HashMap::new();
    match &dm {
        Some(m) => {
            println!(
                "dynamic: snapshot={} vabc={} groups={}",
                m.snapshot,
                m.vabc,
                m.groups.len()
            );
            for g in &m.groups {
                println!(
                    "  group {:12} budget={:<10} members={}",
                    g.name,
                    fmt_gb(g.size),
                    g.parts.join(",")
                );
                for p in &g.parts {
                    in_group.insert(p.clone(), g.name.clone());
                }
            }
        }
        None => println!("dynamic: none (all partitions are physical)"),
    }

    // --- routing: SUPER (logical) vs BLOCK (physical) ---
    println!("-- SUPER (logical, via group rebuild) --");
    let mut super_total = 0u64;
    for p in &parts {
        if let Some(g) = in_group.get(&p.name) {
            let out = p.out_size(layout.block_size);
            super_total += out;
            println!("  {:16} group={:<12} out={}", p.name, g, fmt_gb(out));
        }
    }
    println!("-- BLOCK (physical, direct write) --");
    for p in &parts {
        if !in_group.contains_key(&p.name) {
            let out = p.out_size(layout.block_size);
            println!(
                "  {:16} -> /dev/block/by-name/{}_{}  out={}",
                p.name, p.name, slot,
                fmt_gb(out)
            );
        }
    }
    println!("super total (expanded): {}", fmt_gb(super_total));

    // --- phase-2 fit: does the payload fit the opposite slot? ---
    // Live state comes from `siw info` on device (JSON). Without it we
    // can only report the manifest side.
    println!("-- fit (phase 2) --");
    if let Some(src) = live_json {
        let raw = if let Some(p) = src.strip_prefix('@') {
            std::fs::read_to_string(p).unwrap_or_else(|e| {
                eprintln!("read {p}: {e}");
                std::process::exit(1);
            })
        } else {
            src
        };
        let live: payload_hs::layout::LiveState = serde_json::from_str(&raw)
            .unwrap_or_else(|e| {
                eprintln!("bad --live JSON (want {{\"super_size\":N,\"current\":{{}},\"target\":{{}}}}): {e}");
                std::process::exit(1);
            });
        cmd_fit(&dm, &parts, layout.block_size, &live);
    } else {
        println!("  no --live state: manifest side only (group budgets above).");
        println!("  on device: siw info /dev/block/by-name/super -> JSON -> --live @file");
    }

    // --- helpers: binary call with loud fallback to the stock path ---
    println!("-- helpers (sysroot={sysroot}) --");
    let mut missing = Vec::new();
    for h in ["siw", "lptools_new"] {
        let path = format!("{sysroot}/{h}");
        let found = std::path::Path::new(&path).exists();
        println!("  {h:<12} {path}  {}", if found { "FOUND" } else { "MISSING" });
        if !found {
            missing.push(h);
        }
    }
    let full = manifest::verdict_full(&parts).is_ok();
    if full && missing.is_empty() {
        println!("DECISION: FAST (direct-fd write + one-shot group rebuild)");
    } else {
        let why = if !full {
            "not a full OTA".to_string()
        } else {
            format!("helpers missing: {}", missing.join(","))
        };
        println!("DECISION: STOCK FALLBACK ({why}) -> update_engine path");
    }
}

fn cmd_fit(
    dm: &Option<manifest::DynMeta>,
    parts: &[manifest::Part],
    bs: u64,
    live: &payload_hs::layout::LiveState,
) {
    use payload_hs::layout as L;
    let out_of = |name: &str| -> u64 {
        parts
            .iter()
            .find(|p| p.name == name)
            .map(|p| p.out_size(bs))
            .unwrap_or(0)
    };
    let mut per_group = Vec::new();
    match dm {
        Some(m) => {
            for g in &m.groups {
                let req: Vec<(String, u64)> = g
                    .parts
                    .iter()
                    .map(|n| (n.clone(), out_of(n)))
                    .collect();
                let plan = L::plan_group(&req, &g.parts, g.size, live);
                println!("  group {:24} need={:<10} budget={:<10} -> {}",
                    g.name, fmt_gb(req.iter().map(|(_, s)| s).sum()),
                    fmt_gb(g.size), plan.code());
                per_group.push(plan);
            }
        }
        None => println!("  no dynamic groups: nothing to fit"),
    }
    let final_ = L::strictest(&per_group);
    println!("FIT: {}", final_.code());
    match &final_ {
        L::FitPlan::AsIs => {
            println!("  action: rebuild target group(s) as-is, kill nothing");
        }
        L::FitPlan::Kill(k) => {
            println!("  action: delete from CURRENT slot, then rebuild target:");
            for n in k {
                println!("    - {n}");
            }
        }
        L::FitPlan::WipeAll => {
            println!("  action: delete ALL logical partitions, then rebuild target");
        }
        L::FitPlan::Impossible => {
            println!("  action: ERROR — not even a bare super fits, abort before any write");
            std::process::exit(3);
        }
    }
}

fn cmd_extract(args: &[String]) {
    let zip = flag(args, "--zip").unwrap_or_else(|| usage());
    let out = flag(args, "--out").unwrap_or_else(|| usage());
    let threads: usize = flag(args, "--threads")
        .map(|s| s.parse().unwrap_or_else(|_| usage()))
        .unwrap_or(8);
    let only: Option<Vec<String>> = flag(args, "--parts")
        .map(|s| s.split(',').map(|x| x.trim().to_string()).collect());
    std::fs::create_dir_all(&out).unwrap_or_else(|e| {
        eprintln!("mkdir {out}: {e}");
        std::process::exit(1);
    });

    let (layout, parts) = load(&zip);
    if let Err(r) = manifest::verdict_full(&parts) {
        eprintln!("refusing: {r}");
        std::process::exit(1);
    }
    let mut jobs: Vec<manifest::Part> = parts
        .into_iter()
        .filter(|p| only.as_ref().map(|o| o.contains(&p.name)).unwrap_or(true))
        .collect();
    if jobs.is_empty() {
        eprintln!("no partitions selected");
        std::process::exit(1);
    }
    // LPT: biggest expanded size first (tail bound by the largest).
    jobs.sort_by_key(|p| std::cmp::Reverse(p.out_size(layout.block_size)));

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(threads)
        .build()
        .unwrap_or_else(|e| {
            eprintln!("thread pool: {e}");
            std::process::exit(1);
        });
    println!(
        "extracting {} part(s) with {} threads (LPT order)",
        jobs.len(),
        threads
    );
    let t0 = Instant::now();
    let res: Vec<Result<extract::FetchStat, String>> = pool.install(|| {
        use rayon::prelude::*;
        jobs.par_iter()
            .map(|p| {
                let dst = format!("{out}/{}.img", p.name);
                extract::extract_partition(&zip, layout.data_start, layout.block_size, p, &dst)
            })
            .collect()
    });
    let mut ok_dl = 0u64;
    let mut ok_out = 0u64;
    let mut failed = 0;
    for (p, r) in jobs.iter().zip(res) {
        match r {
            Ok(s) => {
                ok_dl += s.downloaded_bytes;
                ok_out += s.output_bytes;
                // partition hash check against new_partition_info when present
                let hk = if p.new_hash.len() == 32 {
                    match extract::sha256_file(&format!("{out}/{}.img", p.name)) {
                        Ok(sum) => {
                            if sum[..] == p.new_hash[..] {
                                "sha256 OK"
                            } else {
                                "sha256 MISMATCH"
                            }
                        }
                        Err(e) => {
                            failed += 1;
                            eprintln!("  {:16} hash read failed: {e}", p.name);
                            continue;
                        }
                    }
                } else {
                    "no ref hash"
                };
                println!(
                    "  {:16} out={:<10} dl={:<10} {:>6} ms  {hk}",
                    s.name,
                    fmt_gb(s.output_bytes),
                    fmt_gb(s.downloaded_bytes),
                    s.millis
                );
            }
            Err(e) => {
                failed += 1;
                eprintln!("  {:16} FAILED: {e}", p.name);
            }
        }
    }
    println!(
        "done: {} ok, {} failed, out={} dl={} wall={} ms",
        jobs.len() - failed,
        failed,
        fmt_gb(ok_out),
        fmt_gb(ok_dl),
        t0.elapsed().as_millis()
    );
    if failed > 0 {
        std::process::exit(1);
    }
    let _ = ok_out;
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
    }
    match args[1].as_str() {
        "probe" => cmd_probe(&args[2..]),
        "plan" => cmd_plan(&args[2..]),
        "extract" => cmd_extract(&args[2..]),
        _ => usage(),
    }
}
