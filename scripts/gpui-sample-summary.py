#!/usr/bin/env python3
"""Summarize the main thread in a demangled macOS `sample` call tree.

Usage: python3 scripts/gpui-sample-summary.py SAMPLE.demangled.txt [...]
Counts are inclusive stack observations, not event counts or elapsed latency.
Categories overlap; recursive matches within one stack are counted only once.
Input must have Rust symbols demangled before classification.
"""

import json
import re
import sys
from pathlib import Path


def summarize(path):
    text = path.read_text()
    graph = text.split("Call graph:\n", 1)[1].split("Total number in stack", 1)[0]
    categories = {
        "keyboard_entry": lambda s: s in (
            "gpui_macos::window::handle_key_up", "gpui_macos::window::handle_key_down"),
        "window_draw": lambda s: s == "<gpui::window::Window>::draw",
        "chat_render": lambda s: s == "<monitter_gpui_desktop::chat::ChatPane as gpui::element::Render>::render",
        "subagent_record_count": lambda s: s == "<monitter_gpui_desktop::collaboration::SubagentPane>::record_count",
        "subagent_records": lambda s: s == "monitter_gpui_desktop::collaboration::records",
        "publish_native": lambda s: s == "<monitter_gpui_desktop::Desktop>::publish_native",
        "native_state_apply": lambda s: s.endswith("::NativeState>::apply"),
    }
    counts = dict.fromkeys(categories, 0)
    stack = []
    total = None
    for line in graph.splitlines():
        match = re.match(r"^([ +!|:]*)(\d+) (.+)$", line)
        if not match:
            continue
        prefix, count, symbol = match.groups()
        count = int(count)
        if symbol.startswith("Thread_"):
            if total is not None:
                break
            if "com.apple.main-thread" not in symbol:
                continue
            total = count
        if total is None:
            continue
        depth = len(prefix)
        while stack and stack[-1][0] >= depth:
            stack.pop()
        inherited = stack[-1][1] if stack else set()
        symbol = symbol.split("  (in ", 1)[0]
        matched = {key for key, predicate in categories.items() if predicate(symbol)}
        for key in matched - inherited:
            counts[key] += count
        stack.append((depth, inherited | matched))
    if total is None or not counts["window_draw"]:
        raise ValueError(f"{path}: no classified main-thread draw; check symbol demangling")
    return {
        "source": str(path),
        "main_thread_samples": total,
        "inclusive_samples": counts,
        "inclusive_percent": {key: round(value * 100 / total, 2) for key, value in counts.items()},
        "limitations": "Inclusive, overlapping sample counts; uncontrolled live workload; not key-to-paint timing.",
    }


if __name__ == "__main__":
    print(json.dumps([summarize(Path(path)) for path in sys.argv[1:]], indent=2))
