//! Local updater chrome. Only sanitized text is painted; package data never
//! supplies terminal controls. This stays independent of the remote frontend.
use super::{Flow, Phase, clean};
use std::io::{self, Write};

const TEXT: &str = "38;2;216;226;238";
const MUTED: &str = "38;2;122;140;175";
const CYAN: &str = "38;2;67;227;247";
const TITLE: &str = "1;38;2;67;227;247";
const GOLD: &str = "38;2;255;205;110";
const BORDER: &str = "38;2;35;54;68";
const PANEL: &str = "48;2;9;18;27";

// UTF-8 byte width is a conservative cell bound, including wide glyphs. Keep
// complete characters and never let dynamic text overwrite the panel border.
fn clip(text: &str, width: usize) -> String {
    let mut used = 0;
    clean(text)
        .chars()
        .take_while(|ch| {
            used += ch.len_utf8();
            used <= width
        })
        .collect()
}
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for ch in clean(text).chars() {
        if !lines.last().unwrap().is_empty()
            && lines.last().unwrap().len() + ch.len_utf8() > width.max(1)
        {
            lines.push(String::new());
        }
        lines.last_mut().unwrap().push(ch);
    }
    lines
}

pub(super) fn render(flow: &Flow, size: (u16, u16)) -> io::Result<Vec<u8>> {
    let (cols, rows) = (usize::from(size.0), usize::from(size.1));
    let mut frame = b"\x1b[?2026h\x1b[0m\x1b[?25l\x1b[48;2;6;14;22m\x1b[2J\x1b[H".to_vec();
    if cols < 12 || rows < 10 {
        if cols > 0 && rows > 0 {
            write!(
                frame,
                "\x1b[{CYAN}m{}",
                clip("Update Flere", cols.saturating_sub(1))
            )?;
            if rows > 1 {
                write!(
                    frame,
                    "\x1b[2;1H{}",
                    clip(&flow.status, cols.saturating_sub(1))
                )?;
            }
            if rows > 2 {
                write!(
                    frame,
                    "\x1b[3;1H{}",
                    clip("Enlarge window for controls", cols.saturating_sub(1))
                )?;
            }
        }
        frame.extend_from_slice(b"\x1b[0m\x1b[?2026l");
        return Ok(frame);
    }
    let width = cols.saturating_sub(2).min(84);
    let inner = width.saturating_sub(6);
    let mut lines: Vec<(String, &str)> = vec![("Update Flere".into(), TITLE)];
    let heading = match flow.phase {
        Phase::Form => "Custom package sources",
        Phase::Preparing => "1 / 3   Checking and verifying packages",
        Phase::Review => "Ready to update",
        Phase::Applying => "2 / 3   Installing update",
        Phase::Waiting => "3 / 3   Returning to Flere",
        Phase::Failed => "Update needs attention",
        Phase::Current => "Your installation",
        Phase::Complete => "Update complete",
    };
    lines.push((heading.into(), MUTED));
    lines.push((String::new(), TEXT));
    if let Some(plan) = &flow.plan {
        lines.push((
            format!(
                "Flere       {} → {}",
                plan.core.current.package_version, plan.core.candidate.build.package_version
            ),
            TEXT,
        ));
        lines.push((
            format!(
                "Companion   {}",
                plan.companion.package.manifest.build.package_version
            ),
            TEXT,
        ));
        lines.push((
            "Packages verified. Previous version retained.".into(),
            MUTED,
        ));
        if flow.phase == Phase::Review && plan.owners.manual() {
            lines.push(("Enter also adopts the manual remote core.".into(), GOLD));
        }
        if flow.phase == Phase::Review && plan.companion_owner.manual() {
            lines.push(("Enter also adopts the manual local companion.".into(), GOLD));
        }
        if flow.details {
            for text in [
                format!("Core target: {}", plan.core.candidate.build.target),
                format!("Core build: {}", plan.core.candidate.build.build_id),
                format!(
                    "Companion target: {}",
                    plan.companion.package.manifest.build.target
                ),
                format!(
                    "Companion build: {}",
                    plan.companion.package.manifest.build.build_id
                ),
            ] {
                lines.extend(wrap(&text, inner).into_iter().map(|s| (s, MUTED)));
            }
        }
    } else if flow.advanced {
        for (i, label) in ["Remote core", "Local companion"].into_iter().enumerate() {
            lines.push((
                format!(
                    "{} {label}: {}",
                    if flow.field == i { ">" } else { " " },
                    String::from_utf8_lossy(&flow.fields[i])
                ),
                if flow.field == i { CYAN } else { TEXT },
            ));
        }
        lines.push((
            "Package path / HTTPS manifest; blank = saved source".into(),
            MUTED,
        ));
        lines.push((
            "Use --rollback for retained compatible packages".into(),
            MUTED,
        ));
    }
    lines.push((String::new(), TEXT));
    if flow.phase != Phase::Review || flow.plan.is_none() {
        lines.extend(wrap(&flow.status, inner).into_iter().map(|s| {
            (
                s,
                if flow.phase == Phase::Failed {
                    GOLD
                } else {
                    TEXT
                },
            )
        }));
    }
    if matches!(flow.phase, Phase::Review | Phase::Applying | Phase::Waiting) {
        lines.push((
            "Your sessions stay open. Flere returns automatically.".into(),
            MUTED,
        ));
    }
    let hint = match flow.phase {
        Phase::Form => "Tab field · Enter prepare · Ctrl+U clear · Esc cancel",
        Phase::Review
            if flow
                .plan
                .as_ref()
                .is_some_and(|p| p.owners.manual() || p.companion_owner.manual()) =>
        {
            "Enter adopt and update · D details · Esc cancel"
        }
        Phase::Review => "Enter update now · D details · Esc cancel",
        Phase::Current | Phase::Failed => "R check again · A advanced sources · Enter / Esc close",
        Phase::Preparing if flow.advanced => "Preparing packages · Esc cancel",
        Phase::Preparing => "Esc cancel · A advanced sources",
        _ => "D details · Returning automatically when complete",
    };
    let mut hints = wrap(hint, inner);
    hints.truncate(rows.saturating_sub(8).max(1));
    let available = rows.saturating_sub(7 + hints.len()).max(1);
    if lines.len() > available {
        // Keep status/recovery guidance ahead of optional package details on a
        // short screen. Never disguise clipped error text as complete guidance.
        lines = vec![("Update Flere".into(), TITLE)];
        let status = wrap(&flow.status, inner);
        let room = available.saturating_sub(1);
        if status.len() > room {
            lines.extend(
                status
                    .into_iter()
                    .take(room.saturating_sub(1))
                    .map(|s| (s, GOLD)),
            );
            if room > 0 {
                lines.push((
                    "Enlarge this window to read the complete update guidance.".into(),
                    GOLD,
                ));
            }
        } else {
            lines.extend(status.into_iter().map(|s| (s, TEXT)));
        }
    }
    lines.push((String::new(), TEXT));
    lines.extend(hints.into_iter().map(|s| (s, CYAN)));
    let height = (lines.len() + 4).min(rows.saturating_sub(2));
    let (x, y) = ((cols - width) / 2, (rows - height) / 2);
    for row in 0..height {
        write!(
            frame,
            "\x1b[{};{}H\x1b[0;{BORDER};{PANEL}m",
            y + row + 1,
            x + 1
        )?;
        if row == 0 || row == height - 1 {
            write!(
                frame,
                "{}{}{}",
                if row == 0 { '┌' } else { '└' },
                "─".repeat(width - 2),
                if row == 0 { '┐' } else { '┘' }
            )?;
        } else {
            write!(frame, "│{}│", " ".repeat(width - 2))?;
        }
    }
    for (row, (text, color)) in lines.iter().take(height.saturating_sub(4)).enumerate() {
        write!(
            frame,
            "\x1b[{};{}H\x1b[0;{color};{PANEL}m{}",
            y + row + 3,
            x + 3,
            clip(text, inner)
        )?;
    }
    frame.extend_from_slice(b"\x1b[0m\x1b[?2026l");
    Ok(frame)
}
