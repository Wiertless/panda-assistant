"""A short description of this computer for the system prompt.

Kept to a few lines: local models run with small contexts. Live values the
user may ask about later (battery, free disk) stay behind tools.
"""

from __future__ import annotations

import datetime
import os
import platform
from pathlib import Path


def _read(path: str) -> str:
    try:
        return Path(path).read_text(errors="replace").strip()
    except OSError:
        return ""


def _meminfo() -> dict[str, int]:
    out = {}
    for line in _read("/proc/meminfo").splitlines():
        key, _, rest = line.partition(":")
        try:
            out[key] = int(rest.split()[0])  # KiB
        except (IndexError, ValueError):
            pass
    return out


def _cpu() -> str:
    for line in _read("/proc/cpuinfo").splitlines():
        if line.startswith("model name"):
            return line.split(":", 1)[1].strip()
    return platform.processor() or platform.machine()


def _os() -> str:
    for line in _read("/etc/os-release").splitlines():
        if line.startswith("PRETTY_NAME="):
            return line.split("=", 1)[1].strip('"')
    return platform.system()


def system_context() -> str:
    lines = []
    vendor, product = _read("/sys/class/dmi/id/sys_vendor"), _read("/sys/class/dmi/id/product_name")
    if product:
        lines.append(f"Computer: {vendor} {product}".strip())
    cores = os.cpu_count()
    lines.append(f"CPU: {_cpu()}" + (f" ({cores} threads)" if cores else ""))
    mem = _meminfo()
    if "MemTotal" in mem:
        total = mem["MemTotal"] / 2**20
        avail = mem.get("MemAvailable", 0) / 2**20
        lines.append(f"Memory: {total:.0f} GiB total, {avail:.1f} GiB free right now")
    if Path("/dev/accel/accel0").exists():
        lines.append("NPU: AMD XDNA (you are running on it through FastFlowLM)")
    lines.append(f"OS: {_os()}, kernel {platform.release()}, desktop {os.environ.get('XDG_CURRENT_DESKTOP', '?')}")
    bats = sorted(Path("/sys/class/power_supply").glob("BAT*"))
    if bats:
        lines.append(f"Battery: {_read(str(bats[0] / 'capacity'))}% ({_read(str(bats[0] / 'status')).lower()})")
    lines.append("Today: " + datetime.date.today().strftime("%A %Y-%m-%d"))
    return "About this computer:\n" + "\n".join(f"- {line}" for line in lines)
