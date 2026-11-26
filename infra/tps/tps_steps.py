"""`rate:seconds,...` step lists for tps-loadgen run."""


def parse(steps):
    out = []
    for part in steps.split(","):
        rate, secs = part.split(":")
        out.append((float(rate), float(secs)))
    return out


def total_seconds(steps):
    return sum(s for _, s in parse(steps))


def ramp(rates, seconds):
    """A ramp of equal-length steps."""
    return ",".join(f"{r}:{seconds}" for r in rates)
