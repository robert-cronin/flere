//! In-memory snapshot stage probe; no supervisors, files, terminals or models.
use flere::{
    model::{Snapshot, TabView, WorkspaceView},
    terminal::Cell,
    workspace::CardMeta,
};
use serde_json::json;
use std::{hint::black_box, time::Instant};

fn number(args: &[String], index: usize, default: usize) -> usize {
    args.get(index)
        .map_or(default, |s| s.parse().expect("numeric argument"))
}
fn main() {
    let args: Vec<_> = std::env::args().skip(1).collect();
    let cards = number(&args, 0, 64);
    let notes = number(&args, 1, 24000);
    let iterations = number(&args, 2, 100);
    let rounds = number(&args, 3, 4);
    let escaped = args.get(4).is_some_and(|s| s == "escaped");
    assert!(
        args.len() <= 5
            && args.get(4).is_none_or(|s| s == "plain" || s == "escaped")
            && (1..=128).contains(&cards)
            && notes <= 65536
            && (10..=2000).contains(&iterations)
            && (1..=8).contains(&rounds)
            && cards * notes * iterations * rounds <= 16 * 1024 * 1024 * 1024,
        "usage: eval_snapshot [cards 1..128] [notes 0..65536] [iterations 10..2000] [rounds 1..8] [plain|escaped]; total note scan <=16 GiB"
    );
    let pattern = if escaped {
        "synthetic \"note\"\\path\n雪\t"
    } else {
        "n"
    };
    let mut note = pattern.repeat(notes / pattern.len() + 1);
    let mut end = notes;
    while !note.is_char_boundary(end) {
        end -= 1;
    }
    note.truncate(end);
    let conversation = flere::native::Conversation {
        harness: "codex".into(),
        uuid: "11111111-1234-5678-9012-123456789012".into(),
        cwd: "/fixture".into(),
    };
    let snapshot = Snapshot {
        epoch: "synthetic-epoch".into(),
        generation: 1,
        active: 1,
        tab: 1,
        workspaces: (1..=cards)
            .map(|id| WorkspaceView {
                id: id as u64,
                name: format!("fixture {id}"),
                cwd: "/fixture".into(),
                tabs: if id == 1 {
                    vec![TabView {
                        id: 1,
                        run: "a".repeat(32),
                        pid: 1,
                        alive: true,
                        title: "fixture".into(),
                        kind: "shell".into(),
                        path: String::new(),
                        working: false,
                    }]
                } else {
                    Vec::new()
                },
                meta: CardMeta {
                    notes: note.clone(),
                    conversations: vec![conversation.clone()],
                    last_conversation: Some(conversation.clone()),
                    ..Default::default()
                },
            })
            .collect(),
        cols: 80,
        rows: 24,
        x: 0,
        y: 0,
        cursor: true,
        bracketed_paste: false,
        app_cursor: false,
        notice: String::new(),
        cells: vec![Cell::default(); 80 * 24],
        split: None,
    };
    let projected: Vec<_> = snapshot
        .workspaces
        .iter()
        .map(|w| {
            let mut meta = w.meta.clone();
            meta.last_conversation = None;
            meta
        })
        .collect();
    let v4 = snapshot.encode();
    let v6 = snapshot.encode_links();
    for bytes in [&v4, &v6] {
        let decoded = Snapshot::decode(bytes).expect("valid synthetic frame");
        assert_eq!(decoded.workspaces.len(), cards);
        for (w, expected) in decoded.workspaces.iter().zip(&projected) {
            assert_eq!(
                serde_json::to_value(&w.meta).unwrap(),
                serde_json::to_value(expected).unwrap()
            );
        }
    }
    let mut samples = Vec::new();
    for round in 0..rounds {
        let mut stages = [
            "snapshot_clone",
            "metadata_clone",
            "metadata_json",
            "snapshot_v4",
            "snapshot_v6",
        ];
        if round % 2 == 1 {
            stages.reverse();
        }
        for stage in stages {
            let started = Instant::now();
            for _ in 0..iterations {
                match stage {
                    "snapshot_clone" => {
                        black_box(snapshot.clone());
                    }
                    "metadata_clone" => {
                        for w in &snapshot.workspaces {
                            black_box(w.meta.clone());
                        }
                    }
                    "metadata_json" => {
                        for meta in &projected {
                            black_box(serde_json::to_string(meta).unwrap());
                        }
                    }
                    "snapshot_v4" => {
                        black_box(snapshot.encode());
                    }
                    "snapshot_v6" => {
                        black_box(snapshot.encode_links());
                    }
                    _ => unreachable!(),
                }
            }
            samples.push(json!({"round":round,"stage":stage,
                "us_per_iteration":started.elapsed().as_secs_f64()*1e6/iterations as f64}));
        }
    }
    println!(
        "{}",
        json!({"schema":1,"build":serde_json::from_str::<serde_json::Value>(flere::build_info::json()).unwrap(),
        "cards":cards,"notes_bytes_per_card":note.len(),"escaped":escaped,"iterations":iterations,"rounds":rounds,
        "v4_bytes":v4.len(),"v6_bytes":v6.len(),"metadata_verified":true,"samples":samples,
        "method":"Synthetic in-memory stage wall times, batches with reversed order; black_box retains results. Metadata JSON uses the legacy projection prepared outside timing. Stages are separate workloads, not additive attribution. No allocation counts, supervisor, socket, UI, native model or physical-display latency."})
    );
}
