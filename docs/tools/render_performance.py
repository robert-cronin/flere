#!/usr/bin/env python3
"""Generate the performance chart and reference tables from recorded measurements."""

import argparse, json, pathlib, statistics
import matplotlib

matplotlib.use("Agg")
import matplotlib.pyplot as plt
from matplotlib.ticker import MultipleLocator


def main():
    p = argparse.ArgumentParser(description=__doc__)
    p.add_argument("data", type=pathlib.Path)
    p.add_argument("assets", type=pathlib.Path)
    a = p.parse_args()
    data = json.loads(a.data.read_text())
    lat = data["latency"]
    prefix = "flere" if "flere_completed_frame" in lat[0] else "railhand"
    brand = "Flere" if prefix == "flere" else "Railhand"
    resources = data["resources"]
    colors = {
        "bg": "#09101d",
        "fg": "#dce5fa",
        "muted": "#8b9cb8",
        "grid": "#25334b",
        "cyan": "#43e3f7",
        "shell": "#627499",
        "pink": "#d797ff",
    }
    plt.rcParams.update(
        {
            "font.family": "DejaVu Sans",
            "font.size": 11,
            "text.color": colors["fg"],
            "axes.labelcolor": colors["muted"],
            "xtick.color": colors["muted"],
            "ytick.color": colors["fg"],
            "svg.fonttype": "none",
            "svg.hashsalt": f"{prefix}-performance-v1",
        }
    )
    fig = plt.figure(figsize=(13, 7.4), facecolor=colors["bg"])
    fig.text(0.055, 0.91, f"{brand} performance snapshot", fontsize=27, weight="bold")
    fig.text(
        0.055,
        0.855,
        f"{data['cpu']}  /  macOS {data['os_version']}  /  {int(data['memory_bytes'])/1024**3:g} GiB RAM  /  {data['version'].split(' (')[0]} release",
        color=colors["muted"],
        fontsize=11,
    )
    left = fig.add_axes([0.12, 0.34, 0.33, 0.34])
    right = fig.add_axes([0.66, 0.34, 0.29, 0.34])
    for ax in [left, right]:
        ax.set_facecolor(colors["bg"])
        ax.set_axisbelow(True)
        ax.grid(axis="x", color=colors["grid"], linewidth=0.7)
        for s in ax.spines.values():
            s.set_visible(False)
        ax.tick_params(axis="both", length=0, pad=9)
    y = list(range(len(lat)))
    median = [c[f"{prefix}_completed_frame"]["p50_ms"] for c in lat]
    p95 = [c[f"{prefix}_completed_frame"]["p95_ms"] for c in lat]
    left.barh(y, median, color=colors["cyan"], height=0.35)
    left.scatter(p95, y, color=colors["fg"], marker="|", s=250, linewidth=2, zorder=3)
    for i, (m, p) in enumerate(zip(median, p95)):
        left.text(p + 1, i, f"{m:.1f} / {p:.1f}", va="center", fontsize=10)
    left.set(
        yticks=y,
        yticklabels=[f"{c['cols']} × {c['rows']}" for c in lat],
        xlim=(0, 32),
        ylim=(2.65, -0.65),
        xlabel="Milliseconds · lower is better",
    )
    left.xaxis.set_major_locator(MultipleLocator(10))
    fig.text(
        0.055,
        0.745,
        "INPUT → COMPLETED FRAME",
        weight="bold",
        fontsize=12,
        color=colors["cyan"],
    )
    fig.text(
        0.12,
        0.245,
        "Cyan bar: median   |   White tick: p95",
        fontsize=10,
        color=colors["muted"],
    )
    baseline = [c["direct_pty_echo"]["p50_ms"] for c in lat]
    fig.text(
        0.12,
        0.205,
        f"Direct PTY median: {min(baseline):.2f}–{max(baseline):.2f} ms",
        fontsize=10,
        color=colors["muted"],
    )
    rh = [
        statistics.mean(s[f"{prefix}_rss_mib"] for s in c["samples"]) for c in resources
    ]
    shell = [
        statistics.mean(s["shell_rss_mib"] for s in c["samples"]) for c in resources
    ]
    right.barh(y, rh, color=colors["cyan"], height=0.35, label=brand)
    right.barh(y, shell, left=rh, color=colors["shell"], height=0.35, label="Shells")
    for i, (r, s) in enumerate(zip(rh, shell)):
        right.text(r + s + 0.6, i, f"{r+s:.1f}", va="center", fontsize=10)
    right.set(
        yticks=y,
        yticklabels=["Detached\n1 shell", "Attached\n1 shell", "Attached\n8 shells"],
        xlim=(0, 32),
        ylim=(2.65, -0.65),
        xlabel="MiB · mean resident memory",
    )
    right.xaxis.set_major_locator(MultipleLocator(10))
    fig.text(
        0.57,
        0.745,
        "IDLE RESIDENT MEMORY",
        weight="bold",
        fontsize=12,
        color=colors["cyan"],
    )
    right.legend(
        loc="lower left",
        bbox_to_anchor=(-0.01, -0.39),
        ncol=2,
        frameon=False,
        labelcolor=colors["muted"],
        fontsize=10,
    )
    cpu = resources[1][f"{prefix}_cpu_percent_one_core"]
    fig.text(
        0.66,
        0.205,
        f"Attached idle CPU: {cpu:.2f}% of one core",
        fontsize=10,
        color=colors["muted"],
    )
    fig.text(
        0.055,
        0.105,
        "100 measured inputs per viewport · 5 warmups · 10 seconds per resource scenario",
        fontsize=10,
        color=colors["fg"],
    )
    fig.text(
        0.055,
        0.06,
        "One local run. Excludes physical display latency, model inference, network time, and external agent tools.",
        fontsize=9,
        color=colors["muted"],
    )
    a.assets.mkdir(parents=True, exist_ok=True)
    for suffix in ["svg", "png"]:
        fig.savefig(
            a.assets / f"performance.{suffix}",
            dpi=150,
            facecolor=fig.get_facecolor(),
            metadata=(
                {"Creator": f"{brand} documentation tools"} if suffix == "svg" else None
            ),
        )
    svg = a.assets / "performance.svg"
    svg.write_text(
        "\n".join(line.rstrip() for line in svg.read_text().splitlines()) + "\n"
    )
    print(svg)


if __name__ == "__main__":
    main()
