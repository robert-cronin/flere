//! Reproducible local PTY-to-frame latency and resource probe; no native agents.
mod support;
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    io,
    path::PathBuf,
    process::Command,
    time::{Duration, Instant},
};
use support::{Fixture, View};
fn distribution(mut values: Vec<f64>) -> Value {
    values.sort_by(f64::total_cmp);
    let percentile = |p: f64| values[((values.len() as f64 * p).ceil() as usize).saturating_sub(1)];
    json!({"samples":values.len(),"min_ms":values[0],"p50_ms":percentile(0.50),"p95_ms":percentile(0.95),"p99_ms":percentile(0.99),"max_ms":values[values.len()-1],"sorted_ms":values})
}
fn latency(view: &mut View, n: usize) -> io::Result<Value> {
    let mut values = Vec::new();
    for i in 0..n + 5 {
        let marker = format!("RH{i:06}");
        let start = Instant::now();
        view.key(marker.as_bytes())?;
        view.until(|s| s.capture(100).contains(&marker))?;
        let elapsed = start.elapsed().as_secs_f64() * 1000.;
        if i >= 5 {
            values.push(elapsed);
        }
        view.key(&[21])?;
        view.until(|s| !s.capture(100).contains(&marker))?;
    }
    Ok(distribution(values))
}
fn ps(pids: &[u32]) -> io::Result<BTreeMap<u32, (f64, u64)>> {
    let pids = pids
        .iter()
        .map(u32::to_string)
        .collect::<Vec<_>>()
        .join(",");
    let out = Command::new("ps")
        .args(["-p", &pids, "-o", "pid=,time=,rss="])
        .output()?;
    if !out.status.success() {
        return Err(io::Error::other("ps resource sample failed"));
    }
    let mut result = BTreeMap::new();
    for line in String::from_utf8_lossy(&out.stdout).lines() {
        let f = line.split_whitespace().collect::<Vec<_>>();
        if f.len() != 3 {
            return Err(io::Error::other("unexpected ps columns"));
        }
        let mut seconds = 0.;
        for piece in f[1].split(':') {
            seconds = seconds * 60. + piece.parse::<f64>().map_err(io::Error::other)?;
        }
        result.insert(
            f[0].parse().map_err(io::Error::other)?,
            (seconds, f[2].parse().map_err(io::Error::other)?),
        );
    }
    Ok(result)
}
fn resources(
    f: &Fixture,
    mut view: Option<&mut View>,
    name: &str,
    seconds: u64,
) -> io::Result<Value> {
    let flere = std::iter::once(f.server.id())
        .chain(view.as_ref().map(|v| v.child.id()))
        .collect::<Vec<_>>();
    let shells = f
        .snapshot()?
        .workspaces
        .iter()
        .flat_map(|w| w.tabs.iter().map(|t| t.pid))
        .collect::<Vec<_>>();
    let pids = flere
        .iter()
        .chain(shells.iter())
        .copied()
        .collect::<Vec<_>>();
    let first = ps(&pids)?;
    let start = Instant::now();
    let mut samples = Vec::new();
    let mut next = Duration::ZERO;
    while start.elapsed() < Duration::from_secs(seconds) {
        if let Some(v) = view.as_deref_mut() {
            v.read(10)?;
        } else {
            std::thread::sleep(Duration::from_millis(10));
        }
        if start.elapsed() >= next {
            let p = ps(&pids)?;
            samples.push(json!({"seconds":start.elapsed().as_secs_f64(),"flere_rss_mib":flere.iter().filter_map(|id|p.get(id)).map(|v|v.1).sum::<u64>() as f64/1024.,"shell_rss_mib":shells.iter().filter_map(|id|p.get(id)).map(|v|v.1).sum::<u64>() as f64/1024.}));
            next += Duration::from_secs(1);
        }
    }
    let last = ps(&pids)?;
    let elapsed = start.elapsed().as_secs_f64();
    let cpu = |ids: &[u32]| {
        ids.iter()
            .filter_map(|id| Some((last.get(id)?.0 - first.get(id)?.0).max(0.)))
            .sum::<f64>()
            / elapsed
            * 100.
    };
    Ok(
        json!({"scenario":name,"seconds":elapsed,"flere_processes":flere.len(),"shells":shells.len(),"flere_cpu_percent_one_core":cpu(&flere),"shell_cpu_percent_one_core":cpu(&shells),"samples":samples}),
    )
}
fn output(args: &[&str]) -> String {
    Command::new(args[0])
        .args(&args[1..])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = std::env::args_os().skip(1).collect::<Vec<_>>();
    if args.len() != 2 {
        return Err(
            "usage: latency_bench FLERE_EXE OUTPUT.json; run --release, use a home-cache output"
                .into(),
        );
    }
    let binary = std::fs::canonicalize(&args[0])?;
    let dest = PathBuf::from(&args[1]);
    let f = Fixture::new(&binary)?;
    f.workspace("Latency probe")?;
    let mut cases = Vec::new();
    for (cols, rows) in [(80, 24), (120, 32), (160, 42)] {
        eprintln!("Measuring {cols}x{rows}: direct PTY and completed Flere frames");
        let mut direct = View::new(&f, cols, rows, false)?;
        let baseline = latency(&mut direct, 100)?;
        drop(direct);
        let mut view = View::new(&f, cols, rows, true)?;
        let measured = latency(&mut view, 100)?;
        cases.push(json!({"cols":cols,"rows":rows,"direct_pty_echo":baseline,"flere_completed_frame":measured}));
    }
    let mut resource_cases = vec![resources(&f, None, "detached / 1 shell", 10)?];
    let mut view = View::new(&f, 120, 32, true)?;
    resource_cases.push(resources(&f, Some(&mut view), "attached / 1 shell", 10)?);
    let workspace = f.snapshot()?.active;
    for _ in 1..8 {
        f.request(&["tab", &workspace.to_string()])?;
    }
    view.pump(Duration::from_millis(500))?;
    resource_cases.push(resources(&f, Some(&mut view), "attached / 8 shells", 10)?);
    let data = json!({"schema":1,"version":output(&[binary.to_str().ok_or("non-UTF8 executable path")?,"--version"]),"source_commit":output(&["git","rev-parse","HEAD"]),"timestamp_utc":output(&["date","-u","+%Y-%m-%dT%H:%M:%SZ"]),"os":std::env::consts::OS,"arch":std::env::consts::ARCH,"os_version":output(&["sw_vers","-productVersion"]),"cpu":output(&["sysctl","-n","machdep.cpu.brand_string"]),"logical_cpus":output(&["sysctl","-n","hw.logicalcpu"]),"memory_bytes":output(&["sysctl","-n","hw.memsize"]),"rustc":output(&["rustc","--version"]),"method":{"marker_bytes":8,"warmup_samples":5,"samples_per_case":100,"observer":"Flere Terminal emulator; wait for visible marker and DEC 2026 frame end; polling wakes on readable PTY","memory":"ps RSS in KiB / 1024; sums may include shared pages; excludes compressed/nonresident memory","cpu":"delta process CPU time / wall time * 100; 100% is one CPU core","scope":"owned supervisor and attached UI; child shells separately; no models or user sessions"},"latency":cases,"resources":resource_cases});
    std::fs::write(&dest, serde_json::to_vec_pretty(&data)?)?;
    println!("{}", dest.display());
    Ok(())
}
